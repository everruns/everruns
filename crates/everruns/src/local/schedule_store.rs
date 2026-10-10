// SQLite-backed SessionScheduleStore, org + session scoped.
//
// Metadata round-trip decision (EVE-594 acceptance criterion):
// The shared core primitive `everruns_core::session_schedule::SessionSchedule`
// intentionally has NO open metadata bag, and we do not add one — touching the
// shared data model is out of scope. Instead, the local store carries a JSON
// `metadata` column in its OWN schema. An embedder that needs to preserve extra
// fields (name/color/kind/command/model/isolated, etc.) calls the additive
// `create_schedule_with_metadata` / `get_metadata` methods on this concrete
// type. The trait-level `SessionScheduleStore` surface is unchanged, so the
// runtime act path sees the standard schedule store while embedders keep their
// extensible bag locally.

use crate::sqlite as rusqlite;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::runtime::events::EventData;
use everruns_contracts::typed_id::{PrincipalId, ScheduleId, SessionId};
use everruns_core::host::EventLog;
use everruns_core::session_schedule::{
    DEFAULT_MAX_SCHEDULES_PER_ORG, DEFAULT_MIN_INTERVAL_SECONDS, MAX_ACTIVE_SCHEDULES_PER_SESSION,
    ScheduleLimitError, SessionSchedule, validate_cron_min_interval_with,
};
use everruns_core::session_services::SessionScheduleStore;
use rusqlite::{OptionalExtension, TransactionBehavior};
use serde_json::Value;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

fn schedule_limits() -> (i64, i64) {
    let min_interval = std::env::var("SESSION_SCHEDULE_MIN_INTERVAL_SECONDS")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_MIN_INTERVAL_SECONDS);
    let max_per_org = std::env::var("RESOURCE_LIMIT_MAX_SESSION_SCHEDULES_PER_ORG")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_MAX_SCHEDULES_PER_ORG);
    (min_interval, max_per_org)
}

use super::db::SqliteDb;
use super::error::LocalError;
use super::schedule_journal::{self as journal, JournalTimer};

/// SQLite-backed schedule store for local embedded hosts.
#[derive(Clone)]
pub struct LocalScheduleStore {
    db: SqliteDb,
    /// Internal org id this store instance is scoped to.
    org_id: i64,
    /// Principal stamped on created schedules.
    owner_principal_id: PrincipalId,
    /// Session log the schedules are recorded in. `None` keeps the index only
    /// (a store built without a runtime, e.g. in a unit test).
    journal: Option<Arc<dyn EventLog>>,
}

/// What [`LocalScheduleStore::reconcile_journal`] changed.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ScheduleReconcileReport {
    /// Rows rebuilt from timers the log has and the index lacked.
    pub restored: usize,
    /// Rows moved forward to occurrences or cancellations the log records.
    pub advanced: usize,
    /// Enabled rows written into a log that had no timer for them.
    pub journaled: usize,
}

#[derive(Debug)]
pub(crate) struct ClaimedSchedule {
    pub schedule: SessionSchedule,
    pub claim_id: String,
}

impl LocalScheduleStore {
    /// Open (and migrate) a schedule store over `db`, scoped to `org_id`.
    pub fn new(db: SqliteDb, org_id: i64, owner_principal_id: PrincipalId) -> Result<Self> {
        Self::ensure_schema(&db)?;
        Ok(Self::scoped(db, org_id, owner_principal_id))
    }

    /// Create the `local_schedules` schema if it does not yet exist. Idempotent.
    /// The schema is org-agnostic (org scoping is a column), so this only needs
    /// to run once per database file.
    fn ensure_schema(db: &SqliteDb) -> Result<()> {
        db.with_conn(|conn| {
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS local_schedules (
                    id          TEXT PRIMARY KEY,
                    org_id      INTEGER NOT NULL,
                    session_id  TEXT NOT NULL,
                    enabled     INTEGER NOT NULL,
                    snapshot    TEXT NOT NULL,
                    metadata    TEXT NOT NULL DEFAULT '{}',
                    next_trigger_at_ms INTEGER,
                    claimed_by  TEXT,
                    claimed_at_ms INTEGER,
                    last_delivery_error TEXT,
                    runner_migrated INTEGER NOT NULL DEFAULT 1
                 );
                 CREATE INDEX IF NOT EXISTS idx_local_schedules_session
                    ON local_schedules(org_id, session_id);",
            )
        })
        .map_err(AgentLoopError::from)?;
        Self::migrate_runner_columns(db)?;
        Ok(())
    }

    fn migrate_runner_columns(db: &SqliteDb) -> Result<()> {
        db.with_conn_mut(|conn| {
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let columns = {
                let mut stmt = tx.prepare("PRAGMA table_info(local_schedules)")?;
                stmt.query_map([], |row| row.get::<_, String>(1))?
                    .collect::<rusqlite::Result<Vec<_>>>()?
            };
            for (name, sql_type) in [
                ("next_trigger_at_ms", "INTEGER"),
                ("claimed_by", "TEXT"),
                ("claimed_at_ms", "INTEGER"),
                ("last_delivery_error", "TEXT"),
                ("runner_migrated", "INTEGER NOT NULL DEFAULT 0"),
            ] {
                if !columns.iter().any(|column| column == name) {
                    tx.execute(
                        &format!("ALTER TABLE local_schedules ADD COLUMN {name} {sql_type}"),
                        [],
                    )?;
                }
            }
            tx.execute(
                "CREATE INDEX IF NOT EXISTS idx_local_schedules_due
                 ON local_schedules(enabled, next_trigger_at_ms, claimed_at_ms)",
                [],
            )?;
            tx.commit()
        })
        .map_err(AgentLoopError::from)?;

        let rows: Vec<(String, String)> = db
            .with_conn(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, snapshot FROM local_schedules WHERE runner_migrated = 0",
                )?;
                stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                    .collect()
            })
            .map_err(AgentLoopError::from)?;
        for (id, json) in rows {
            let mut schedule: SessionSchedule = serde_json::from_str(&json)
                .map_err(|e| AgentLoopError::from(LocalError::from(e)))?;
            if schedule.enabled
                && schedule.next_trigger_at.is_none()
                && schedule.cron_expression.is_some()
            {
                match next_cron_trigger(&schedule, Utc::now()) {
                    Ok(next) => {
                        schedule.next_trigger_at = Some(next);
                        schedule.updated_at = Utc::now();
                    }
                    Err(error) => {
                        tracing::warn!(schedule_id = %schedule.id, error = %error, "Local recurring schedule could not be migrated for execution");
                    }
                }
            }
            let snapshot = serde_json::to_string(&schedule)
                .map_err(|e| AgentLoopError::from(LocalError::from(e)))?;
            let next_ms = schedule.next_trigger_at.map(|time| time.timestamp_millis());
            db.with_conn(|conn| {
                conn.execute(
                    "UPDATE local_schedules
                     SET snapshot = ?2, next_trigger_at_ms = ?3, runner_migrated = 1
                     WHERE id = ?1",
                    rusqlite::params![id, snapshot, next_ms],
                )
            })
            .map_err(AgentLoopError::from)?;
        }
        Ok(())
    }

    /// Construct a store scoped to `org_id` without touching the database.
    /// Callers must have already ensured the schema exists (via [`Self::new`]);
    /// this keeps the per-(org) factory on the act path cheap and infallible.
    pub(crate) fn scoped(db: SqliteDb, org_id: i64, owner_principal_id: PrincipalId) -> Self {
        Self {
            db,
            org_id,
            owner_principal_id,
            journal: None,
        }
    }

    /// Record schedules in `log` as `timer.*` entries of their session, so the
    /// log is enough to rebuild them (see
    /// [`reconcile_journal`](Self::reconcile_journal)).
    pub fn with_journal(mut self, log: Arc<dyn EventLog>) -> Self {
        self.journal = Some(log);
        self
    }

    async fn record(&self, session_id: SessionId, data: impl Into<EventData>) -> Result<()> {
        match &self.journal {
            Some(log) => journal::append(log.as_ref(), session_id, data).await,
            None => Ok(()),
        }
    }

    fn build_schedule(
        &self,
        session_id: SessionId,
        description: String,
        cron_expression: Option<String>,
        scheduled_at: Option<DateTime<Utc>>,
        timezone: String,
    ) -> Result<SessionSchedule> {
        let now = Utc::now();
        let mut schedule = SessionSchedule {
            id: ScheduleId::new(),
            session_id,
            owner_principal_id: self.owner_principal_id,
            resolved_owner_user_id: None,
            owner: None,
            effective_owner: None,
            description,
            cron_expression: cron_expression.clone(),
            scheduled_at,
            timezone,
            enabled: true,
            schedule_type: SessionSchedule::derive_type(&cron_expression),
            next_trigger_at: scheduled_at,
            last_triggered_at: None,
            trigger_count: 0,
            created_at: now,
            updated_at: now,
        };
        if schedule.cron_expression.is_some() {
            schedule.next_trigger_at = Some(next_cron_trigger(&schedule, now)?);
        }
        Ok(schedule)
    }

    fn insert(&self, schedule: &SessionSchedule, metadata: &Value) -> Result<()> {
        let snapshot = serde_json::to_string(schedule)
            .map_err(|e| AgentLoopError::from(LocalError::from(e)))?;
        let metadata_json = serde_json::to_string(metadata)
            .map_err(|e| AgentLoopError::from(LocalError::from(e)))?;
        let id = schedule.id.to_string();
        let session = schedule.session_id.to_string();
        let enabled = schedule.enabled as i64;
        let org_id = self.org_id;
        let next_trigger_at_ms = schedule.next_trigger_at.map(|time| time.timestamp_millis());
        self.db
            .with_conn(|conn| {
                conn.execute(
                    "INSERT INTO local_schedules (id, org_id, session_id, enabled, snapshot, metadata, next_trigger_at_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                     ON CONFLICT(id) DO UPDATE SET
                        enabled = excluded.enabled,
                        snapshot = excluded.snapshot,
                        metadata = excluded.metadata,
                        next_trigger_at_ms = excluded.next_trigger_at_ms,
                        claimed_by = CASE WHEN excluded.enabled = 0 THEN NULL ELSE claimed_by END,
                        claimed_at_ms = CASE WHEN excluded.enabled = 0 THEN NULL ELSE claimed_at_ms END",
                    rusqlite::params![id, org_id, session, enabled, snapshot, metadata_json, next_trigger_at_ms],
                )
            })
            .map_err(AgentLoopError::from)?;
        Ok(())
    }

    fn load(&self, schedule_id: ScheduleId) -> Result<Option<SessionSchedule>> {
        let id = schedule_id.to_string();
        let org_id = self.org_id;
        let snapshot: Option<String> = self
            .db
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT snapshot FROM local_schedules WHERE id = ?1 AND org_id = ?2",
                    rusqlite::params![id, org_id],
                    |row| row.get(0),
                )
                .optional()
            })
            .map_err(AgentLoopError::from)?;
        match snapshot {
            Some(json) => Ok(Some(
                serde_json::from_str(&json)
                    .map_err(|e| AgentLoopError::from(LocalError::from(e)))?,
            )),
            None => Ok(None),
        }
    }

    /// Create a schedule and persist caller-supplied extra fields in the local
    /// `metadata` column. This is the additive seam that satisfies the
    /// "extensible metadata bag" criterion without changing the core primitive.
    pub async fn create_schedule_with_metadata(
        &self,
        session_id: SessionId,
        description: String,
        cron_expression: Option<String>,
        scheduled_at: Option<DateTime<Utc>>,
        timezone: String,
        metadata: Value,
    ) -> Result<SessionSchedule> {
        let schedule = self.build_schedule(
            session_id,
            description,
            cron_expression,
            scheduled_at,
            timezone,
        )?;
        self.record(session_id, journal::timer_set(&schedule, &metadata))
            .await?;
        self.insert(&schedule, &metadata)?;
        Ok(schedule)
    }

    /// Read back the metadata bag previously stored for a schedule. Returns
    /// `None` when the schedule does not exist in this org scope.
    pub async fn get_metadata(&self, schedule_id: ScheduleId) -> Result<Option<Value>> {
        let id = schedule_id.to_string();
        let org_id = self.org_id;
        let metadata: Option<String> = self
            .db
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT metadata FROM local_schedules WHERE id = ?1 AND org_id = ?2",
                    rusqlite::params![id, org_id],
                    |row| row.get(0),
                )
                .optional()
            })
            .map_err(AgentLoopError::from)?;
        match metadata {
            Some(json) => Ok(Some(
                serde_json::from_str(&json)
                    .map_err(|e| AgentLoopError::from(LocalError::from(e)))?,
            )),
            None => Ok(None),
        }
    }

    /// Last scheduled-delivery error, retained until a later delivery succeeds.
    pub async fn last_delivery_error(&self, schedule_id: ScheduleId) -> Result<Option<String>> {
        let id = schedule_id.to_string();
        let org_id = self.org_id;
        self.db
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT last_delivery_error FROM local_schedules WHERE id = ?1 AND org_id = ?2",
                    rusqlite::params![id, org_id],
                    |row| row.get(0),
                )
                .optional()
                .map(|value| value.flatten())
            })
            .map_err(AgentLoopError::from)
    }

    pub(crate) fn claim_due(
        &self,
        runner_id: &str,
        now: DateTime<Utc>,
        claim_timeout: Duration,
        limit: usize,
        routable_session_ids: Option<&[SessionId]>,
    ) -> Result<Vec<ClaimedSchedule>> {
        if routable_session_ids.is_some_and(<[SessionId]>::is_empty) {
            return Ok(Vec::new());
        }
        let now_ms = now.timestamp_millis();
        let timeout_ms = i64::try_from(claim_timeout.as_millis()).unwrap_or(i64::MAX);
        let stale_before_ms = now_ms.saturating_sub(timeout_ms);
        let org_id = self.org_id;
        let runner_id = runner_id.to_string();
        let routable_session_ids_json = routable_session_ids
            .map(|ids| ids.iter().map(ToString::to_string).collect::<Vec<_>>())
            .map(|ids| serde_json::to_string(&ids))
            .transpose()
            .map_err(|error| AgentLoopError::from(LocalError::from(error)))?;
        self.db
            .with_conn_mut(|conn| {
                let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let candidates = {
                    let mut stmt = tx.prepare(
                        "SELECT id, session_id, snapshot FROM local_schedules
                         WHERE org_id = ? AND enabled = 1
                           AND next_trigger_at_ms IS NOT NULL AND next_trigger_at_ms <= ?
                           AND (claimed_at_ms IS NULL OR claimed_at_ms <= ?)
                           AND (?4 IS NULL OR session_id IN (SELECT value FROM json_each(?4)))
                         ORDER BY next_trigger_at_ms ASC LIMIT ?5",
                    )?;
                    stmt.query_map(
                        rusqlite::params![
                            org_id,
                            now_ms,
                            stale_before_ms,
                            routable_session_ids_json,
                            limit as i64
                        ],
                        |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, String>(2)?,
                            ))
                        },
                    )?
                    .collect::<rusqlite::Result<Vec<_>>>()?
                };
                let mut claimed = Vec::with_capacity(candidates.len());
                for (id, session_id, snapshot) in candidates {
                    let changed = tx.execute(
                        "UPDATE local_schedules SET claimed_by = ?2, claimed_at_ms = ?3
                         WHERE id = ?1 AND org_id = ?4 AND enabled = 1
                           AND session_id = ?6
                           AND (claimed_at_ms IS NULL OR claimed_at_ms <= ?5)",
                        rusqlite::params![
                            id,
                            runner_id,
                            now_ms,
                            org_id,
                            stale_before_ms,
                            session_id
                        ],
                    )?;
                    if changed == 1 {
                        claimed.push((snapshot, id));
                    }
                }
                tx.commit()?;
                claimed
                    .into_iter()
                    .map(|(snapshot, id)| {
                        serde_json::from_str(&snapshot)
                            .map(|schedule| ClaimedSchedule {
                                schedule,
                                claim_id: id,
                            })
                            .map_err(|error| {
                                rusqlite::Error::FromSqlConversionFailure(
                                    0,
                                    rusqlite::types::Type::Text,
                                    Box::new(error),
                                )
                            })
                    })
                    .collect()
            })
            .map_err(AgentLoopError::from)
    }

    pub(crate) async fn complete_delivery(
        &self,
        claim: &ClaimedSchedule,
        runner_id: &str,
        delivered_at: DateTime<Utc>,
    ) -> Result<()> {
        let mut schedule = claim.schedule.clone();
        schedule.last_triggered_at = Some(delivered_at);
        schedule.trigger_count = schedule.trigger_count.saturating_add(1);
        schedule.updated_at = delivered_at;
        if schedule.cron_expression.is_some() {
            schedule.next_trigger_at = Some(next_cron_trigger(&schedule, delivered_at)?);
        } else {
            schedule.enabled = false;
            schedule.next_trigger_at = None;
        }
        self.record(
            schedule.session_id,
            journal::timer_fired(&schedule, delivered_at),
        )
        .await?;
        let snapshot = serde_json::to_string(&schedule)
            .map_err(|e| AgentLoopError::from(LocalError::from(e)))?;
        let next_ms = schedule.next_trigger_at.map(|time| time.timestamp_millis());
        let changed = self
            .db
            .with_conn(|conn| {
                conn.execute(
                    "UPDATE local_schedules
                     SET enabled = ?3, snapshot = ?4, next_trigger_at_ms = ?5,
                         claimed_by = NULL, claimed_at_ms = NULL, last_delivery_error = NULL
                     WHERE id = ?1 AND org_id = ?2 AND claimed_by = ?6",
                    rusqlite::params![
                        claim.claim_id,
                        self.org_id,
                        schedule.enabled as i64,
                        snapshot,
                        next_ms,
                        runner_id,
                    ],
                )
            })
            .map_err(AgentLoopError::from)?;
        if changed != 1 {
            return Err(AgentLoopError::store(format!(
                "local schedule claim {} is no longer owned by runner",
                claim.claim_id
            )));
        }
        Ok(())
    }

    pub(crate) fn renew_claim(
        &self,
        claim: &ClaimedSchedule,
        runner_id: &str,
        now: DateTime<Utc>,
    ) -> Result<bool> {
        let changed = self
            .db
            .with_conn(|conn| {
                conn.execute(
                    "UPDATE local_schedules SET claimed_at_ms = ?4
                     WHERE id = ?1 AND org_id = ?2 AND claimed_by = ?3",
                    rusqlite::params![
                        claim.claim_id,
                        self.org_id,
                        runner_id,
                        now.timestamp_millis(),
                    ],
                )
            })
            .map_err(AgentLoopError::from)?;
        Ok(changed == 1)
    }

    pub(crate) fn fail_delivery(
        &self,
        claim: &ClaimedSchedule,
        runner_id: &str,
        failed_at: DateTime<Utc>,
        error: &str,
    ) -> Result<()> {
        let changed = self
            .db
            .with_conn(|conn| {
                conn.execute(
                    "UPDATE local_schedules
                     SET claimed_by = NULL, claimed_at_ms = ?4, last_delivery_error = ?5
                     WHERE id = ?1 AND org_id = ?2 AND claimed_by = ?3",
                    rusqlite::params![
                        claim.claim_id,
                        self.org_id,
                        runner_id,
                        failed_at.timestamp_millis(),
                        error
                    ],
                )
            })
            .map_err(AgentLoopError::from)?;
        if changed != 1 {
            return Err(AgentLoopError::store(format!(
                "local schedule claim {} is no longer owned by runner",
                claim.claim_id
            )));
        }
        Ok(())
    }
}

impl LocalScheduleStore {
    fn delete_row(&self, schedule_id: ScheduleId) -> Result<()> {
        let id = schedule_id.to_string();
        let org_id = self.org_id;
        self.db
            .with_conn(|conn| {
                conn.execute(
                    "DELETE FROM local_schedules WHERE id = ?1 AND org_id = ?2",
                    rusqlite::params![id, org_id],
                )
            })
            .map_err(AgentLoopError::from)?;
        Ok(())
    }

    /// Sessions whose log or index may hold schedules: every session in the
    /// local catalog plus every session the index names.
    fn reconcile_sessions(&self) -> Result<Vec<SessionId>> {
        let org_id = self.org_id;
        let ids: Vec<String> = self
            .db
            .with_conn(|conn| {
                let catalog: bool = conn.query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master
                     WHERE type = 'table' AND name = 'framework_sessions')",
                    [],
                    |row| row.get(0),
                )?;
                let sql = if catalog {
                    "SELECT session_id FROM local_schedules WHERE org_id = ?1
                     UNION SELECT session_id FROM framework_sessions"
                } else {
                    "SELECT DISTINCT session_id FROM local_schedules WHERE org_id = ?1"
                };
                let mut stmt = conn.prepare(sql)?;
                stmt.query_map(rusqlite::params![org_id], |row| row.get(0))?
                    .collect::<rusqlite::Result<Vec<String>>>()
            })
            .map_err(AgentLoopError::from)?;
        Ok(ids
            .into_iter()
            .filter_map(|id| SessionId::from_str(&id).ok())
            .collect())
    }

    /// Make the index agree with the session logs; the log wins where it has
    /// the schedule. The runner calls this before it first polls. A store
    /// without a journal has nothing to reconcile.
    pub async fn reconcile_journal(&self) -> Result<ScheduleReconcileReport> {
        let mut report = ScheduleReconcileReport::default();
        let Some(log) = self.journal.clone() else {
            return Ok(report);
        };
        for session_id in self.reconcile_sessions()? {
            let timers = match journal::read_timers(log.as_ref(), session_id).await {
                Ok(timers) => timers,
                Err(error) => {
                    tracing::warn!(%session_id, %error, "Skipping schedule reconcile for a session whose log could not be read");
                    continue;
                }
            };
            let mut journaled = std::collections::HashSet::new();
            for timer in &timers {
                journaled.insert(timer.set.timer_id.clone());
                self.reconcile_timer(session_id, timer, &mut report)?;
            }
            for schedule in self.list_schedules(session_id).await? {
                if schedule.enabled && !journaled.contains(&schedule.id.to_string()) {
                    let metadata = self
                        .get_metadata(schedule.id)
                        .await?
                        .unwrap_or_else(|| Value::Object(Default::default()));
                    self.record(session_id, journal::timer_set(&schedule, &metadata))
                        .await?;
                    report.journaled += 1;
                }
            }
        }
        if report != ScheduleReconcileReport::default() {
            tracing::info!(?report, "Local schedules reconciled with session logs");
        }
        Ok(report)
    }

    fn reconcile_timer(
        &self,
        session_id: SessionId,
        timer: &JournalTimer,
        report: &mut ScheduleReconcileReport,
    ) -> Result<()> {
        let rebuilt = timer.schedule(session_id, self.owner_principal_id)?;
        let Some(mut row) = self.load(rebuilt.id)? else {
            self.insert(&rebuilt, &timer.metadata())?;
            report.restored += 1;
            return Ok(());
        };
        let newer_fires = timer.fired_since(row.last_triggered_at);
        let cancel = timer.cancelled && row.enabled;
        if newer_fires == 0 && !cancel {
            return Ok(());
        }
        if newer_fires > 0 {
            row.trigger_count = row.trigger_count.saturating_add(newer_fires);
            row.last_triggered_at = rebuilt.last_triggered_at;
            row.next_trigger_at = rebuilt.next_trigger_at;
            row.enabled = row.enabled && rebuilt.next_trigger_at.is_some();
        }
        if cancel {
            row.enabled = false;
        }
        row.updated_at = Utc::now();
        let metadata = self
            .db
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT metadata FROM local_schedules WHERE id = ?1 AND org_id = ?2",
                    rusqlite::params![row.id.to_string(), self.org_id],
                    |r| r.get::<_, String>(0),
                )
            })
            .map_err(AgentLoopError::from)?;
        let metadata: Value = serde_json::from_str(&metadata)
            .map_err(|e| AgentLoopError::from(LocalError::from(e)))?;
        self.insert(&row, &metadata)?;
        report.advanced += 1;
        Ok(())
    }
}

fn next_cron_trigger(schedule: &SessionSchedule, after: DateTime<Utc>) -> Result<DateTime<Utc>> {
    let expression = schedule
        .cron_expression
        .as_deref()
        .ok_or_else(|| AgentLoopError::config("recurring schedule has no cron expression"))?;
    let fields: Vec<_> = expression.split_whitespace().collect();
    let normalized = match fields.len() {
        5 => format!("0 {} *", fields.join(" ")),
        6 | 7 => expression.to_string(),
        _ => {
            return Err(AgentLoopError::config(format!(
                "invalid cron expression '{expression}'"
            )));
        }
    };
    let cron = cron::Schedule::from_str(&normalized).map_err(|error| {
        AgentLoopError::config(format!("invalid cron expression '{expression}': {error}"))
    })?;
    let timezone = Tz::from_str(&schedule.timezone).map_err(|error| {
        AgentLoopError::config(format!(
            "invalid IANA timezone '{}': {error}",
            schedule.timezone
        ))
    })?;
    let local_after = after.with_timezone(&timezone);
    cron.after(&local_after)
        .next()
        .map(|next| next.with_timezone(&Utc))
        .ok_or_else(|| AgentLoopError::config("cron expression has no future occurrence"))
}

#[async_trait]
impl SessionScheduleStore for LocalScheduleStore {
    async fn create_schedule(
        &self,
        session_id: SessionId,
        description: String,
        cron_expression: Option<String>,
        scheduled_at: Option<DateTime<Utc>>,
        timezone: String,
    ) -> Result<SessionSchedule> {
        // Trait path: empty metadata bag.
        self.create_schedule_with_metadata(
            session_id,
            description,
            cron_expression,
            scheduled_at,
            timezone,
            Value::Object(Default::default()),
        )
        .await
    }

    async fn create_schedule_enforcing_limits(
        &self,
        session_id: SessionId,
        description: String,
        cron_expression: Option<String>,
        scheduled_at: Option<DateTime<Utc>>,
        timezone: String,
    ) -> std::result::Result<SessionSchedule, ScheduleLimitError> {
        if let Some(cron) = cron_expression.as_deref() {
            validate_cron_min_interval_with(cron, schedule_limits().0)
                .map_err(ScheduleLimitError::Rejected)?;
        }

        let schedule = self
            .build_schedule(
                session_id,
                description,
                cron_expression,
                scheduled_at,
                timezone,
            )
            .map_err(ScheduleLimitError::Store)?;
        let snapshot = serde_json::to_string(&schedule)
            .map_err(|e| ScheduleLimitError::Store(AgentLoopError::from(LocalError::from(e))))?;
        let metadata_json = serde_json::to_string(&Value::Object(Default::default()))
            .map_err(|e| ScheduleLimitError::Store(AgentLoopError::from(LocalError::from(e))))?;
        let id = schedule.id.to_string();
        let session = session_id.to_string();
        let enabled = schedule.enabled as i64;
        let org_id = self.org_id;
        let max_per_org = schedule_limits().1;

        let inserted = self
            .db
            .with_conn(|conn| {
                let active_session_count: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM local_schedules
                     WHERE org_id = ?1 AND session_id = ?2 AND enabled = 1",
                    rusqlite::params![org_id, session],
                    |row| row.get(0),
                )?;
                let active_org_count: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM local_schedules
                     WHERE org_id = ?1 AND enabled = 1",
                    rusqlite::params![org_id],
                    |row| row.get(0),
                )?;
                if active_session_count >= i64::from(MAX_ACTIVE_SCHEDULES_PER_SESSION) {
                    return Ok(false);
                }
                if active_org_count >= max_per_org {
                    return Ok(false);
                }
                conn.execute(
                    "INSERT INTO local_schedules (id, org_id, session_id, enabled, snapshot, metadata, next_trigger_at_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    rusqlite::params![id, org_id, session, enabled, snapshot, metadata_json, schedule.next_trigger_at.map(|time| time.timestamp_millis())],
                )?;
                Ok(true)
            })
            .map_err(|e| ScheduleLimitError::Store(AgentLoopError::from(e)))?;

        if inserted {
            let entry = journal::timer_set(&schedule, &Value::Object(Default::default()));
            if let Err(error) = self.record(session_id, entry).await {
                // The limit check needs the row first; a create the log refused
                // must not stay behind as a schedule the log never heard of.
                let _ = self.delete_row(schedule.id);
                return Err(ScheduleLimitError::Store(error));
            }
            Ok(schedule)
        } else if self
            .count_active_schedules(session_id)
            .await
            .map_err(ScheduleLimitError::Store)?
            >= MAX_ACTIVE_SCHEDULES_PER_SESSION
        {
            Err(ScheduleLimitError::Rejected(format!(
                "Maximum {MAX_ACTIVE_SCHEDULES_PER_SESSION} active schedules per session. Cancel an existing schedule first."
            )))
        } else {
            Err(ScheduleLimitError::Rejected(format!(
                "Maximum {max_per_org} active schedules per org reached. Cancel an existing schedule first."
            )))
        }
    }

    async fn cancel_schedule(
        &self,
        _session_id: SessionId,
        schedule_id: ScheduleId,
    ) -> Result<SessionSchedule> {
        let mut schedule = self
            .load(schedule_id)?
            .ok_or_else(|| AgentLoopError::tool("schedule not found".to_string()))?;
        if schedule.enabled {
            self.record(schedule.session_id, journal::timer_cancelled(schedule_id))
                .await?;
        }
        schedule.enabled = false;
        schedule.updated_at = Utc::now();
        // Preserve existing metadata across the snapshot rewrite.
        let metadata = self
            .get_metadata(schedule_id)
            .await?
            .unwrap_or_else(|| Value::Object(Default::default()));
        self.insert(&schedule, &metadata)?;
        Ok(schedule)
    }

    async fn list_schedules(&self, session_id: SessionId) -> Result<Vec<SessionSchedule>> {
        let session = session_id.to_string();
        let org_id = self.org_id;
        let snapshots: Vec<String> = self
            .db
            .with_conn(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT snapshot FROM local_schedules
                     WHERE org_id = ?1 AND session_id = ?2 ORDER BY rowid ASC",
                )?;
                stmt.query_map(rusqlite::params![org_id, session], |row| row.get(0))?
                    .collect::<rusqlite::Result<Vec<String>>>()
            })
            .map_err(AgentLoopError::from)?;
        snapshots
            .into_iter()
            .map(|json| {
                serde_json::from_str(&json).map_err(|e| AgentLoopError::from(LocalError::from(e)))
            })
            .collect()
    }

    async fn count_active_schedules(&self, session_id: SessionId) -> Result<u32> {
        let session = session_id.to_string();
        let org_id = self.org_id;
        let count: i64 = self
            .db
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM local_schedules
                     WHERE org_id = ?1 AND session_id = ?2 AND enabled = 1",
                    rusqlite::params![org_id, session],
                    |row| row.get(0),
                )
            })
            .map_err(AgentLoopError::from)?;
        Ok(count as u32)
    }

    async fn count_active_org_schedules(&self) -> Result<u32> {
        let org_id = self.org_id;
        let count: i64 = self
            .db
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM local_schedules
                     WHERE org_id = ?1 AND enabled = 1",
                    rusqlite::params![org_id],
                    |row| row.get(0),
                )
            })
            .map_err(AgentLoopError::from)?;
        Ok(count as u32)
    }
}

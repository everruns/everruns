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

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use everruns_core::error::{AgentLoopError, Result};
use everruns_core::session_schedule::SessionSchedule;
use everruns_core::traits::SessionScheduleStore;
use everruns_core::typed_id::{PrincipalId, ScheduleId, SessionId};
use rusqlite::OptionalExtension;
use serde_json::Value;

use crate::db::SqliteDb;
use crate::error::LocalError;

/// SQLite-backed schedule store for local embedded hosts.
#[derive(Clone)]
pub struct LocalScheduleStore {
    db: SqliteDb,
    /// Internal org id this store instance is scoped to.
    org_id: i64,
    /// Principal stamped on created schedules.
    owner_principal_id: PrincipalId,
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
                    metadata    TEXT NOT NULL DEFAULT '{}'
                 );
                 CREATE INDEX IF NOT EXISTS idx_local_schedules_session
                    ON local_schedules(org_id, session_id);",
            )
        })
        .map_err(AgentLoopError::from)?;
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
        }
    }

    fn build_schedule(
        &self,
        session_id: SessionId,
        description: String,
        cron_expression: Option<String>,
        scheduled_at: Option<DateTime<Utc>>,
        timezone: String,
    ) -> SessionSchedule {
        let now = Utc::now();
        SessionSchedule {
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
        }
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
        self.db
            .with_conn(|conn| {
                conn.execute(
                    "INSERT INTO local_schedules (id, org_id, session_id, enabled, snapshot, metadata)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                     ON CONFLICT(id) DO UPDATE SET
                        enabled = excluded.enabled,
                        snapshot = excluded.snapshot,
                        metadata = excluded.metadata",
                    rusqlite::params![id, org_id, session, enabled, snapshot, metadata_json],
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
        );
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

    async fn cancel_schedule(
        &self,
        _session_id: SessionId,
        schedule_id: ScheduleId,
    ) -> Result<SessionSchedule> {
        let mut schedule = self
            .load(schedule_id)?
            .ok_or_else(|| AgentLoopError::tool("schedule not found".to_string()))?;
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

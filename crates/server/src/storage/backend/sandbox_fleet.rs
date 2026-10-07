//! Org-wide Sandbox fleet reads: list, roll-ups, lifecycle history.
//!
//! Decision: the fleet is a hosted (PostgreSQL) surface. It reads the logical
//! `sandboxes` rows, their physical `sandbox_instances`, and the trigger-written
//! `sandbox_state_transitions` log (migration 175). The in-memory backend has no
//! durable Sandbox state to report, so it answers with an empty fleet.
//!
//! Decision: the user-facing state and the "needs attention" reasons are
//! computed in SQL, so filtering and paging happen in one query instead of
//! loading the org's whole history.

use super::*;

/// Window used by the roll-ups that count "recent" activity.
pub const FLEET_STATS_WINDOW_DAYS: i64 = 7;

/// A Sandbox that is running with no recorded activity for this long needs attention.
pub const IDLE_RUNNING_ATTENTION_SECONDS: i64 = 3600;

/// Fleet states that count as "live": the Sandbox exists, or should, at its provider.
pub const LIVE_FLEET_STATES: &[&str] = &["running", "paused", "lost", "starting", "failed"];

/// Map `sandboxes.observed_state` to the state the fleet shows.
pub fn fleet_state(observed_state: &str) -> &'static str {
    match observed_state {
        "ready" => "running",
        "paused" => "paused",
        "lost" => "lost",
        "provisioning" => "starting",
        "failed" => "failed",
        "absent" => "not_started",
        _ => "deleted",
    }
}

const FLEET_STATE_SQL: &str = "CASE s.observed_state \
    WHEN 'ready' THEN 'running' WHEN 'paused' THEN 'paused' WHEN 'lost' THEN 'lost' \
    WHEN 'provisioning' THEN 'starting' WHEN 'failed' THEN 'failed' \
    WHEN 'absent' THEN 'not_started' ELSE 'deleted' END";

/// One row of the fleet: the logical Sandbox, what owns it, and its latest incarnation.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SandboxFleetRow {
    pub id: Uuid,
    pub session_id: Option<Uuid>,
    pub session_title: Option<String>,
    pub agent_id: Option<Uuid>,
    pub agent_name: Option<String>,
    pub provider: String,
    pub role: String,
    pub target_kind: Option<String>,
    pub idle_after_seconds: Option<i64>,
    pub fleet_state: String,
    pub desired_state: String,
    pub generation: i64,
    pub template_id: Option<Uuid>,
    pub template_name: Option<String>,
    pub template_revision: Option<i32>,
    pub external_id: Option<String>,
    pub workspace_path: Option<String>,
    pub last_init_error: Option<String>,
    pub checkpoint_count: i64,
    pub attention: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_activity_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

/// Filters shared by the fleet list, roll-ups and timeline.
#[derive(Debug, Clone, Default)]
pub struct SandboxFleetFilter {
    pub states: Option<Vec<String>>,
    pub providers: Option<Vec<String>>,
    pub agent_id: Option<Uuid>,
    pub template_id: Option<Uuid>,
    pub needs_attention: bool,
    pub search: Option<String>,
    /// In-process targets (virtual filesystem, host) have no provider resource
    /// and would drown the fleet; they are left out unless asked for.
    pub include_in_process: bool,
    pub ids: Option<Vec<Uuid>>,
}

/// Physical incarnation of a logical Sandbox.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SandboxInstanceRow {
    pub generation: i64,
    pub external_id: String,
    pub status: String,
    pub last_init_error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub retired_at: Option<DateTime<Utc>>,
}

/// One entry of the lifecycle log, with the time the next entry replaced it.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SandboxTransitionRow {
    pub sandbox_id: Uuid,
    pub generation: i64,
    pub state: String,
    pub start_at: DateTime<Utc>,
    pub end_at: DateTime<Utc>,
    /// The Sandbox is still in this state.
    pub current: bool,
}

/// Aggregates for the fleet's summary strip.
#[derive(Debug, Clone, Default)]
pub struct SandboxFleetAggregates {
    pub by_state: Vec<(String, i64)>,
    pub live_by_provider: Vec<(String, i64)>,
    pub created_in_window: i64,
    pub created_in_prior_window: i64,
    pub running_seconds_in_window: i64,
    pub recoveries_in_window: i64,
    pub needs_attention: i64,
}

/// The fleet projection. `$1` is the org; filters bind from `$2` on.
///
/// Every interpolated piece of this and the queries built on it is a constant
/// of this module; values always travel as bind parameters, which is what the
/// `AssertSqlSafe` wrappers below rely on.
fn fleet_cte() -> String {
    format!(
        r#"
        WITH fleet AS (
            SELECT s.id, s.session_id,
                   COALESCE(sess.title, s.session_title) AS session_title,
                   a.id AS agent_id,
                   COALESCE(NULLIF(a.display_name, ''), a.name) AS agent_name,
                   s.provider, s.role,
                   s.profile_snapshot->'target'->>'kind' AS target_kind,
                   (s.profile_snapshot->'lifecycle'->>'idle_after_seconds')::bigint
                       AS idle_after_seconds,
                   {FLEET_STATE_SQL} AS fleet_state,
                   s.desired_state, s.generation,
                   env.id AS template_id, env.display_name AS template_name,
                   rev.revision AS template_revision,
                   i.external_id, i.workspace_path, i.last_init_error,
                   (SELECT count(*) FROM sandbox_checkpoints c
                     WHERE c.sandbox_id = s.id AND c.attached_at IS NOT NULL) AS checkpoint_count,
                   ARRAY_REMOVE(ARRAY[
                       CASE WHEN s.observed_state = 'lost' THEN 'lost' END,
                       CASE WHEN s.observed_state = 'failed' THEN 'failed' END,
                       CASE WHEN i.last_init_error IS NOT NULL
                             AND s.observed_state NOT IN ('deleted', 'deleting')
                            THEN 'init_failed' END,
                       CASE WHEN s.observed_state = 'ready'
                             AND COALESCE(s.last_activity_at, s.updated_at)
                                 < now() - make_interval(secs => {IDLE_RUNNING_ATTENTION_SECONDS})
                            THEN 'idle_running' END,
                       CASE WHEN i.external_id IS NOT NULL AND EXISTS (
                                SELECT 1 FROM leased_resources lr
                                 WHERE lr.org_id = s.org_id
                                   AND lr.external_id = i.external_id
                                   AND lr.status = 'cleanup_failed')
                            THEN 'cleanup_failed' END
                   ]::text[], NULL) AS attention,
                   s.created_at, s.updated_at, s.last_activity_at, s.deleted_at
            FROM sandboxes s
            LEFT JOIN sessions sess ON sess.id = s.session_id
            LEFT JOIN agents a ON a.id = COALESCE(sess.agent_id, s.agent_id)
            LEFT JOIN execution_environment_revisions rev ON rev.id = s.environment_revision_id
            LEFT JOIN execution_environments env ON env.id = rev.environment_id
            LEFT JOIN LATERAL (
                SELECT external_id, workspace_path, last_init_error
                FROM sandbox_instances
                WHERE sandbox_id = s.id
                ORDER BY generation DESC
                LIMIT 1
            ) i ON true
            WHERE s.org_id = $1
        ),
        filtered AS (
            SELECT * FROM fleet
            WHERE ($2::text[] IS NULL OR fleet_state = ANY($2))
              AND ($3::text[] IS NULL OR provider = ANY($3))
              AND ($4::uuid IS NULL OR agent_id = $4)
              AND ($5::uuid IS NULL OR template_id = $5)
              AND (NOT $6 OR cardinality(attention) > 0)
              AND ($7::text IS NULL
                   OR session_title ILIKE $7 OR external_id ILIKE $7
                   OR agent_name ILIKE $7 OR provider ILIKE $7)
              AND ($8 OR target_kind IS NULL OR target_kind NOT IN ('vfs', 'host'))
              AND ($9::uuid[] IS NULL OR id = ANY($9))
        )
        "#
    )
}

fn search_pattern(search: &Option<String>) -> Option<String> {
    search
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| {
            let escaped = s
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_");
            format!("%{escaped}%")
        })
}

macro_rules! bind_fleet_filter {
    ($query:expr, $org_id:expr, $filter:expr, $search:expr) => {
        $query
            .bind($org_id)
            .bind($filter.states.as_deref())
            .bind($filter.providers.as_deref())
            .bind($filter.agent_id)
            .bind($filter.template_id)
            .bind($filter.needs_attention)
            .bind($search)
            .bind($filter.include_in_process)
            .bind($filter.ids.as_deref())
    };
}

impl StorageBackend {
    /// Page through the org's Sandboxes, live first, most recently active first.
    pub async fn list_sandbox_fleet(
        &self,
        org_id: i64,
        filter: &SandboxFleetFilter,
        limit: i64,
        offset: i64,
    ) -> Result<(Vec<SandboxFleetRow>, i64)> {
        let db = self.database();
        let search = search_pattern(&filter.search);
        let sql = format!(
            "{} SELECT *, count(*) OVER () AS total FROM filtered \
             ORDER BY (fleet_state IN ('running', 'paused', 'lost', 'starting', 'failed')) DESC, \
                      COALESCE(last_activity_at, updated_at) DESC, id \
             LIMIT $10 OFFSET $11",
            fleet_cte()
        );
        #[derive(sqlx::FromRow)]
        struct Paged {
            #[sqlx(flatten)]
            row: SandboxFleetRow,
            total: i64,
        }
        let rows: Vec<Paged> = bind_fleet_filter!(
            sqlx::query_as(sqlx::AssertSqlSafe(sql)),
            org_id,
            filter,
            &search
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(db.pool())
        .await?;
        let total = rows.first().map(|r| r.total).unwrap_or(0);
        Ok((rows.into_iter().map(|r| r.row).collect(), total))
    }

    /// One Sandbox of the org, whatever its state.
    pub async fn get_sandbox_fleet_row(
        &self,
        org_id: i64,
        sandbox_id: Uuid,
    ) -> Result<Option<SandboxFleetRow>> {
        let filter = SandboxFleetFilter {
            include_in_process: true,
            ids: Some(vec![sandbox_id]),
            ..Default::default()
        };
        let (mut rows, _) = self.list_sandbox_fleet(org_id, &filter, 1, 0).await?;
        Ok(rows.pop())
    }

    /// Physical incarnations, newest first.
    pub async fn list_sandbox_instances(
        &self,
        org_id: i64,
        sandbox_id: Uuid,
    ) -> Result<Vec<SandboxInstanceRow>> {
        let db = self.database();
        Ok(sqlx::query_as(
            r#"
            SELECT i.generation, i.external_id, i.status, i.last_init_error,
                   i.created_at, i.retired_at
            FROM sandbox_instances i
            JOIN sandboxes s ON s.id = i.sandbox_id
            WHERE s.org_id = $1 AND s.id = $2
            ORDER BY i.generation DESC
            "#,
        )
        .bind(org_id)
        .bind(sandbox_id)
        .fetch_all(db.pool())
        .await?)
    }

    /// Lifecycle intervals overlapping `[from, to)`, clipped to it. Pass
    /// `sandbox_id` for one Sandbox's history.
    pub async fn list_sandbox_transitions(
        &self,
        org_id: i64,
        sandbox_id: Option<Uuid>,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<SandboxTransitionRow>> {
        let db = self.database();
        Ok(sqlx::query_as(
            r#"
            WITH t AS (
                SELECT tr.sandbox_id, tr.generation, tr.state, tr.at,
                       lead(tr.at) OVER (PARTITION BY tr.sandbox_id ORDER BY tr.at, tr.id) AS next_at
                FROM sandbox_state_transitions tr
                WHERE tr.org_id = $1
                  AND ($2::uuid IS NULL OR tr.sandbox_id = $2)
                  AND tr.at < $4
            )
            SELECT sandbox_id, generation, state,
                   GREATEST(at, $3) AS start_at,
                   LEAST(COALESCE(next_at, now()), $4) AS end_at,
                   next_at IS NULL AS current
            FROM t
            WHERE next_at IS NULL OR next_at > $3
            ORDER BY sandbox_id, at
            "#,
        )
        .bind(org_id)
        .bind(sandbox_id)
        .bind(from)
        .bind(to)
        .fetch_all(db.pool())
        .await?)
    }

    /// Roll-ups for the fleet summary over the trailing stats window.
    pub async fn sandbox_fleet_aggregates(
        &self,
        org_id: i64,
        filter: &SandboxFleetFilter,
    ) -> Result<SandboxFleetAggregates> {
        let db = self.database();
        let search = search_pattern(&filter.search);
        let window = format!("make_interval(days => {FLEET_STATS_WINDOW_DAYS})");

        let by_state: Vec<(String, i64)> = bind_fleet_filter!(
            sqlx::query_as(sqlx::AssertSqlSafe(format!(
                "{} SELECT fleet_state, count(*) FROM filtered GROUP BY fleet_state ORDER BY 1",
                fleet_cte()
            ))),
            org_id,
            filter,
            &search
        )
        .fetch_all(db.pool())
        .await?;

        let live_by_provider: Vec<(String, i64)> = bind_fleet_filter!(
            sqlx::query_as(sqlx::AssertSqlSafe(format!(
                "{} SELECT provider, count(*) FROM filtered \
                 WHERE fleet_state IN ('running', 'paused', 'lost', 'starting', 'failed') \
                 GROUP BY provider ORDER BY 2 DESC, 1",
                fleet_cte()
            ))),
            org_id,
            filter,
            &search
        )
        .fetch_all(db.pool())
        .await?;

        let (created_in_window, created_in_prior_window, needs_attention): (i64, i64, i64) =
            bind_fleet_filter!(
                sqlx::query_as(sqlx::AssertSqlSafe(format!(
                    "{} SELECT \
                       count(*) FILTER (WHERE created_at >= now() - {window}), \
                       count(*) FILTER (WHERE created_at >= now() - 2 * {window} \
                                          AND created_at < now() - {window}), \
                       count(*) FILTER (WHERE cardinality(attention) > 0) \
                     FROM filtered",
                    fleet_cte()
                ))),
                org_id,
                filter,
                &search
            )
            .fetch_one(db.pool())
            .await?;

        let (running_seconds_in_window, recoveries_in_window): (i64, i64) =
            bind_fleet_filter!(
                sqlx::query_as(sqlx::AssertSqlSafe(format!(
                    "{}, t AS ( \
                       SELECT tr.state, tr.at, tr.generation, \
                              COALESCE(lead(tr.at) OVER (PARTITION BY tr.sandbox_id \
                                                         ORDER BY tr.at, tr.id), now()) AS until, \
                              lag(tr.generation) OVER (PARTITION BY tr.sandbox_id \
                                                       ORDER BY tr.at, tr.id) AS prev_generation \
                       FROM sandbox_state_transitions tr \
                       JOIN filtered f ON f.id = tr.sandbox_id \
                       WHERE tr.org_id = $1) \
                     SELECT \
                       COALESCE(SUM(EXTRACT(EPOCH FROM until - GREATEST(at, now() - {window}))) \
                                FILTER (WHERE state = 'ready' AND until > now() - {window}), 0)::bigint, \
                       count(*) FILTER (WHERE prev_generation IS NOT NULL \
                                          AND generation > prev_generation \
                                          AND at >= now() - {window}) \
                     FROM t",
                    fleet_cte()
                ))),
                org_id,
                filter,
                &search
            )
            .fetch_one(db.pool())
            .await?;

        Ok(SandboxFleetAggregates {
            by_state,
            live_by_provider,
            created_in_window,
            created_in_prior_window,
            running_seconds_in_window,
            recoveries_in_window,
            needs_attention,
        })
    }
}

//! Read-side monitoring over committed event envelopes and actor ownership.
//! Prompt, tool argument, result, and agent names never enter this projection.

use super::Database;
use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::{FromRow, QueryBuilder};
use uuid::Uuid;

pub const MONITORING_ACTOR_PAGE: i64 = 200;
pub const MONITORING_COHORT_LIMIT: i64 = 4096;

// Counts and detail use the same classification. A parked actor is not
// assigned to its previous holder after that lease has been released.
// THREAT[TM-TENANT-002]: owning-session scope remains authoritative when
// a denormalized actor org is stale; validate it at the population boundary.
const POPULATION: &str = r#"
WITH candidates AS MATERIALIZED (
    SELECT id FROM sessions WHERE ($1::bigint IS NULL OR org_id = $1) AND ($3::uuid IS NULL OR id = $3)
      AND status IN ('active', 'waiting_for_tool_results', 'resolving_tool_results', 'paused')
    UNION
    SELECT session_id FROM session_actors WHERE ($1::bigint IS NULL OR org_id = $1) AND ($3::uuid IS NULL OR session_id = $3)
      AND (runnable_since IS NOT NULL OR lease_holder IS NOT NULL OR $3::uuid IS NOT NULL)
), population AS MATERIALIZED (
    SELECT s.id AS session_id, o.public_id::text AS org_id, s.status AS session_status,
        a.lease_holder, a.lease_fence, a.lease_expires_at, a.lease_renewed_at,
        a.runnable_since, a.inbox_seq, a.consumed_seq, a.next_input_seq,
        a.attempts, a.cancel_requested_at,
        (a.lease_holder IS NOT NULL AND a.lease_expires_at > $2) AS leased,
        (a.lease_holder IS NOT NULL AND a.lease_expires_at <= $2) AS expired,
        CASE WHEN a.lease_holder IS NOT NULL AND a.lease_expires_at > $2
            THEN a.lease_holder END AS live_holder,
        CASE
            WHEN a.runnable_since IS NOT NULL AND
                (a.lease_holder IS NULL OR a.lease_expires_at <= $2) THEN 'runnable'
            WHEN s.status IN ('waiting_for_tool_results', 'paused') THEN 'waiting'
            WHEN s.status NOT IN ('active', 'resolving_tool_results')
                AND a.runnable_since IS NULL AND a.lease_holder IS NULL THEN 'idle'
            WHEN p.event_type = 'reason.started' THEN 'model'
            WHEN p.event_type = 'act.started' THEN 'tools'
            ELSE 'setup'
        END AS phase,
        COALESCE(p.ts, a.runnable_since, s.updated_at) AS phase_since,
        CASE WHEN a.runnable_since IS NOT NULL AND
            (a.lease_holder IS NULL OR a.lease_expires_at <= $2)
            THEN GREATEST(0, EXTRACT(EPOCH FROM ($2 -
                CASE WHEN a.lease_expires_at <= $2
                    THEN GREATEST(a.runnable_since, a.lease_expires_at)
                    ELSE a.runnable_since END)) * 1000)::bigint END AS runnable_age_ms
    FROM candidates c JOIN sessions s ON s.id = c.id
    JOIN organizations o ON o.org_id = s.org_id
    LEFT JOIN session_actors a ON a.session_id = s.id
    LEFT JOIN LATERAL (
        SELECT event_type, ts FROM events WHERE session_id = s.id AND event_type IN
            ('turn.started', 'reason.started', 'reason.completed', 'act.started',
             'act.completed', 'turn.completed', 'turn.failed', 'turn.cancelled', 'turn.sealed')
        ORDER BY sequence DESC LIMIT 1
    ) p ON true
    WHERE ($1::bigint IS NULL OR s.org_id = $1)
)
"#;

#[derive(Debug, Clone, FromRow)]
pub struct MonitoringCohortRow {
    pub phase: String,
    pub org_id: String,
    pub holder: Option<String>,
    pub actors: i64,
    pub leased: i64,
    pub expired: i64,
    pub oldest_runnable_ms: Option<i64>,
    pub oldest_phase_ms: i64,
}

#[derive(Debug, Clone, FromRow)]
pub struct MonitoringActorRow {
    pub session_id: Uuid,
    pub org_id: String,
    pub session_status: String,
    pub phase: String,
    pub phase_since: DateTime<Utc>,
    pub lease_holder: Option<String>,
    pub lease_fence: Option<i64>,
    pub lease_expires_at: Option<DateTime<Utc>>,
    pub lease_renewed_at: Option<DateTime<Utc>>,
    pub runnable_since: Option<DateTime<Utc>>,
    pub runnable_age_ms: Option<i64>,
    pub inbox_seq: Option<i64>,
    pub consumed_seq: Option<i64>,
    pub next_input_seq: Option<i64>,
    pub attempts: Option<i32>,
    pub cancel_requested_at: Option<DateTime<Utc>>,
    pub leased: Option<bool>,
    pub expired: Option<bool>,
}

#[derive(Debug, Clone, FromRow)]
pub struct MonitoringRateRow {
    pub event_type: String,
    pub count: i64,
}

#[derive(Debug, Clone, FromRow)]
pub struct MonitoringLeaseEventRow {
    pub occurred_at: DateTime<Utc>,
    pub kind: String,
    pub holder: String,
    pub fence: i64,
    pub previous_holder: Option<String>,
    pub previous_expires_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, FromRow)]
pub struct MonitoringStepRow {
    pub turn_no: i32,
    pub step_no: i32,
    pub kind: String,
    pub status: String,
    pub started_at: DateTime<Utc>,
    pub duration_ms: Option<i64>,
}

#[derive(Debug, Clone, FromRow)]
pub struct MonitoringTotalRow {
    pub actors: i64,
    pub organizations: i64,
    pub leased: i64,
    pub expired: i64,
    pub oldest_runnable_ms: Option<i64>,
    pub runnable: i64,
    pub model: i64,
    pub tools: i64,
    pub waiting: i64,
    pub setup: i64,
}

#[derive(Debug, Clone, FromRow)]
pub struct MonitoringWorkerRow {
    pub id: String,
    pub worker_group: Option<String>,
    pub status: String,
    pub current_load: i64,
    pub max_concurrency: i64,
    pub accepting_tasks: bool,
    pub last_heartbeat_at: DateTime<Utc>,
}

impl Database {
    pub async fn monitoring_totals(
        &self,
        org: Option<i64>,
        now: DateTime<Utc>,
    ) -> Result<MonitoringTotalRow> {
        let mut sql = QueryBuilder::new(POPULATION);
        sql.push(
            " SELECT COUNT(*) AS actors, COUNT(DISTINCT org_id) AS organizations, \
            COUNT(*) FILTER (WHERE leased) AS leased, COUNT(*) FILTER (WHERE expired) AS expired, \
            MAX(runnable_age_ms) AS oldest_runnable_ms, \
            COUNT(*) FILTER (WHERE phase = 'runnable') AS runnable, \
            COUNT(*) FILTER (WHERE phase = 'model') AS model, \
            COUNT(*) FILTER (WHERE phase = 'tools') AS tools, \
            COUNT(*) FILTER (WHERE phase = 'waiting') AS waiting, \
            COUNT(*) FILTER (WHERE phase = 'setup') AS setup FROM population",
        );
        Ok(sql
            .build_query_as()
            .bind(org)
            .bind(now)
            .bind(None::<Uuid>)
            .fetch_one(&self.pool)
            .await?)
    }

    pub async fn monitoring_cohorts(
        &self,
        org: Option<i64>,
        now: DateTime<Utc>,
    ) -> Result<Vec<MonitoringCohortRow>> {
        let mut sql = QueryBuilder::new(POPULATION);
        sql.push(" SELECT phase, org_id, live_holder AS holder, COUNT(*) AS actors, \
            COUNT(*) FILTER (WHERE leased) AS leased, COUNT(*) FILTER (WHERE expired) AS expired, \
            MAX(runnable_age_ms) AS oldest_runnable_ms, \
            GREATEST(0, EXTRACT(EPOCH FROM ($2 - MIN(phase_since))) * 1000)::bigint AS oldest_phase_ms \
            FROM population GROUP BY phase, org_id, live_holder \
            ORDER BY COUNT(*) DESC, org_id, live_holder, phase LIMIT $4");
        Ok(sql
            .build_query_as()
            .bind(org)
            .bind(now)
            .bind(None::<Uuid>)
            .bind(MONITORING_COHORT_LIMIT + 1)
            .fetch_all(&self.pool)
            .await?)
    }

    pub async fn monitoring_actors(
        &self,
        org: Option<i64>,
        now: DateTime<Utc>,
        offset: i64,
        worker: Option<&str>,
        phase: Option<&str>,
    ) -> Result<Vec<MonitoringActorRow>> {
        let mut sql = QueryBuilder::new(POPULATION);
        sql.push(" SELECT * FROM population \
            WHERE ($6::text IS NULL OR live_holder = $6) AND ($7::text IS NULL OR phase = $7) \
            ORDER BY expired DESC NULLS LAST, runnable_age_ms DESC NULLS LAST, session_id LIMIT $4 OFFSET $5");
        Ok(sql
            .build_query_as()
            .bind(org)
            .bind(now)
            .bind(None::<Uuid>)
            .bind(MONITORING_ACTOR_PAGE + 1)
            .bind(offset)
            .bind(worker)
            .bind(phase)
            .fetch_all(&self.pool)
            .await?)
    }

    pub async fn monitoring_actor(
        &self,
        org: Option<i64>,
        now: DateTime<Utc>,
        session: Uuid,
    ) -> Result<Option<MonitoringActorRow>> {
        // Narrow candidates before lateral event reads, not after materializing
        // the fleet. Detail remains independent of the fleet's population size.
        let mut sql = QueryBuilder::new(POPULATION);
        sql.push(" SELECT * FROM population");
        Ok(sql
            .build_query_as()
            .bind(org)
            .bind(now)
            .bind(session)
            .fetch_optional(&self.pool)
            .await?)
    }

    pub async fn monitoring_rates(
        &self,
        org: Option<i64>,
        now: DateTime<Utc>,
    ) -> Result<Vec<MonitoringRateRow>> {
        Ok(sqlx::query_as(r#"
            SELECT e.event_type, COUNT(*) AS count FROM events e JOIN sessions s ON s.id = e.session_id
            WHERE ($1::bigint IS NULL OR s.org_id = $1)
              AND e.ts >= $2 - interval '60 seconds' AND e.ts <= $2
              AND e.event_type IN ('turn.started', 'reason.started', 'act.started', 'tool.started',
                'tool.completed', 'turn.completed', 'turn.failed', 'turn.cancelled', 'turn.sealed')
            GROUP BY e.event_type
        "#).bind(org).bind(now).fetch_all(&self.pool).await?)
    }

    pub async fn monitoring_lease_rates(
        &self,
        org: Option<i64>,
        now: DateTime<Utc>,
    ) -> Result<Vec<MonitoringRateRow>> {
        Ok(sqlx::query_as(
            "SELECT h.kind AS event_type, COUNT(*) AS count FROM session_actor_lease_history h \
             JOIN sessions s ON s.id = h.session_id \
             WHERE ($1::bigint IS NULL OR s.org_id = $1) AND h.occurred_at >= $2 - interval '60 seconds' \
             AND h.occurred_at <= $2 GROUP BY h.kind",
        ).bind(org).bind(now).fetch_all(&self.pool).await?)
    }

    pub async fn monitoring_workers(&self) -> Result<Vec<MonitoringWorkerRow>> {
        Ok(sqlx::query_as(
            "SELECT id, worker_group, status, current_load::bigint, max_concurrency::bigint, \
             accepting_tasks, last_heartbeat_at FROM durable_workers \
             ORDER BY last_heartbeat_at DESC, id LIMIT 2049",
        )
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn monitoring_lease_events(
        &self,
        org: Option<i64>,
        session: Uuid,
    ) -> Result<Vec<MonitoringLeaseEventRow>> {
        Ok(sqlx::query_as(r#"
            SELECT occurred_at, kind, holder, fence, previous_holder, previous_expires_at, expires_at
            FROM session_actor_lease_history WHERE session_id = $2 AND ($1::bigint IS NULL OR org_id = $1)
            ORDER BY id DESC LIMIT 50
        "#).bind(org).bind(session).fetch_all(&self.pool).await?)
    }

    pub async fn monitoring_steps(&self, session: Uuid) -> Result<Vec<MonitoringStepRow>> {
        Ok(sqlx::query_as(
            r#"
            SELECT turn_no, step_no, kind, status, started_at, duration_ms
            FROM session_trace_steps WHERE session_id = $1
              AND turn_no = (SELECT MAX(turn_no) FROM session_trace_turns WHERE session_id = $1)
            ORDER BY step_no DESC LIMIT 100
        "#,
        )
        .bind(session)
        .fetch_all(&self.pool)
        .await?)
    }
}

#[cfg(test)]
mod tests;

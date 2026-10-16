// Failure history reads the same terminal events as agent activity. The trace
// index is asynchronous, so it cannot define which failures the count links to.

use super::{Database, SESSION_COLUMNS, SessionRow};
use anyhow::Result;
use chrono::{DateTime, Utc};
use everruns_contracts::typed_id::AgentId;

#[derive(Debug, sqlx::FromRow)]
pub struct FailedSessionRow {
    #[sqlx(flatten)]
    pub session: SessionRow,
    pub failed_turns: i64,
    pub failure_sequence: i32,
    pub failed_at: DateTime<Utc>,
    pub error: Option<String>,
}

#[derive(Debug, Default, sqlx::FromRow)]
pub struct AgentFailureTotals {
    pub sessions: i64,
    pub failed_turns: i64,
}

// THREAT[TM-TENANT-001][TM-API-001]: all selectors are bound, including org and
// agent. Archived sessions remain included, matching the Agents activity read.
// Restrict candidates by recent activity, then use the per-session turn index.
const FAILURE_SESSIONS: &str = "WITH failure_sessions AS MATERIALIZED (
    SELECT s.id AS session_id, COUNT(*)::BIGINT AS failed_turns,
           MAX(e.ts) AS failed_at
    FROM sessions s JOIN events e ON e.session_id = s.id
    WHERE s.org_id = $1 AND s.agent_id = $2 AND s.updated_at > $3
      AND e.event_type = 'turn.failed' AND e.ts > $3 AND e.ts <= $4
    GROUP BY s.id
)";

impl Database {
    pub async fn agent_failure_sessions(
        &self,
        org_id: i64,
        agent_id: AgentId,
        after: DateTime<Utc>,
        before: DateTime<Utc>,
        offset: u32,
        limit: u32,
    ) -> Result<(Vec<FailedSessionRow>, AgentFailureTotals)> {
        // Totals and pagination describe one snapshot even if sessions are
        // deleted or late-arriving historical events commit between the reads.
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await?;
        let totals = sqlx::query_as::<_, AgentFailureTotals>(sqlx::AssertSqlSafe(format!(
            "{FAILURE_SESSIONS} SELECT COUNT(*)::BIGINT AS sessions,
             COALESCE(SUM(failed_turns), 0)::BIGINT AS failed_turns FROM failure_sessions"
        )))
        .bind(org_id)
        .bind(agent_id)
        .bind(after)
        .bind(before)
        .fetch_one(&mut *tx)
        .await?;
        let rows = sqlx::query_as::<_, FailedSessionRow>(sqlx::AssertSqlSafe(format!(
            "{FAILURE_SESSIONS}, page AS (
                SELECT * FROM failure_sessions
                ORDER BY failed_at DESC, session_id LIMIT $5 OFFSET $6
             ) SELECT sessions.*, page.failed_turns, page.failed_at,
                      failure.failure_sequence, failure.error
             FROM (SELECT {SESSION_COLUMNS} FROM sessions WHERE org_id = $1) sessions
             JOIN page ON page.session_id = sessions.id
             JOIN LATERAL (
                 SELECT e.sequence AS failure_sequence, LEFT(e.data->>'error', 1000) AS error
                 FROM events e WHERE e.session_id = sessions.id
                   AND e.event_type = 'turn.failed' AND e.ts > $3 AND e.ts <= $4
                 ORDER BY e.ts DESC, e.sequence DESC LIMIT 1
             ) failure ON TRUE
             ORDER BY page.failed_at DESC, sessions.id"
        )))
        .bind(org_id)
        .bind(agent_id)
        .bind(after)
        .bind(before)
        .bind(i64::from(limit.clamp(1, 50)))
        .bind(i64::from(offset))
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok((rows, totals))
    }
}

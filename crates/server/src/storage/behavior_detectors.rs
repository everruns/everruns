//! The numbers the session behavior detectors read
//! (`domains/health_issues/behavior.rs`): tool calls that keep failing or
//! keep repeating, and a session spending far above its agent's normal hour.
//!
//! Decision: every query starts from what happened in the last hour, an
//! indexed range on `llm_generations.created_at`, and reaches older history
//! only through the sessions or agents found there, so a sweep's cost follows
//! current activity rather than table size. Eval sessions are skipped: an
//! eval is meant to probe odd behavior.
use super::StorageBackend;
use anyhow::Result;
use uuid::Uuid;

/// The tool call a session made most often among its recent tool calls.
#[derive(Debug, sqlx::FromRow)]
pub struct ToolPatternRow {
    pub org_id: i64,
    pub session_id: Uuid,
    pub agent_id: Option<Uuid>,
    pub tool: String,
    /// Calls of this tool on this same target in the window.
    pub calls: i64,
    /// How many of those failed.
    pub failed: i64,
    /// Tool calls in the window, of any kind.
    pub window_calls: i64,
}

/// A session's spend over the last hour against its agent's usual hour.
#[derive(Debug, sqlx::FromRow)]
pub struct SessionSpendRow {
    pub org_id: i64,
    pub session_id: Uuid,
    pub agent_id: Uuid,
    pub last_hour_usd: f64,
    /// Median spend of one of the agent's sessions in one active hour over
    /// the week before.
    pub baseline_hourly_usd: f64,
    /// Session-hours the median is taken over.
    pub samples: i64,
}

impl StorageBackend {
    /// For each session with model calls in the last hour, the tool call it
    /// repeated most (same tool, same target) among its last `window` tool
    /// calls of that hour, when it was called `min_calls` times or failed
    /// `min_failures` times.
    pub async fn repeated_tool_calls(
        &self,
        window: i64,
        min_failures: i64,
        min_calls: i64,
    ) -> Result<Vec<ToolPatternRow>> {
        Ok(sqlx::query_as(
            "WITH active AS (
                SELECT DISTINCT session_id FROM llm_generations
                WHERE created_at >= NOW() - INTERVAL '1 hour' AND session_id IS NOT NULL
             ), recent AS (
                SELECT active.session_id, t.name, t.target, t.status
                FROM active
                CROSS JOIN LATERAL (
                    SELECT name, target, status FROM session_trace_steps
                    WHERE session_id = active.session_id AND kind = 'tool'
                      AND started_at >= NOW() - INTERVAL '1 hour'
                    ORDER BY turn_no DESC, step_no DESC LIMIT $1
                ) t
             ), totals AS (
                SELECT session_id, COUNT(*) AS window_calls FROM recent GROUP BY session_id
             ), grouped AS (
                SELECT session_id, COALESCE(name, 'unknown') AS tool,
                    COUNT(*) AS calls, COUNT(*) FILTER (WHERE status = 'error') AS failed
                FROM recent GROUP BY session_id, COALESCE(name, 'unknown'), COALESCE(target, '')
             )
             SELECT DISTINCT ON (g.session_id) s.org_id, g.session_id, s.agent_id, g.tool,
                    g.calls, g.failed, t.window_calls
             FROM grouped g
             JOIN totals t ON t.session_id = g.session_id
             JOIN sessions s ON s.id = g.session_id
             WHERE s.source <> 'eval' AND (g.failed >= $2 OR g.calls >= $3)
             ORDER BY g.session_id, g.failed DESC, g.calls DESC",
        )
        .bind(window)
        .bind(min_failures)
        .bind(min_calls)
        .fetch_all(self.database().pool())
        .await?)
    }

    /// Sessions that spent more than `floor_usd` in the last hour and more
    /// than `factor` times their agent's median session-hour over the week
    /// before, judged only once the agent has `min_samples` such hours.
    pub async fn session_spend_spikes(
        &self,
        floor_usd: f64,
        factor: f64,
        min_samples: i64,
    ) -> Result<Vec<SessionSpendRow>> {
        Ok(sqlx::query_as(
            "WITH current AS (
                SELECT session_id,
                    SUM(COALESCE(actual_cost_usd, estimated_cost_usd, 0)) AS usd
                FROM llm_generations
                WHERE created_at >= NOW() - INTERVAL '1 hour' AND session_id IS NOT NULL
                GROUP BY session_id
                HAVING SUM(COALESCE(actual_cost_usd, estimated_cost_usd, 0)) > $1
             ), candidates AS (
                SELECT s.org_id, c.session_id, s.agent_id, c.usd
                FROM current c JOIN sessions s ON s.id = c.session_id
                WHERE s.agent_id IS NOT NULL AND s.source <> 'eval'
             ), hourly AS (
                SELECT s.agent_id, g.session_id, date_trunc('hour', g.created_at) AS hour,
                    SUM(COALESCE(g.actual_cost_usd, g.estimated_cost_usd, 0)) AS usd
                FROM (SELECT DISTINCT agent_id FROM candidates) a
                JOIN sessions s ON s.agent_id = a.agent_id AND s.source <> 'eval'
                JOIN llm_generations g ON g.session_id = s.id
                WHERE g.created_at >= NOW() - INTERVAL '7 days'
                  AND g.created_at < NOW() - INTERVAL '1 hour'
                GROUP BY s.agent_id, g.session_id, date_trunc('hour', g.created_at)
             ), baseline AS (
                SELECT agent_id,
                    percentile_cont(0.5) WITHIN GROUP (ORDER BY usd) AS median,
                    COUNT(*) AS samples
                FROM hourly GROUP BY agent_id
             )
             SELECT c.org_id, c.session_id, c.agent_id, c.usd::float8 AS last_hour_usd,
                    b.median::float8 AS baseline_hourly_usd, b.samples
             FROM candidates c JOIN baseline b ON b.agent_id = c.agent_id
             WHERE b.samples >= $3 AND c.usd > $2 * b.median",
        )
        .bind(floor_usd)
        .bind(factor)
        .bind(min_samples)
        .fetch_all(self.database().pool())
        .await?)
    }
}

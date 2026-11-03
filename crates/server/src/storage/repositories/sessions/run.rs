//! Sessions in one run: the first session and every session started under it.
use super::*;

/// Most sessions of one run read for waiting requests. A run's lead hands work
/// to at most 32 agents, each of which can wait on one batch at a time.
const MAX_WAITING_RUN_SESSIONS: i64 = 64;

impl Database {
    /// Sessions started under `root` (not `root` itself) that wait on tool
    /// results, oldest first. Their questions and approvals are answered from
    /// the first session (`knowledge/runtime-resources/agents-using-agents.md`).
    pub async fn list_waiting_run_sessions(
        &self,
        org_id: i64,
        root: SessionId,
    ) -> Result<Vec<SessionRow>> {
        let sql = format!(
            r#"
            SELECT {SESSION_COLUMNS}
            FROM sessions
            WHERE org_id = $1
              AND root_session_id = $2
              AND id <> $2
              AND status = 'waiting_for_tool_results'
            ORDER BY created_at ASC
            LIMIT $3
            "#,
        );
        // Only the static column projection is interpolated; no caller data.
        let rows = sqlx::query_as::<_, SessionRow>(sqlx::AssertSqlSafe(sql.as_str()))
            .bind(org_id)
            .bind(root)
            .bind(MAX_WAITING_RUN_SESSIONS)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows)
    }
}

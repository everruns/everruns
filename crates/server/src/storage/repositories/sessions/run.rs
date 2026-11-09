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
              AND deleted_at IS NULL
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

    /// The first session of `session`'s run: itself for a top-level session.
    pub async fn get_run_root(&self, session: SessionId) -> Result<Option<SessionId>> {
        let root: Option<Option<uuid::Uuid>> =
            sqlx::query_scalar("SELECT root_session_id FROM sessions WHERE id = $1")
                .bind(session)
                .fetch_optional(&self.pool)
                .await?;
        Ok(root.flatten().map(SessionId::from_uuid))
    }

    /// The first session `root` and every session started under it, first
    /// session first, then by creation. At most `limit` rows.
    pub async fn list_run_sessions(
        &self,
        org_id: i64,
        root: SessionId,
        limit: i64,
    ) -> Result<Vec<SessionRow>> {
        let sql = format!(
            r#"
            SELECT {SESSION_COLUMNS}, root_session_id
            FROM sessions
            WHERE org_id = $1
              AND deleted_at IS NULL
              AND (id = $2 OR root_session_id = $2)
            ORDER BY (id = $2) DESC, created_at ASC
            LIMIT $3
            "#,
        );
        // Only the static column projection is interpolated; no caller data.
        let rows = sqlx::query_as::<_, SessionRow>(sqlx::AssertSqlSafe(sql.as_str()))
            .bind(org_id)
            .bind(root)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows)
    }
}

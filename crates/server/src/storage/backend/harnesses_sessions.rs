//! Harnesses, agent triggers, sessions, workspaces, and memory files.

use super::*;

impl StorageBackend {
    pub async fn get_harness(&self, org_id: i64, id: HarnessId) -> Result<Option<HarnessRow>> {
        #[cfg(test)]
        self.fail_if_forced("get_harness")?;
        #[cfg(test)]
        self.record_session_list_lookup().await;
        dispatch!(self, get_harness, org_id, id)
    }

    pub async fn get_harness_ancestry_by_ids(
        &self,
        org_id: i64,
        ids: &[HarnessId],
    ) -> Result<Vec<HarnessRow>> {
        #[cfg(test)]
        self.record_session_list_lookup().await;
        dispatch!(self, get_harness_ancestry_by_ids, org_id, ids)
    }

    // ============================================
    // Sessions
    // ============================================

    pub async fn get_session(&self, org_id: i64, id: SessionId) -> Result<Option<SessionRow>> {
        #[cfg(test)]
        self.fail_if_forced("get_session")?;
        dispatch!(self, get_session, org_id, id)
    }

    /// Get session without org scoping. For internal system use only (e.g. usage tracking).
    pub async fn get_session_unscoped(&self, id: SessionId) -> Result<Option<SessionRow>> {
        #[cfg(test)]
        self.fail_if_forced("get_session_unscoped")?;
        dispatch!(self, get_session_unscoped, id)
    }

    /// List sessions for an agent with pagination, validating org ownership.
    /// Returns (sessions, total_count).
    pub async fn list_sessions(
        &self,
        org_id: i64,
        filters: &SessionListFilters,
        pagination: Pagination,
    ) -> Result<(Vec<SessionRow>, u32)> {
        #[cfg(test)]
        self.record_session_list_lookup().await;
        dispatch!(self, list_sessions, org_id, filters, pagination)
    }

    /// Whether the session ran on the OpenAI Agents API backend and so has
    /// provider-held state (EVE-1126).
    pub async fn session_has_agents_api_state(&self, org_id: i64, id: SessionId) -> Result<bool> {
        let pool = self.pool();
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM agents_api_sessions WHERE org_id = $1 AND session_id = $2)",
        )
        .bind(org_id)
        .bind(id)
        .fetch_one(pool)
        .await?;
        Ok(exists)
    }

    // ============================================
    // Workspaces (see knowledge/runtime-resources/workspace.md)
    // ============================================

    pub async fn get_workspace(
        &self,
        org_id: i64,
        workspace_id: WorkspaceId,
    ) -> Result<Option<WorkspaceRow>> {
        dispatch!(
            self,
            get_workspace_by_public_id,
            org_id,
            &workspace_id.to_string()
        )
    }

    // ============================================
    // Memories
    // ============================================

    pub async fn get_memory(&self, org_id: i64, memory_id: MemoryId) -> Result<Option<MemoryRow>> {
        dispatch!(
            self,
            get_memory_by_public_id,
            org_id,
            &memory_id.to_string()
        )
    }
}

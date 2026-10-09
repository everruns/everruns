//! Users, principals, tokens, OAuth clients, and agent records.

use super::*;

impl StorageBackend {
    #[cfg(test)]
    fn test_hooks(&self) -> Option<&crate::storage::test_database::TestHooks> {
        self.test_database_handle().map(|database| database.hooks())
    }

    #[cfg(test)]
    pub(crate) async fn record_session_list_lookup(&self) {
        if let Some(hooks) = self.test_hooks() {
            let delay_ms = hooks.record_session_list_lookup();
            if delay_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn reset_session_list_lookup_count(&self) {
        if let Some(hooks) = self.test_hooks() {
            hooks.reset_session_list_lookup_count();
        }
    }

    #[cfg(test)]
    pub(crate) fn session_list_lookup_count(&self) -> usize {
        self.test_hooks()
            .map_or(0, |hooks| hooks.session_list_lookup_count())
    }

    /// Test-only fault injection: the next call to `method` fails with
    /// [`FORCED_STORAGE_FAILURE`]. Transport-boundary tests use this to prove a
    /// storage error never reaches a client verbatim.
    #[cfg(test)]
    pub(crate) fn force_storage_failure(&self, method: &str) {
        if let Some(hooks) = self.test_hooks() {
            hooks.force_failure(method);
        }
    }

    #[cfg(test)]
    pub(crate) fn fail_if_forced(&self, method: &str) -> Result<()> {
        if self
            .test_hooks()
            .is_some_and(|hooks| hooks.take_forced_failure(method))
        {
            anyhow::bail!(FORCED_STORAGE_FAILURE);
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn set_session_list_lookup_delay_ms(&self, delay_ms: u64) {
        if let Some(hooks) = self.test_hooks() {
            hooks.set_session_list_lookup_delay_ms(delay_ms);
        }
    }

    /// Create a PostgreSQL storage backend from a database URL
    pub async fn postgres(database_url: &str) -> Result<Self> {
        Ok(Self::from_database(Database::from_url(database_url).await?))
    }

    /// The request pool.
    pub fn pool(&self) -> &PgPool {
        self.db.pool()
    }

    /// The pool reserved for background sweeps (EVE-1081).
    pub fn background_pool(&self) -> &PgPool {
        self.db.background_pool()
    }

    /// The same storage, routed onto the background pool.
    ///
    /// Background loops take this instead of the request-path backend so a
    /// burst of HTTP traffic cannot starve them, and so their own sweeps
    /// cannot eat the connections requests are waiting for.
    pub fn for_background(&self) -> Self {
        Self::from_database(self.db.for_background())
    }

    /// Attach an object-storage blob backend for content offload.
    pub fn with_blob_store(
        self,
        blob_store: Option<crate::storage::blob_store::SharedBlobStore>,
    ) -> Self {
        Self::from_database(self.db.with_blob_store(blob_store))
    }

    /// The configured object-storage blob backend, if any. `None` for
    /// deployments running with the default inline (`db`) storage, which have
    /// no external objects to garbage-collect.
    pub fn blob_store(&self) -> Option<crate::storage::blob_store::SharedBlobStore> {
        self.db.blob_store().cloned()
    }

    // ============================================
    // Principals
    // ============================================

    pub async fn get_principal(
        &self,
        org_id: i64,
        id: PrincipalId,
    ) -> Result<Option<PrincipalRow>> {
        #[cfg(test)]
        self.record_session_list_lookup().await;
        dispatch!(self, get_principal, org_id, id)
    }

    pub async fn get_principal_by_subject(
        &self,
        org_id: i64,
        kind: &str,
        subject_id: Uuid,
    ) -> Result<Option<PrincipalRow>> {
        #[cfg(test)]
        self.fail_if_forced("get_principal_by_subject")?;
        #[cfg(test)]
        self.record_session_list_lookup().await;
        dispatch!(self, get_principal_by_subject, org_id, kind, subject_id)
    }

    pub async fn get_principals_for_session_list(
        &self,
        org_id: i64,
        principal_ids: &[PrincipalId],
        resolved_user_ids: &[Uuid],
    ) -> Result<Vec<PrincipalRow>> {
        #[cfg(test)]
        self.record_session_list_lookup().await;
        dispatch!(
            self,
            get_principals_for_session_list,
            org_id,
            principal_ids,
            resolved_user_ids
        )
    }

    // ============================================
    // Password Reset / Email Verification Tokens
    // ============================================
    // Hashed, single-use, short-TTL tokens for native-auth account recovery.
    // The raw token is emailed once and never stored; only its SHA-256 hash is
    // persisted. `consume_*` is race-safe (single atomic UPDATE) like
    // `consume_refresh_token_by_hash`.

    // ============================================
    // Agents
    // ============================================

    pub async fn get_agent(&self, org_id: i64, id: AgentId) -> Result<Option<AgentRow>> {
        #[cfg(test)]
        self.fail_if_forced("get_agent")?;
        #[cfg(test)]
        self.record_session_list_lookup().await;
        dispatch!(self, get_agent, org_id, id)
    }

    pub async fn get_agents_by_ids(&self, org_id: i64, ids: &[AgentId]) -> Result<Vec<AgentRow>> {
        #[cfg(test)]
        self.record_session_list_lookup().await;
        dispatch!(self, get_agents_by_ids, org_id, ids)
    }

    pub async fn get_agent_by_public_id(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<AgentRow>> {
        #[cfg(test)]
        self.fail_if_forced("get_agent_by_public_id")?;
        dispatch!(self, get_agent_by_public_id, org_id, public_id)
    }

    pub async fn get_agent_public_id(&self, org_id: i64, id: AgentId) -> Result<Option<String>> {
        #[cfg(test)]
        self.record_session_list_lookup().await;
        dispatch!(self, get_agent_public_id, org_id, id)
    }
}

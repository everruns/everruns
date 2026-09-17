//! Users, principals, tokens, OAuth clients, and agent records.

use super::*;

impl StorageBackend {
    #[cfg(test)]
    pub(crate) async fn record_session_list_lookup(&self) {
        if let Self::InMemory(db) = self {
            db.record_session_list_lookup();
            let delay_ms = db.session_list_lookup_delay_ms();
            if delay_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn reset_session_list_lookup_count(&self) {
        if let Self::InMemory(db) = self {
            db.reset_session_list_lookup_count();
        }
    }

    #[cfg(test)]
    pub(crate) fn session_list_lookup_count(&self) -> usize {
        match self {
            Self::InMemory(db) => db.session_list_lookup_count(),
            Self::Postgres(_) => 0,
        }
    }

    /// Test-only fault injection: the next call to `method` fails with
    /// [`FORCED_STORAGE_FAILURE`]. Transport-boundary tests use this to prove a
    /// storage error never reaches a client verbatim.
    #[cfg(test)]
    pub(crate) fn force_storage_failure(&self, method: &str) {
        if let Self::InMemory(db) = self {
            db.force_failure(method);
        }
    }

    #[cfg(test)]
    pub(crate) fn fail_if_forced(&self, method: &str) -> Result<()> {
        if let Self::InMemory(db) = self
            && db.take_forced_failure(method)
        {
            anyhow::bail!(FORCED_STORAGE_FAILURE);
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn set_session_list_lookup_delay_ms(&self, delay_ms: u64) {
        if let Self::InMemory(db) = self {
            db.set_session_list_lookup_delay_ms(delay_ms);
        }
    }

    /// Create a PostgreSQL storage backend from a database URL
    pub async fn postgres(database_url: &str) -> Result<Self> {
        let db = Database::from_url(database_url).await?;
        Ok(Self::Postgres(db))
    }

    /// Create an in-memory storage backend
    pub fn in_memory() -> Self {
        Self::InMemory(std::sync::Arc::new(InMemoryDatabase::new()))
    }

    /// Check if this is dev mode (in-memory)
    pub fn is_dev_mode(&self) -> bool {
        matches!(self, Self::InMemory(_))
    }

    /// Get the PostgreSQL pool if using PostgreSQL backend
    /// Returns None for in-memory backend
    pub fn pool(&self) -> Option<&PgPool> {
        match self {
            Self::Postgres(db) => Some(db.pool()),
            Self::InMemory(_) => None,
        }
    }

    /// Attach an object-storage blob backend for content offload
    /// (knowledge/runtime-resources/object-storage.md). Only the PostgreSQL backend offloads content;
    /// the in-memory dev backend always stores bytes inline.
    pub fn with_blob_store(
        self,
        blob_store: Option<crate::storage::blob_store::SharedBlobStore>,
    ) -> Self {
        match self {
            Self::Postgres(db) => Self::Postgres(db.with_blob_store(blob_store)),
            other => other,
        }
    }

    /// The configured object-storage blob backend, if any. `None` for the
    /// in-memory dev backend and for PostgreSQL deployments running with the
    /// default inline (`db`) storage — both of which have no external objects
    /// to garbage-collect.
    pub fn blob_store(&self) -> Option<crate::storage::blob_store::SharedBlobStore> {
        match self {
            Self::Postgres(db) => db.blob_store().cloned(),
            Self::InMemory(_) => None,
        }
    }

    // ============================================
    // Users
    // ============================================

    pub async fn create_user(&self, input: CreateUserRow) -> Result<UserRow> {
        dispatch!(self, create_user, input)
    }

    /// Create user with a specific UUID (for seeding).
    /// Returns None if id already exists.
    pub async fn create_user_with_id(
        &self,
        id: Uuid,
        input: CreateUserRow,
    ) -> Result<Option<UserRow>> {
        dispatch!(self, create_user_with_id, id, input)
    }

    pub async fn get_user_by_email(&self, email: &str) -> Result<Option<UserRow>> {
        dispatch!(self, get_user_by_email, email)
    }

    pub async fn get_user(&self, id: Uuid) -> Result<Option<UserRow>> {
        dispatch!(self, get_user, id)
    }

    pub async fn get_user_by_oauth(
        &self,
        provider: &str,
        provider_id: &str,
    ) -> Result<Option<UserRow>> {
        dispatch!(self, get_user_by_oauth, provider, provider_id)
    }

    pub async fn link_oauth_identity(
        &self,
        id: Uuid,
        provider: &str,
        provider_id: &str,
    ) -> Result<Option<UserRow>> {
        dispatch!(self, link_oauth_identity, id, provider, provider_id)
    }

    pub async fn update_user(&self, id: Uuid, input: UpdateUser) -> Result<Option<UserRow>> {
        dispatch!(self, update_user, id, input)
    }

    pub async fn list_users(&self, search: Option<&str>) -> Result<Vec<UserRow>> {
        dispatch!(self, list_users, search)
    }

    pub async fn list_users_by_org(
        &self,
        org_id: i64,
        search: Option<&str>,
    ) -> Result<Vec<UserRow>> {
        dispatch!(self, list_users_by_org, org_id, search)
    }

    /// Hard-delete a user and all associated data (cascading).
    /// Returns true if the user existed and was deleted.
    pub async fn delete_user_account(&self, user_id: Uuid) -> Result<bool> {
        dispatch!(self, delete_user_account, user_id)
    }

    /// Export all user-owned data as a structured JSON value.
    pub async fn export_user_data(&self, user_id: Uuid) -> Result<Option<serde_json::Value>> {
        dispatch!(self, export_user_data, user_id)
    }

    // ============================================
    // Principals
    // ============================================

    pub async fn create_principal(&self, input: CreatePrincipalRow) -> Result<PrincipalRow> {
        dispatch!(self, create_principal, input)
    }

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

    pub async fn get_agent_trigger_by_ingress_id_unscoped(
        &self,
        ingress_id: &str,
    ) -> Result<Option<AgentTriggerRow>> {
        dispatch!(self, get_agent_trigger_by_ingress_id_unscoped, ingress_id)
    }

    pub async fn list_principals_by_resolved_user(
        &self,
        org_id: i64,
        user_id: Uuid,
    ) -> Result<Vec<PrincipalRow>> {
        dispatch!(self, list_principals_by_resolved_user, org_id, user_id)
    }

    pub async fn update_principal(
        &self,
        org_id: i64,
        id: PrincipalId,
        input: UpdatePrincipalRow,
    ) -> Result<Option<PrincipalRow>> {
        dispatch!(self, update_principal, org_id, id, input)
    }

    // ============================================
    // Personal Access Tokens
    // ============================================

    pub async fn create_personal_access_token(
        &self,
        input: CreatePersonalAccessTokenRow,
    ) -> Result<PersonalAccessTokenRow> {
        dispatch!(self, create_personal_access_token, input)
    }

    pub async fn get_personal_access_token_by_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<PersonalAccessTokenRow>> {
        dispatch!(self, get_personal_access_token_by_hash, token_hash)
    }

    pub async fn list_personal_access_tokens_for_user(
        &self,
        user_id: Uuid,
    ) -> Result<Vec<PersonalAccessTokenRow>> {
        dispatch!(self, list_personal_access_tokens_for_user, user_id)
    }

    pub async fn update_personal_access_token_last_used(&self, id: Uuid) -> Result<()> {
        dispatch!(self, update_personal_access_token_last_used, id)
    }

    pub async fn delete_personal_access_token(&self, id: Uuid, user_id: Uuid) -> Result<bool> {
        dispatch!(self, delete_personal_access_token, id, user_id)
    }

    // ============================================
    // CLI Auth Sessions
    // ============================================

    pub async fn create_cli_auth_session(
        &self,
        input: CreateCliAuthSessionRow,
    ) -> Result<CliAuthSessionRow> {
        dispatch!(self, create_cli_auth_session, input)
    }

    pub async fn get_cli_auth_session_by_state(
        &self,
        state: &str,
    ) -> Result<Option<CliAuthSessionRow>> {
        dispatch!(self, get_cli_auth_session_by_state, state)
    }

    pub async fn get_cli_auth_session_by_exchange_code(
        &self,
        code: &str,
    ) -> Result<Option<CliAuthSessionRow>> {
        dispatch!(self, get_cli_auth_session_by_exchange_code, code)
    }

    pub async fn complete_cli_auth_session(&self, id: Uuid, user_id: Uuid) -> Result<bool> {
        dispatch!(self, complete_cli_auth_session, id, user_id)
    }

    pub async fn delete_expired_cli_auth_sessions(&self) -> Result<u64> {
        dispatch!(self, delete_expired_cli_auth_sessions)
    }

    pub async fn delete_cli_auth_session(&self, id: Uuid) -> Result<bool> {
        dispatch!(self, delete_cli_auth_session, id)
    }

    // ============================================
    // Refresh Tokens
    // ============================================

    pub async fn create_refresh_token(
        &self,
        input: CreateRefreshTokenRow,
    ) -> Result<RefreshTokenRow> {
        dispatch!(self, create_refresh_token, input)
    }

    pub async fn get_refresh_token_by_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<RefreshTokenRow>> {
        dispatch!(self, get_refresh_token_by_hash, token_hash)
    }

    pub async fn delete_refresh_token(&self, id: Uuid) -> Result<bool> {
        dispatch!(self, delete_refresh_token, id)
    }

    /// EVE-454: atomic single-use refresh-token consume. See
    /// `repositories::auth::AuthRepository::consume_refresh_token_by_hash`.
    pub async fn consume_refresh_token_by_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<RefreshTokenRow>> {
        dispatch!(self, consume_refresh_token_by_hash, token_hash)
    }

    pub async fn delete_expired_refresh_tokens(&self) -> Result<u64> {
        dispatch!(self, delete_expired_refresh_tokens)
    }

    pub async fn delete_user_refresh_tokens(&self, user_id: Uuid) -> Result<u64> {
        dispatch!(self, delete_user_refresh_tokens, user_id)
    }

    // ============================================
    // Password Reset / Email Verification Tokens
    // ============================================
    // Hashed, single-use, short-TTL tokens for native-auth account recovery.
    // The raw token is emailed once and never stored; only its SHA-256 hash is
    // persisted. `consume_*` is race-safe (single atomic UPDATE) like
    // `consume_refresh_token_by_hash`.

    pub async fn create_password_reset_token(
        &self,
        user_id: Uuid,
        token_hash: &str,
        expires_at: DateTime<Utc>,
    ) -> Result<()> {
        dispatch!(
            self,
            create_password_reset_token,
            user_id,
            token_hash,
            expires_at
        )
    }

    /// Atomically claim a password reset token. Returns the owning `user_id`
    /// only if a matching, unexpired, not-yet-used token exists; otherwise None.
    pub async fn consume_password_reset_token(&self, token_hash: &str) -> Result<Option<Uuid>> {
        dispatch!(self, consume_password_reset_token, token_hash)
    }

    pub async fn create_email_verification_token(
        &self,
        user_id: Uuid,
        token_hash: &str,
        expires_at: DateTime<Utc>,
    ) -> Result<()> {
        dispatch!(
            self,
            create_email_verification_token,
            user_id,
            token_hash,
            expires_at
        )
    }

    /// Atomically claim an email verification token. Returns the owning
    /// `user_id` only if a matching, unexpired, not-yet-used token exists.
    pub async fn consume_email_verification_token(&self, token_hash: &str) -> Result<Option<Uuid>> {
        dispatch!(self, consume_email_verification_token, token_hash)
    }

    // ============================================
    // OAuth Clients (MCP OAuth 2.1)
    // ============================================

    pub async fn create_oauth_client(&self, input: CreateOAuthClientRow) -> Result<OAuthClientRow> {
        dispatch!(self, create_oauth_client, input)
    }

    pub async fn get_oauth_client_by_client_id(
        &self,
        client_id: &str,
    ) -> Result<Option<OAuthClientRow>> {
        dispatch!(self, get_oauth_client_by_client_id, client_id)
    }

    // ============================================
    // OAuth Authorization Codes
    // ============================================

    pub async fn create_oauth_authorization_code(
        &self,
        input: CreateOAuthAuthorizationCodeRow,
    ) -> Result<OAuthAuthorizationCodeRow> {
        dispatch!(self, create_oauth_authorization_code, input)
    }

    pub async fn get_oauth_authorization_code_by_hash(
        &self,
        code_hash: &str,
    ) -> Result<Option<OAuthAuthorizationCodeRow>> {
        dispatch!(self, get_oauth_authorization_code_by_hash, code_hash)
    }

    pub async fn consume_oauth_authorization_code(&self, id: Uuid) -> Result<bool> {
        dispatch!(self, consume_oauth_authorization_code, id)
    }

    pub async fn delete_expired_oauth_authorization_codes(&self) -> Result<u64> {
        dispatch!(self, delete_expired_oauth_authorization_codes)
    }

    // ============================================
    // OAuth Refresh Tokens
    // ============================================

    pub async fn create_oauth_refresh_token(
        &self,
        input: CreateOAuthRefreshTokenRow,
    ) -> Result<OAuthRefreshTokenRow> {
        dispatch!(self, create_oauth_refresh_token, input)
    }

    pub async fn get_oauth_refresh_token_by_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<OAuthRefreshTokenRow>> {
        dispatch!(self, get_oauth_refresh_token_by_hash, token_hash)
    }

    pub async fn consume_oauth_refresh_token_by_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<OAuthRefreshTokenRow>> {
        dispatch!(self, consume_oauth_refresh_token_by_hash, token_hash)
    }

    pub async fn delete_oauth_refresh_token(&self, id: Uuid) -> Result<bool> {
        dispatch!(self, delete_oauth_refresh_token, id)
    }

    pub async fn delete_expired_oauth_refresh_tokens(&self) -> Result<u64> {
        dispatch!(self, delete_expired_oauth_refresh_tokens)
    }

    // ============================================
    // Agents
    // ============================================

    pub async fn create_agent(&self, org_id: i64, input: CreateAgentRow) -> Result<AgentRow> {
        dispatch!(self, create_agent, org_id, input)
    }

    pub async fn create_agent_with_id(
        &self,
        org_id: i64,
        id: AgentId,
        input: CreateAgentRow,
    ) -> Result<Option<AgentRow>> {
        dispatch!(self, create_agent_with_id, org_id, id, input)
    }

    pub async fn get_agent(&self, org_id: i64, id: AgentId) -> Result<Option<AgentRow>> {
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

    pub async fn list_agents(
        &self,
        org_id: i64,
        search: Option<&str>,
        include_archived: bool,
        pagination: Pagination,
    ) -> Result<(Vec<AgentRow>, u32)> {
        dispatch!(
            self,
            list_agents,
            org_id,
            search,
            include_archived,
            pagination
        )
    }

    pub async fn count_sessions_for_agent(&self, org_id: i64, agent_id: AgentId) -> Result<u64> {
        dispatch!(self, count_sessions_for_agent, org_id, agent_id)
    }

    pub async fn get_agent_by_name(&self, org_id: i64, name: &str) -> Result<Option<AgentRow>> {
        dispatch!(self, get_agent_by_name, org_id, name)
    }

    pub async fn update_agent(
        &self,
        org_id: i64,
        id: AgentId,
        input: UpdateAgent,
    ) -> Result<Option<AgentRow>> {
        dispatch!(self, update_agent, org_id, id, input)
    }

    pub async fn set_agent_identity_id(
        &self,
        org_id: i64,
        id: AgentId,
        agent_identity_id: AgentIdentityId,
    ) -> Result<bool> {
        dispatch!(self, set_agent_identity_id, org_id, id, agent_identity_id)
    }

    pub async fn has_agent_with_identity(
        &self,
        org_id: i64,
        agent_identity_id: AgentIdentityId,
    ) -> Result<bool> {
        dispatch!(self, has_agent_with_identity, org_id, agent_identity_id)
    }

    pub async fn delete_agent(&self, org_id: i64, id: AgentId) -> Result<bool> {
        dispatch!(self, delete_agent, org_id, id)
    }

    pub async fn destroy_agent(&self, org_id: i64, id: AgentId) -> Result<bool> {
        dispatch!(self, destroy_agent, org_id, id)
    }

    pub async fn upsert_agent(
        &self,
        org_id: i64,
        input: CreateAgentRow,
    ) -> Result<(AgentRow, bool)> {
        dispatch!(self, upsert_agent, org_id, input)
    }

    pub async fn upsert_agent_by_name(
        &self,
        org_id: i64,
        input: CreateAgentRow,
    ) -> Result<(AgentRow, bool)> {
        dispatch!(self, upsert_agent_by_name, org_id, input)
    }

    pub async fn get_agent_public_id(&self, org_id: i64, id: AgentId) -> Result<Option<String>> {
        #[cfg(test)]
        self.record_session_list_lookup().await;
        dispatch!(self, get_agent_public_id, org_id, id)
    }

    pub async fn create_agent_version(
        &self,
        input: CreateAgentVersionRow,
    ) -> Result<AgentVersionRow> {
        dispatch!(self, create_agent_version, input)
    }

    pub async fn list_agent_versions(
        &self,
        org_id: i64,
        agent_id: AgentId,
    ) -> Result<Vec<AgentVersionRow>> {
        dispatch!(self, list_agent_versions, org_id, agent_id)
    }

    pub async fn get_agent_version(
        &self,
        org_id: i64,
        id: everruns_provider::typed_id::AgentVersionId,
    ) -> Result<Option<AgentVersionRow>> {
        dispatch!(self, get_agent_version, org_id, id)
    }

    pub async fn get_latest_agent_version(
        &self,
        org_id: i64,
        agent_id: AgentId,
    ) -> Result<Option<AgentVersionRow>> {
        dispatch!(self, get_latest_agent_version, org_id, agent_id)
    }

    pub async fn get_latest_agent_snapshot(
        &self,
        org_id: i64,
        agent_id: AgentId,
    ) -> Result<Option<AgentVersionRow>> {
        dispatch!(self, get_latest_agent_snapshot, org_id, agent_id)
    }

    pub async fn prune_agent_auto_snapshots(
        &self,
        org_id: i64,
        agent_id: AgentId,
        keep: i64,
    ) -> Result<u64> {
        dispatch!(self, prune_agent_auto_snapshots, org_id, agent_id, keep)
    }

    pub async fn upsert_agent_mcp_secret_binding(
        &self,
        input: UpsertAgentMcpSecretBindingRow,
    ) -> Result<AgentMcpSecretBindingRow> {
        dispatch!(self, upsert_agent_mcp_secret_binding, input)
    }

    pub async fn list_agent_mcp_secret_bindings(
        &self,
        org_id: i64,
        agent_id: AgentId,
    ) -> Result<Vec<AgentMcpSecretBindingRow>> {
        dispatch!(self, list_agent_mcp_secret_bindings, org_id, agent_id)
    }

    pub async fn get_agent_mcp_secret_binding(
        &self,
        org_id: i64,
        agent_id: AgentId,
        binding_id: Uuid,
    ) -> Result<Option<AgentMcpSecretBindingRow>> {
        dispatch!(
            self,
            get_agent_mcp_secret_binding,
            org_id,
            agent_id,
            binding_id
        )
    }

    pub async fn set_agent_mcp_secret_binding_value(
        &self,
        org_id: i64,
        agent_id: AgentId,
        binding_id: Uuid,
        value_encrypted: Vec<u8>,
    ) -> Result<Option<AgentMcpSecretBindingRow>> {
        dispatch!(
            self,
            set_agent_mcp_secret_binding_value,
            org_id,
            agent_id,
            binding_id,
            value_encrypted
        )
    }

    pub async fn delete_agent_mcp_secret_binding(
        &self,
        org_id: i64,
        agent_id: AgentId,
        binding_id: Uuid,
    ) -> Result<bool> {
        dispatch!(
            self,
            delete_agent_mcp_secret_binding,
            org_id,
            agent_id,
            binding_id
        )
    }

    // ============================================
    // Harnesses
    // ============================================
}

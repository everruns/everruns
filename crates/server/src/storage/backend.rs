// Storage backend abstraction
// Decision: Use enum dispatch for simplicity over trait objects
//
// This module provides a unified StorageBackend enum that can work with
// either PostgreSQL (production) or in-memory (dev mode) storage.

use anyhow::Result;
use chrono::{DateTime, Utc};
use everruns_core::message_filter::MessageQuery;
use everruns_provider::typed_id::{
    AgentId, AgentIdentityId, EventId, HarnessId, KnowledgeBaseId, KnowledgeEntryId,
    KnowledgeIndexId, LeasedResourceId, MemoryId, MessageId, NotificationId, PrincipalId,
    ScheduleId, SessionId, SessionParticipantId, TriggerId, WorkspaceId,
};
use sqlx::PgPool;
use uuid::Uuid;

pub const USER_PREFERENCE_LIMIT_EXCEEDED: &str = "user preference limit exceeded";

use super::memory::InMemoryDatabase;
use super::models::*;
use super::reporting::models::ReportingOutboxRow;
use super::repositories::Database;
use crate::api::common::Pagination;

/// Hard upper bound on a single retention-prune batch (EVE-580). Caps the
/// destructive `prune_terminal_session_tasks_with_artifacts` regardless of
/// caller input so a misconfigured limit can never request an unbounded or
/// oversized delete; large backlogs drain over successive reaper ticks.
const MAX_RETENTION_PRUNE_LIMIT: i64 = 1000;

/// Hard cap for participant history returned in one session response.
/// Storage queries fetch one extra row so callers can reject oversized histories
/// instead of allocating or serializing unbounded attacker-created rows.
pub const MAX_SESSION_PARTICIPANT_HISTORY: usize = 512;

const TASK_ARTIFACT_ROOTS: &[&str] = &["/.tasks", "/.background", "/.agent-runs"];

fn task_artifact_delete_root(result_path: &str) -> Option<&str> {
    if !result_path.starts_with('/') || result_path.contains("..") || result_path.contains("//") {
        return None;
    }

    let mut parts = result_path.split('/');
    if parts.next() != Some("") {
        return None;
    }
    let root_name = parts.next()?;
    let run_id = parts.next()?;

    if root_name.is_empty() || run_id.is_empty() || parts.next().is_none() {
        return None;
    }

    let root = match root_name {
        ".tasks" => "/.tasks",
        ".background" => "/.background",
        ".agent-runs" => "/.agent-runs",
        _ => return None,
    };
    if !TASK_ARTIFACT_ROOTS.contains(&root) {
        return None;
    }

    Some(&result_path[..root.len() + 1 + run_id.len()])
}

/// Helper macro to dispatch method calls to the appropriate backend.
///
/// This reduces the repetitive match pattern from 4 lines to 1 line per method.
///
/// # Usage
///
/// ```ignore
/// pub async fn method_name(&self, arg1: T1, arg2: T2) -> Result<R> {
///     dispatch!(self, method_name, arg1, arg2)
/// }
/// ```
macro_rules! dispatch {
    ($self:ident, $method:ident $(, $arg:expr)*) => {
        match $self {
            Self::Postgres(db) => db.$method($($arg),*).await,
            Self::InMemory(db) => db.$method($($arg),*).await,
        }
    };
}

/// Storage backend that can be either PostgreSQL or in-memory
#[derive(Clone)]
pub enum StorageBackend {
    /// PostgreSQL database (production)
    Postgres(Database),
    /// In-memory database (dev mode)
    InMemory(std::sync::Arc<InMemoryDatabase>),
}

impl StorageBackend {
    #[cfg(test)]
    async fn record_session_list_lookup(&self) {
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

    pub async fn create_harness(&self, org_id: i64, input: CreateHarnessRow) -> Result<HarnessRow> {
        dispatch!(self, create_harness, org_id, input)
    }

    pub async fn create_harness_with_id(
        &self,
        org_id: i64,
        id: HarnessId,
        input: CreateHarnessRow,
    ) -> Result<Option<HarnessRow>> {
        dispatch!(self, create_harness_with_id, org_id, id, input)
    }

    pub async fn get_harness(&self, org_id: i64, id: HarnessId) -> Result<Option<HarnessRow>> {
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

    pub async fn get_harness_by_name(&self, org_id: i64, name: &str) -> Result<Option<HarnessRow>> {
        dispatch!(self, get_harness_by_name, org_id, name)
    }

    pub async fn list_harnesses(
        &self,
        org_id: i64,
        search: Option<&str>,
        include_archived: bool,
    ) -> Result<Vec<HarnessRow>> {
        dispatch!(self, list_harnesses, org_id, search, include_archived)
    }

    pub async fn count_sessions_for_harness(
        &self,
        org_id: i64,
        harness_id: HarnessId,
    ) -> Result<u64> {
        dispatch!(self, count_sessions_for_harness, org_id, harness_id)
    }

    pub async fn count_sessions_for_harnesses(
        &self,
        org_id: i64,
        harness_ids: &[HarnessId],
    ) -> Result<Vec<(HarnessId, i64)>> {
        dispatch!(self, count_sessions_for_harnesses, org_id, harness_ids)
    }

    /// Count user-created harnesses in an org (for resource limits); excludes
    /// soft-deleted rows and system-seeded built-in harnesses.
    pub async fn count_harnesses_for_org(&self, org_id: i64) -> Result<i64> {
        dispatch!(self, count_harnesses_for_org, org_id)
    }

    /// Count non-deleted, non-built-in agents in an org (for resource limits).
    pub async fn count_agents_for_org(&self, org_id: i64) -> Result<i64> {
        dispatch!(self, count_agents_for_org, org_id)
    }

    /// Flag an agent as platform-supplied.
    ///
    /// Reserved for org bootstrap reconciliation, which must be able to adopt a
    /// row that already exists — an org seeded before built-in agents shipped
    /// has the agent but not the flag. No command path calls this.
    pub async fn mark_agent_built_in(&self, org_id: i64, id: AgentId) -> Result<()> {
        dispatch!(self, mark_agent_built_in, org_id, id)
    }

    /// Count sessions in an org (for resource limits).
    pub async fn count_sessions_for_org(&self, org_id: i64) -> Result<i64> {
        dispatch!(self, count_sessions_for_org, org_id)
    }

    pub async fn update_harness(
        &self,
        org_id: i64,
        id: HarnessId,
        input: UpdateHarness,
    ) -> Result<Option<HarnessRow>> {
        dispatch!(self, update_harness, org_id, id, input)
    }

    pub async fn list_child_harnesses(
        &self,
        org_id: i64,
        parent_id: HarnessId,
    ) -> Result<Vec<HarnessRow>> {
        dispatch!(self, list_child_harnesses, org_id, parent_id)
    }

    pub async fn release_built_in_harness(&self, org_id: i64, name: &str) -> Result<bool> {
        dispatch!(self, release_built_in_harness, org_id, name)
    }

    pub async fn delete_harness(&self, org_id: i64, id: HarnessId) -> Result<bool> {
        dispatch!(self, delete_harness, org_id, id)
    }

    pub async fn destroy_harness(&self, org_id: i64, id: HarnessId) -> Result<bool> {
        dispatch!(self, destroy_harness, org_id, id)
    }

    // ============================================
    // Agent identities
    // ============================================

    pub async fn create_agent_identity(
        &self,
        input: CreateAgentIdentityRow,
    ) -> Result<AgentIdentityRow> {
        dispatch!(self, create_agent_identity, input)
    }

    pub async fn get_agent_identity(
        &self,
        org_id: i64,
        id: AgentIdentityId,
    ) -> Result<Option<AgentIdentityRow>> {
        dispatch!(self, get_agent_identity, org_id, id)
    }

    pub async fn list_agent_identities(
        &self,
        org_id: i64,
        search: Option<&str>,
        include_archived: bool,
    ) -> Result<Vec<AgentIdentityRow>> {
        dispatch!(
            self,
            list_agent_identities,
            org_id,
            search,
            include_archived
        )
    }

    pub async fn update_agent_identity(
        &self,
        org_id: i64,
        id: AgentIdentityId,
        input: UpdateAgentIdentity,
    ) -> Result<Option<AgentIdentityRow>> {
        dispatch!(self, update_agent_identity, org_id, id, input)
    }

    pub async fn delete_agent_identity(&self, org_id: i64, id: AgentIdentityId) -> Result<bool> {
        dispatch!(self, delete_agent_identity, org_id, id)
    }

    pub async fn destroy_agent_identity(&self, org_id: i64, id: AgentIdentityId) -> Result<bool> {
        dispatch!(self, destroy_agent_identity, org_id, id)
    }

    // ============================================
    // Agent triggers
    // ============================================

    pub async fn create_agent_trigger(
        &self,
        input: CreateAgentTriggerRow,
    ) -> Result<AgentTriggerRow> {
        dispatch!(self, create_agent_trigger, input)
    }

    pub async fn get_agent_trigger(
        &self,
        org_id: i64,
        id: TriggerId,
    ) -> Result<Option<AgentTriggerRow>> {
        dispatch!(self, get_agent_trigger, org_id, id)
    }

    pub async fn list_agent_triggers(
        &self,
        org_id: i64,
        agent_id: Option<AgentId>,
        include_archived: bool,
    ) -> Result<Vec<AgentTriggerRow>> {
        dispatch!(
            self,
            list_agent_triggers,
            org_id,
            agent_id,
            include_archived
        )
    }

    pub async fn update_agent_trigger(
        &self,
        org_id: i64,
        id: TriggerId,
        input: UpdateAgentTrigger,
    ) -> Result<Option<AgentTriggerRow>> {
        dispatch!(self, update_agent_trigger, org_id, id, input)
    }

    pub async fn set_agent_trigger_durable_schedule_id(
        &self,
        org_id: i64,
        id: TriggerId,
        durable_schedule_id: Option<Uuid>,
    ) -> Result<Option<AgentTriggerRow>> {
        dispatch!(
            self,
            set_agent_trigger_durable_schedule_id,
            org_id,
            id,
            durable_schedule_id
        )
    }

    pub async fn delete_agent_trigger(&self, org_id: i64, id: TriggerId) -> Result<bool> {
        dispatch!(self, delete_agent_trigger, org_id, id)
    }

    // ============================================
    // Sessions
    // ============================================

    pub async fn create_session(&self, input: CreateSessionRow) -> Result<SessionRow> {
        dispatch!(self, create_session, input)
    }

    pub async fn create_session_participant(
        &self,
        input: CreateSessionParticipantRow,
    ) -> Result<SessionParticipantRow> {
        dispatch!(self, create_session_participant, input)
    }

    pub async fn ensure_active_user_session_participant(
        &self,
        input: CreateSessionParticipantRow,
    ) -> Result<SessionParticipantRow> {
        dispatch!(self, ensure_active_user_session_participant, input)
    }

    pub async fn list_session_participants(
        &self,
        org_id: i64,
        session_id: SessionId,
    ) -> Result<Vec<SessionParticipantRow>> {
        dispatch!(self, list_session_participants, org_id, session_id)
    }

    pub async fn leave_session_participant(
        &self,
        org_id: i64,
        session_id: SessionId,
        participant_id: SessionParticipantId,
    ) -> Result<Option<SessionParticipantRow>> {
        dispatch!(
            self,
            leave_session_participant,
            org_id,
            session_id,
            participant_id
        )
    }

    pub async fn list_reporting_outbox(
        &self,
        org_id: i64,
        source_type: &str,
        source_id: &str,
        reason: &str,
    ) -> Result<Vec<ReportingOutboxRow>> {
        dispatch!(
            self,
            list_reporting_outbox,
            org_id,
            source_type,
            source_id,
            reason
        )
    }

    /// Record fork provenance on an already-created session
    /// (knowledge/runtime-resources/forking-sessions.md).
    pub async fn set_session_fork_lineage(
        &self,
        session_id: SessionId,
        forked_from_session_id: SessionId,
        forked_from_sequence: Option<i32>,
    ) -> Result<()> {
        dispatch!(
            self,
            set_session_fork_lineage,
            session_id,
            forked_from_session_id,
            forked_from_sequence
        )
    }

    pub async fn get_session(&self, org_id: i64, id: SessionId) -> Result<Option<SessionRow>> {
        dispatch!(self, get_session, org_id, id)
    }

    /// Get session without org scoping. For internal system use only (e.g. usage tracking).
    pub async fn get_session_unscoped(&self, id: SessionId) -> Result<Option<SessionRow>> {
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

    /// Facet-rail counts and masthead metrics over the same predicate as
    /// [`Self::list_sessions`] (EVE-852).
    pub async fn session_facets(
        &self,
        org_id: i64,
        filters: &SessionListFilters,
    ) -> Result<SessionFacetsRow> {
        dispatch!(self, session_facets, org_id, filters)
    }

    /// List child sessions (subagents) for a parent session.
    pub async fn list_child_sessions(
        &self,
        parent_session_id: SessionId,
    ) -> Result<Vec<SessionRow>> {
        dispatch!(self, list_child_sessions, parent_session_id)
    }

    /// Count sessions grouped by status for an organization.
    pub async fn count_sessions_by_status(&self, org_id: i64) -> Result<Vec<(String, i64)>> {
        dispatch!(self, count_sessions_by_status, org_id)
    }

    pub async fn count_active_sessions_for_org(&self, org_id: i64) -> Result<i64> {
        dispatch!(self, count_active_sessions_for_org, org_id)
    }

    pub async fn count_active_turns_for_org(&self, org_id: i64) -> Result<i64> {
        dispatch!(self, count_active_turns_for_org, org_id)
    }

    pub async fn reserve_active_turn_slot_for_org(
        &self,
        org_id: i64,
        session_id: SessionId,
        max_active_turns: i64,
    ) -> Result<ReserveActiveTurnSlotResult> {
        dispatch!(
            self,
            reserve_active_turn_slot_for_org,
            org_id,
            session_id,
            max_active_turns
        )
    }

    /// Release a previously reserved active-turn slot, restoring the session's
    /// status captured at reservation time (best-effort; only reverts a session
    /// still `active`).
    pub async fn release_active_turn_slot_for_org(
        &self,
        org_id: i64,
        session_id: SessionId,
        previous_status: &str,
    ) -> Result<()> {
        dispatch!(
            self,
            release_active_turn_slot_for_org,
            org_id,
            session_id,
            previous_status
        )
    }

    /// Aggregate session and execution stats for an optional agent or harness scope.
    pub async fn session_aggregate_stats(
        &self,
        org_id: i64,
        agent_id: Option<AgentId>,
        harness_id: Option<HarnessId>,
    ) -> Result<SessionAggregateStatsRow> {
        dispatch!(self, session_aggregate_stats, org_id, agent_id, harness_id)
    }

    /// Find active sessions with Slack tags (for startup recovery).
    pub async fn find_active_slack_sessions(&self) -> Result<Vec<SessionRow>> {
        dispatch!(self, find_active_slack_sessions)
    }

    /// Find sessions in `waiting_for_tool_results` with updated_at before the
    /// given cutoff. Returns `(session_id, org_id)` pairs.
    pub async fn list_sessions_waiting_tool_results_before(
        &self,
        cutoff: chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<(SessionId, i64)>> {
        dispatch!(self, list_sessions_waiting_tool_results_before, cutoff)
    }

    /// Find a single session matching ALL given tags within an org.
    pub async fn find_session_by_tags(
        &self,
        org_id: i64,
        tags: &[String],
    ) -> Result<Option<SessionRow>> {
        dispatch!(self, find_session_by_tags, org_id, tags)
    }

    /// Find a single app-owned session matching ALL given tags within an org.
    pub async fn find_app_session_by_tags(
        &self,
        org_id: i64,
        app_id: Uuid,
        tags: &[String],
    ) -> Result<Option<SessionRow>> {
        dispatch!(self, find_app_session_by_tags, org_id, app_id, tags)
    }

    /// Find a single session matching ALL given tags + owner within an org.
    pub async fn find_session_by_tags_and_owner(
        &self,
        org_id: i64,
        owner_principal_id: PrincipalId,
        tags: &[String],
    ) -> Result<Option<SessionRow>> {
        dispatch!(
            self,
            find_session_by_tags_and_owner,
            org_id,
            owner_principal_id,
            tags
        )
    }

    /// Find a single app-owned session matching ALL given tags + owner within an org.
    pub async fn find_app_session_by_tags_and_owner(
        &self,
        org_id: i64,
        app_id: Uuid,
        owner_principal_id: PrincipalId,
        tags: &[String],
    ) -> Result<Option<SessionRow>> {
        dispatch!(
            self,
            find_app_session_by_tags_and_owner,
            org_id,
            app_id,
            owner_principal_id,
            tags
        )
    }

    pub async fn update_session(
        &self,
        org_id: i64,
        id: SessionId,
        input: UpdateSession,
    ) -> Result<Option<SessionRow>> {
        dispatch!(self, update_session, org_id, id, input)
    }

    /// Store a generated run summary, fenced on the terminal turn it describes
    /// so a late out-of-band write cannot overwrite a newer one (EVE-867).
    pub async fn set_session_run_summary(
        &self,
        org_id: i64,
        id: SessionId,
        summary: &str,
        turn_sequence: i64,
    ) -> Result<bool> {
        dispatch!(
            self,
            set_session_run_summary,
            org_id,
            id,
            summary,
            turn_sequence
        )
    }

    pub async fn set_session_archived(
        &self,
        org_id: i64,
        id: SessionId,
        archived: bool,
    ) -> Result<Option<SessionRow>> {
        dispatch!(self, set_session_archived, org_id, id, archived)
    }

    pub async fn delete_session(&self, org_id: i64, id: SessionId) -> Result<bool> {
        dispatch!(self, delete_session, org_id, id)
    }

    // ============================================
    // Workspaces (see knowledge/runtime-resources/workspace.md)
    // ============================================

    pub async fn create_workspace(
        &self,
        org_id: i64,
        input: CreateWorkspaceRow,
    ) -> Result<WorkspaceRow> {
        dispatch!(self, create_workspace, org_id, input)
    }

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

    pub async fn get_workspace_by_id(&self, org_id: i64, id: Uuid) -> Result<Option<WorkspaceRow>> {
        dispatch!(self, get_workspace_by_id, org_id, id)
    }

    pub async fn get_workspace_organization_id(&self, public_id: &str) -> Result<Option<i64>> {
        dispatch!(self, get_workspace_organization_id, public_id)
    }

    pub async fn list_workspaces(
        &self,
        org_id: i64,
        search: Option<&str>,
        include_archived: bool,
    ) -> Result<Vec<WorkspaceRow>> {
        dispatch!(self, list_workspaces, org_id, search, include_archived)
    }

    pub async fn update_workspace(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateWorkspace,
    ) -> Result<Option<WorkspaceRow>> {
        dispatch!(self, update_workspace, org_id, id, input)
    }

    pub async fn archive_workspace(&self, org_id: i64, id: Uuid) -> Result<bool> {
        dispatch!(self, archive_workspace, org_id, id)
    }

    // ============================================
    // Memories
    // ============================================

    pub async fn create_memory(&self, org_id: i64, input: CreateMemoryRow) -> Result<MemoryRow> {
        dispatch!(self, create_memory, org_id, input)
    }

    pub async fn get_memory(&self, org_id: i64, memory_id: MemoryId) -> Result<Option<MemoryRow>> {
        dispatch!(
            self,
            get_memory_by_public_id,
            org_id,
            &memory_id.to_string()
        )
    }

    pub async fn get_memory_by_id(&self, org_id: i64, id: Uuid) -> Result<Option<MemoryRow>> {
        dispatch!(self, get_memory_by_id, org_id, id)
    }

    pub async fn get_memory_by_scope_owner(
        &self,
        org_id: i64,
        scope: &str,
        owner_agent_id: Option<AgentId>,
        owner_user_id: Option<Uuid>,
    ) -> Result<Option<MemoryRow>> {
        dispatch!(
            self,
            get_memory_by_scope_owner,
            org_id,
            scope,
            owner_agent_id,
            owner_user_id
        )
    }

    pub async fn list_memories(
        &self,
        org_id: i64,
        search: Option<&str>,
        include_archived: bool,
    ) -> Result<Vec<MemoryRow>> {
        dispatch!(self, list_memories, org_id, search, include_archived)
    }

    pub async fn update_memory(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateMemory,
    ) -> Result<Option<MemoryRow>> {
        dispatch!(self, update_memory, org_id, id, input)
    }

    pub async fn archive_memory(&self, org_id: i64, id: Uuid) -> Result<bool> {
        dispatch!(self, archive_memory, org_id, id)
    }

    pub async fn claim_next_memory_sync(&self) -> Result<Option<MemoryRow>> {
        dispatch!(self, claim_next_memory_sync)
    }

    pub async fn complete_memory_sync(
        &self,
        memory_id: Uuid,
        claimed_at: DateTime<Utc>,
        files: Vec<CreateMemoryFileRow>,
    ) -> Result<Option<MemoryRow>> {
        dispatch!(self, complete_memory_sync, memory_id, claimed_at, files)
    }

    pub async fn fail_memory_sync(
        &self,
        memory_id: Uuid,
        claimed_at: DateTime<Utc>,
        error: &str,
    ) -> Result<Option<MemoryRow>> {
        dispatch!(self, fail_memory_sync, memory_id, claimed_at, error)
    }

    pub async fn list_all_memory_files(&self, memory_id: Uuid) -> Result<Vec<MemoryFileRow>> {
        dispatch!(self, list_all_memory_files, memory_id)
    }

    pub async fn create_memory_file(
        &self,
        memory_id: Uuid,
        input: CreateMemoryFileRow,
    ) -> Result<MemoryFileRow> {
        dispatch!(self, create_memory_file, memory_id, input)
    }

    pub async fn get_memory_file(
        &self,
        memory_id: Uuid,
        path: &str,
    ) -> Result<Option<MemoryFileRow>> {
        dispatch!(self, get_memory_file, memory_id, path)
    }

    pub async fn get_memory_file_info(
        &self,
        memory_id: Uuid,
        path: &str,
    ) -> Result<Option<MemoryFileInfoRow>> {
        dispatch!(self, get_memory_file_info, memory_id, path)
    }

    pub async fn list_memory_files(
        &self,
        memory_id: Uuid,
        parent_path: &str,
    ) -> Result<Vec<MemoryFileInfoRow>> {
        dispatch!(self, list_memory_files, memory_id, parent_path)
    }

    pub async fn update_memory_file(
        &self,
        memory_id: Uuid,
        path: &str,
        input: UpdateMemoryFile,
    ) -> Result<Option<MemoryFileRow>> {
        dispatch!(self, update_memory_file, memory_id, path, input)
    }

    pub async fn delete_memory_file(&self, memory_id: Uuid, path: &str) -> Result<bool> {
        dispatch!(self, delete_memory_file, memory_id, path)
    }

    pub async fn delete_memory_file_recursive(&self, memory_id: Uuid, path: &str) -> Result<u64> {
        dispatch!(self, delete_memory_file_recursive, memory_id, path)
    }

    pub async fn grep_memory_files(
        &self,
        memory_id: Uuid,
        pattern: &str,
        path_pattern: Option<&str>,
        max_file_bytes: i64,
    ) -> Result<Vec<MemoryFileInfoRow>> {
        dispatch!(
            self,
            grep_memory_files,
            memory_id,
            pattern,
            path_pattern,
            max_file_bytes
        )
    }

    pub async fn memory_file_exists(&self, memory_id: Uuid, path: &str) -> Result<bool> {
        dispatch!(self, memory_file_exists, memory_id, path)
    }

    pub async fn memory_directory_has_children(&self, memory_id: Uuid, path: &str) -> Result<bool> {
        dispatch!(self, memory_directory_has_children, memory_id, path)
    }

    // ============================================
    // Knowledge Bases
    // ============================================

    pub async fn create_knowledge_base(
        &self,
        org_id: i64,
        input: CreateKnowledgeBaseRow,
    ) -> Result<KnowledgeBaseRow> {
        dispatch!(self, create_knowledge_base, org_id, input)
    }

    pub async fn get_knowledge_base(
        &self,
        org_id: i64,
        kb_id: KnowledgeBaseId,
    ) -> Result<Option<KnowledgeBaseRow>> {
        dispatch!(
            self,
            get_knowledge_base_by_public_id,
            org_id,
            &kb_id.to_string()
        )
    }

    pub async fn get_knowledge_base_by_id(
        &self,
        org_id: i64,
        id: Uuid,
    ) -> Result<Option<KnowledgeBaseRow>> {
        dispatch!(self, get_knowledge_base_by_id, org_id, id)
    }

    pub async fn list_knowledge_bases(
        &self,
        org_id: i64,
        search: Option<&str>,
        include_archived: bool,
    ) -> Result<Vec<KnowledgeBaseRow>> {
        dispatch!(self, list_knowledge_bases, org_id, search, include_archived)
    }

    pub async fn update_knowledge_base(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateKnowledgeBase,
    ) -> Result<Option<KnowledgeBaseRow>> {
        dispatch!(self, update_knowledge_base, org_id, id, input)
    }

    pub async fn archive_knowledge_base(&self, org_id: i64, id: Uuid) -> Result<bool> {
        dispatch!(self, archive_knowledge_base, org_id, id)
    }

    pub async fn create_knowledge_entry(
        &self,
        kb_id: Uuid,
        input: CreateKnowledgeEntryRow,
    ) -> Result<KnowledgeEntryRow> {
        dispatch!(self, create_knowledge_entry, kb_id, input)
    }

    pub async fn get_knowledge_entry(
        &self,
        kb_id: Uuid,
        entry_id: KnowledgeEntryId,
    ) -> Result<Option<KnowledgeEntryRow>> {
        dispatch!(
            self,
            get_knowledge_entry_by_public_id,
            kb_id,
            &entry_id.to_string()
        )
    }

    pub async fn list_knowledge_entries(
        &self,
        kb_id: Uuid,
        search: Option<&str>,
        kind: Option<&str>,
    ) -> Result<Vec<KnowledgeEntryRow>> {
        dispatch!(self, list_knowledge_entries, kb_id, search, kind)
    }

    pub async fn search_knowledge_entries(
        &self,
        kb_ids: &[Uuid],
        query: &str,
        kind: Option<&str>,
        tags: &[String],
        limit: usize,
    ) -> Result<Vec<KnowledgeEntryRow>> {
        dispatch!(
            self,
            search_knowledge_entries,
            kb_ids,
            query,
            kind,
            tags,
            limit
        )
    }

    pub async fn update_knowledge_entry(
        &self,
        kb_id: Uuid,
        id: Uuid,
        input: UpdateKnowledgeEntry,
    ) -> Result<Option<KnowledgeEntryRow>> {
        dispatch!(self, update_knowledge_entry, kb_id, id, input)
    }

    pub async fn delete_knowledge_entry(&self, kb_id: Uuid, id: Uuid) -> Result<bool> {
        dispatch!(self, delete_knowledge_entry, kb_id, id)
    }

    // ============================================
    // Knowledge Indexes (see knowledge/runtime-resources/knowledge-indexes.md)
    // ============================================

    pub async fn create_knowledge_index(
        &self,
        org_id: i64,
        input: CreateKnowledgeIndexRow,
    ) -> Result<KnowledgeIndexRow> {
        dispatch!(self, create_knowledge_index, org_id, input)
    }

    pub async fn get_knowledge_index(
        &self,
        org_id: i64,
        index_id: KnowledgeIndexId,
    ) -> Result<Option<KnowledgeIndexRow>> {
        dispatch!(
            self,
            get_knowledge_index_by_public_id,
            org_id,
            &index_id.to_string()
        )
    }

    pub async fn get_knowledge_index_by_public_id(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<KnowledgeIndexRow>> {
        dispatch!(self, get_knowledge_index_by_public_id, org_id, public_id)
    }

    pub async fn get_knowledge_index_by_id(
        &self,
        org_id: i64,
        id: Uuid,
    ) -> Result<Option<KnowledgeIndexRow>> {
        dispatch!(self, get_knowledge_index_by_id, org_id, id)
    }

    pub async fn list_knowledge_indexes(
        &self,
        org_id: i64,
        search: Option<&str>,
        include_archived: bool,
    ) -> Result<Vec<KnowledgeIndexRow>> {
        dispatch!(
            self,
            list_knowledge_indexes,
            org_id,
            search,
            include_archived
        )
    }

    pub async fn update_knowledge_index(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateKnowledgeIndex,
    ) -> Result<Option<KnowledgeIndexRow>> {
        dispatch!(self, update_knowledge_index, org_id, id, input)
    }

    pub async fn archive_knowledge_index(&self, org_id: i64, id: Uuid) -> Result<bool> {
        dispatch!(self, archive_knowledge_index, org_id, id)
    }

    pub async fn list_knowledge_index_documents(
        &self,
        index_id: Uuid,
    ) -> Result<Vec<KnowledgeIndexDocumentRow>> {
        dispatch!(self, list_knowledge_index_documents, index_id)
    }

    pub async fn count_knowledge_index_documents(
        &self,
        index_ids: &[Uuid],
    ) -> Result<std::collections::HashMap<Uuid, usize>> {
        dispatch!(self, count_knowledge_index_documents, index_ids)
    }

    pub async fn list_knowledge_index_chunks(
        &self,
        index_id: Uuid,
    ) -> Result<Vec<KnowledgeIndexChunkRow>> {
        dispatch!(self, list_knowledge_index_chunks, index_id)
    }

    pub async fn get_knowledge_index_chunks_with_documents(
        &self,
        index_id: Uuid,
        chunk_public_ids: &[String],
    ) -> Result<Vec<KnowledgeIndexChunkWithDocument>> {
        dispatch!(
            self,
            get_knowledge_index_chunks_with_documents,
            index_id,
            chunk_public_ids
        )
    }

    pub async fn enqueue_knowledge_index_sync(
        &self,
        org_id: i64,
        id: Uuid,
    ) -> Result<Option<KnowledgeIndexRow>> {
        dispatch!(self, enqueue_knowledge_index_sync, org_id, id)
    }

    pub async fn claim_next_knowledge_index_sync(&self) -> Result<Option<KnowledgeIndexRow>> {
        dispatch!(self, claim_next_knowledge_index_sync)
    }

    pub async fn complete_knowledge_index_sync(
        &self,
        index_id: Uuid,
        claimed_at: DateTime<Utc>,
        documents: Vec<CreateKnowledgeIndexDocumentWithChunks>,
        vector_dim: Option<i32>,
    ) -> Result<Option<KnowledgeIndexRow>> {
        dispatch!(
            self,
            complete_knowledge_index_sync,
            index_id,
            claimed_at,
            documents,
            vector_dim
        )
    }

    pub async fn fail_knowledge_index_sync(
        &self,
        index_id: Uuid,
        claimed_at: DateTime<Utc>,
        error: &str,
    ) -> Result<Option<KnowledgeIndexRow>> {
        dispatch!(self, fail_knowledge_index_sync, index_id, claimed_at, error)
    }

    // ============================================
    // Pinned Sessions
    // ============================================

    pub async fn pin_session(
        &self,
        user_id: Uuid,
        session_id: SessionId,
        org_id: i64,
    ) -> Result<()> {
        dispatch!(self, pin_session, user_id, session_id, org_id)
    }

    pub async fn unpin_session(
        &self,
        user_id: Uuid,
        session_id: SessionId,
        org_id: i64,
    ) -> Result<bool> {
        dispatch!(self, unpin_session, user_id, session_id, org_id)
    }

    pub async fn list_pinned_session_ids(
        &self,
        user_id: Uuid,
        org_id: i64,
    ) -> Result<Vec<SessionId>> {
        #[cfg(test)]
        self.record_session_list_lookup().await;
        dispatch!(self, list_pinned_session_ids, user_id, org_id)
    }

    // ============================================
    // Notifications
    // ============================================

    pub async fn create_notification_turn_request(
        &self,
        input: CreateNotificationTurnRequestRow,
    ) -> Result<()> {
        dispatch!(self, create_notification_turn_request, input)
    }

    pub async fn get_notification_turn_request(
        &self,
        input_message_id: MessageId,
    ) -> Result<Option<NotificationTurnRequestRow>> {
        dispatch!(self, get_notification_turn_request, input_message_id)
    }

    pub async fn create_notification(
        &self,
        input: CreateNotificationRow,
    ) -> Result<NotificationRow> {
        dispatch!(self, create_notification, input)
    }

    pub async fn get_notification(
        &self,
        org_id: i64,
        user_id: Uuid,
        id: NotificationId,
    ) -> Result<Option<NotificationRow>> {
        dispatch!(self, get_notification, org_id, user_id, id)
    }

    pub async fn list_notifications(
        &self,
        org_id: i64,
        user_id: Uuid,
        limit: i64,
    ) -> Result<Vec<NotificationRow>> {
        dispatch!(self, list_notifications, org_id, user_id, limit)
    }

    pub async fn list_notifications_updated_since(
        &self,
        org_id: i64,
        user_id: Uuid,
        updated_since: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<Vec<NotificationRow>> {
        dispatch!(
            self,
            list_notifications_updated_since,
            org_id,
            user_id,
            updated_since,
            limit
        )
    }

    pub async fn count_unviewed_notifications(&self, org_id: i64, user_id: Uuid) -> Result<u32> {
        dispatch!(self, count_unviewed_notifications, org_id, user_id)
    }

    pub async fn count_unviewed_notifications_by_kind(
        &self,
        org_id: i64,
        user_id: Uuid,
        kind: &str,
    ) -> Result<u32> {
        dispatch!(
            self,
            count_unviewed_notifications_by_kind,
            org_id,
            user_id,
            kind
        )
    }

    pub async fn mark_notification_viewed(
        &self,
        org_id: i64,
        user_id: Uuid,
        id: NotificationId,
    ) -> Result<Option<NotificationRow>> {
        dispatch!(self, mark_notification_viewed, org_id, user_id, id)
    }

    // ============================================
    // Events
    // ============================================

    pub async fn create_event(&self, input: CreateEventRow) -> Result<EventRow> {
        dispatch!(self, create_event, input)
    }

    /// Check if an input.message event with a given slack_ts already exists in a session.
    /// Used for Slack event dedup across server instances.
    pub async fn has_event_with_slack_ts(
        &self,
        session_id: SessionId,
        slack_ts: &str,
    ) -> Result<bool> {
        dispatch!(self, has_event_with_slack_ts, session_id, slack_ts)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn list_events(
        &self,
        session_id: SessionId,
        since_sequence: Option<i32>,
        since_id: Option<EventId>,
        filter_types: &[String],
        exclude_types: &[String],
        before_sequence: Option<i32>,
        limit: Option<i32>,
    ) -> Result<Vec<EventRow>> {
        dispatch!(
            self,
            list_events,
            session_id,
            since_sequence,
            since_id,
            filter_types,
            exclude_types,
            before_sequence,
            limit
        )
    }

    /// Count events for a session using SELECT COUNT(*) — no row materialization.
    pub async fn count_events(
        &self,
        session_id: SessionId,
        exclude_types: &[String],
    ) -> Result<i64> {
        dispatch!(self, count_events, session_id, exclude_types)
    }

    /// Advanced event listing for debugging.
    /// Supports time range, context-id (turn/exec/trace), tags, tool name,
    /// full-text search, around-id windowing, and direction.
    pub async fn list_events_advanced(
        &self,
        params: &crate::storage::models::ListEventsParams,
    ) -> Result<Vec<EventRow>> {
        dispatch!(self, list_events_advanced, params)
    }

    /// One-shot debug summary: per-type counts + first/last timestamps.
    pub async fn events_summary(
        &self,
        session_id: SessionId,
    ) -> Result<crate::storage::models::EventsSummary> {
        dispatch!(self, events_summary, session_id)
    }

    /// Find the nearest turn.started sequence at or before the given sequence.
    pub async fn find_turn_boundary(
        &self,
        session_id: SessionId,
        before_sequence: i32,
    ) -> Result<Option<i32>> {
        dispatch!(self, find_turn_boundary, session_id, before_sequence)
    }

    pub async fn list_message_events(&self, session_id: SessionId) -> Result<Vec<EventRow>> {
        dispatch!(self, list_message_events, session_id)
    }

    /// List message events with an optional limit on count.
    /// Returns most recent N messages in sequence order when limit is provided.
    pub async fn list_message_events_limited(
        &self,
        session_id: SessionId,
        limit: Option<i32>,
    ) -> Result<Vec<EventRow>> {
        dispatch!(self, list_message_events_limited, session_id, limit)
    }

    /// Count message events for a session using COUNT(*) — no row materialization.
    pub async fn count_message_events(&self, session_id: SessionId) -> Result<i64> {
        dispatch!(self, count_message_events, session_id)
    }

    /// List message events with filters applied
    ///
    /// This method applies the filters from the MessageQuery to efficiently
    /// retrieve messages. DB-mappable filters are pushed to the database,
    /// while custom filters are applied in-memory.
    ///
    /// Note: Injections are NOT applied here - they should be applied at the
    /// MessageRetriever layer after converting events to messages.
    pub async fn list_message_events_filtered(
        &self,
        query: &MessageQuery,
    ) -> Result<Vec<EventRow>> {
        dispatch!(self, list_message_events_filtered, query)
    }

    pub async fn get_compaction_checkpoint(
        &self,
        session_id: SessionId,
        provider_type: &str,
        model: &str,
        format_version: i32,
    ) -> Result<Option<CompactionCheckpointRow>> {
        dispatch!(
            self,
            get_compaction_checkpoint,
            session_id,
            provider_type,
            model,
            format_version
        )
    }

    pub async fn install_compaction_checkpoint(
        &self,
        input: InstallCompactionCheckpointRow,
    ) -> Result<bool> {
        dispatch!(self, install_compaction_checkpoint, input)
    }

    pub async fn copy_compaction_checkpoints(
        &self,
        source_session_id: SessionId,
        target_session_id: SessionId,
        through_sequence: i32,
    ) -> Result<u64> {
        dispatch!(
            self,
            copy_compaction_checkpoints,
            source_session_id,
            target_session_id,
            through_sequence
        )
    }

    /// Get preview text for multiple sessions (first user message)
    pub async fn get_session_previews(
        &self,
        session_ids: &[Uuid],
    ) -> Result<std::collections::HashMap<Uuid, String>> {
        #[cfg(test)]
        self.record_session_list_lookup().await;
        dispatch!(self, get_session_previews, session_ids)
    }

    /// Get output preview text for multiple sessions (last agent message)
    pub async fn get_session_output_previews(
        &self,
        session_ids: &[Uuid],
    ) -> Result<std::collections::HashMap<Uuid, String>> {
        #[cfg(test)]
        self.record_session_list_lookup().await;
        dispatch!(self, get_session_output_previews, session_ids)
    }

    // ============================================
    // LLM Providers
    // ============================================

    pub async fn create_provider(
        &self,
        org_id: i64,
        input: CreateProviderRow,
    ) -> Result<ProviderRow> {
        dispatch!(self, create_provider, org_id, input)
    }

    /// Create a provider with a specific ID (for seeding)
    /// Returns None if provider already exists (idempotent)
    pub async fn create_provider_with_id(
        &self,
        org_id: i64,
        id: Uuid,
        input: CreateProviderRow,
    ) -> Result<Option<ProviderRow>> {
        dispatch!(self, create_provider_with_id, org_id, id, input)
    }

    pub async fn get_provider(&self, org_id: i64, id: Uuid) -> Result<Option<ProviderRow>> {
        dispatch!(self, get_provider, org_id, id)
    }

    pub async fn list_providers(&self, org_id: i64) -> Result<Vec<ProviderRow>> {
        dispatch!(self, list_providers, org_id)
    }

    pub async fn update_provider(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateProvider,
    ) -> Result<Option<ProviderRow>> {
        dispatch!(self, update_provider, org_id, id, input)
    }

    pub async fn delete_provider(&self, org_id: i64, id: Uuid) -> Result<bool> {
        dispatch!(self, delete_provider, org_id, id)
    }

    /// Mark (or unmark) a provider as host-managed (EVE-810).
    pub async fn set_provider_managed(&self, org_id: i64, id: Uuid, managed: bool) -> Result<bool> {
        dispatch!(self, set_provider_managed, org_id, id, managed)
    }

    /// Update provider's last_synced_at timestamp
    pub async fn update_provider_last_synced(
        &self,
        org_id: i64,
        id: Uuid,
        last_synced_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<()> {
        dispatch!(
            self,
            update_provider_last_synced,
            org_id,
            id,
            last_synced_at
        )
    }

    /// Get a provider with its decrypted API key
    /// Note: This is not async, so cannot use dispatch! macro
    pub fn get_provider_with_api_key(
        &self,
        provider: &ProviderRow,
        encryption: &super::EncryptionService,
    ) -> Result<ProviderWithApiKey> {
        match self {
            Self::Postgres(db) => db.get_provider_with_api_key(provider, encryption),
            Self::InMemory(db) => db.get_provider_with_api_key(provider, encryption),
        }
    }

    // ============================================
    // LLM Models
    // ============================================

    pub async fn get_default_model(&self, org_id: i64) -> Result<Option<ModelWithProviderRow>> {
        dispatch!(self, get_default_model, org_id)
    }

    pub async fn get_organization_settings(
        &self,
        org_id: i64,
    ) -> Result<Option<OrganizationSettingsRow>> {
        dispatch!(self, get_organization_settings, org_id)
    }

    pub async fn upsert_organization_settings(
        &self,
        org_id: i64,
        default_model_id: Option<uuid::Uuid>,
    ) -> Result<OrganizationSettingsRow> {
        dispatch!(self, upsert_organization_settings, org_id, default_model_id)
    }

    pub async fn patch_organization_settings(
        &self,
        org_id: i64,
        input: UpdateOrganizationSettings,
    ) -> Result<OrganizationSettingsRow> {
        dispatch!(self, patch_organization_settings, org_id, input)
    }

    pub async fn list_org_feature_flags(
        &self,
        org_id: i64,
    ) -> Result<std::collections::HashMap<String, bool>> {
        dispatch!(self, list_org_feature_flags, org_id)
    }

    pub async fn replace_org_feature_flags(
        &self,
        org_id: i64,
        flags: &std::collections::HashMap<String, bool>,
    ) -> Result<()> {
        dispatch!(self, replace_org_feature_flags, org_id, flags)
    }

    pub async fn create_model(&self, org_id: i64, input: CreateModelRow) -> Result<ModelRow> {
        dispatch!(self, create_model, org_id, input)
    }

    /// Create a model with a specific ID (for seeding)
    /// Returns None if model already exists (idempotent)
    pub async fn create_model_with_id(
        &self,
        org_id: i64,
        id: Uuid,
        input: CreateModelRow,
    ) -> Result<Option<ModelRow>> {
        dispatch!(self, create_model_with_id, org_id, id, input)
    }

    pub async fn get_model(&self, org_id: i64, id: Uuid) -> Result<Option<ModelRow>> {
        dispatch!(self, get_model, org_id, id)
    }

    pub async fn get_model_with_provider(
        &self,
        org_id: i64,
        id: Uuid,
    ) -> Result<Option<ModelWithProviderRow>> {
        dispatch!(self, get_model_with_provider, org_id, id)
    }

    pub async fn list_models_for_provider(
        &self,
        org_id: i64,
        provider_id: Uuid,
    ) -> Result<Vec<ModelRow>> {
        dispatch!(self, list_models_for_provider, org_id, provider_id)
    }

    pub async fn list_all_models(&self, org_id: i64) -> Result<Vec<ModelWithProviderRow>> {
        dispatch!(self, list_all_models, org_id)
    }

    pub async fn update_model(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateModel,
    ) -> Result<Option<ModelRow>> {
        dispatch!(self, update_model, org_id, id, input)
    }

    pub async fn delete_model(&self, org_id: i64, id: Uuid) -> Result<bool> {
        dispatch!(self, delete_model, org_id, id)
    }

    pub async fn get_model_by_model_id(
        &self,
        org_id: i64,
        model_id: &str,
    ) -> Result<Option<ModelWithProviderRow>> {
        dispatch!(self, get_model_by_model_id, org_id, model_id)
    }

    // ============================================
    // Agent Capabilities
    // ============================================

    pub async fn get_agent_capabilities(&self, agent_id: Uuid) -> Result<Vec<AgentCapabilityRow>> {
        #[cfg(test)]
        self.record_session_list_lookup().await;
        dispatch!(self, get_agent_capabilities, agent_id)
    }

    pub async fn get_agent_capabilities_by_agent_ids(
        &self,
        org_id: i64,
        agent_ids: &[AgentId],
    ) -> Result<Vec<AgentCapabilityRow>> {
        #[cfg(test)]
        self.record_session_list_lookup().await;
        dispatch!(self, get_agent_capabilities_by_agent_ids, org_id, agent_ids)
    }

    pub async fn set_agent_capabilities(
        &self,
        agent_id: Uuid,
        capabilities: Vec<(String, i32, serde_json::Value)>,
    ) -> Result<Vec<AgentCapabilityRow>> {
        dispatch!(self, set_agent_capabilities, agent_id, capabilities)
    }

    pub async fn add_agent_capability(
        &self,
        input: CreateAgentCapabilityRow,
    ) -> Result<AgentCapabilityRow> {
        dispatch!(self, add_agent_capability, input)
    }

    pub async fn remove_agent_capability(
        &self,
        agent_id: Uuid,
        capability_id: &str,
    ) -> Result<bool> {
        dispatch!(self, remove_agent_capability, agent_id, capability_id)
    }

    // ============================================
    // Harness Capabilities
    // ============================================

    pub async fn get_harness_capabilities(
        &self,
        harness_id: Uuid,
    ) -> Result<Vec<HarnessCapabilityRow>> {
        #[cfg(test)]
        self.record_session_list_lookup().await;
        dispatch!(self, get_harness_capabilities, harness_id)
    }

    pub async fn get_harness_capabilities_by_harness_ids(
        &self,
        org_id: i64,
        harness_ids: &[HarnessId],
    ) -> Result<Vec<HarnessCapabilityRow>> {
        #[cfg(test)]
        self.record_session_list_lookup().await;
        dispatch!(
            self,
            get_harness_capabilities_by_harness_ids,
            org_id,
            harness_ids
        )
    }

    pub async fn set_harness_capabilities(
        &self,
        harness_id: Uuid,
        capabilities: Vec<(String, i32, serde_json::Value)>,
    ) -> Result<Vec<HarnessCapabilityRow>> {
        dispatch!(self, set_harness_capabilities, harness_id, capabilities)
    }

    /// Count active agents per capability_id within an org.
    pub async fn count_agent_capability_references(
        &self,
        org_id: i64,
    ) -> Result<std::collections::HashMap<String, u64>> {
        dispatch!(self, count_agent_capability_references, org_id)
    }

    /// Count active harnesses per capability_id within an org.
    pub async fn count_harness_capability_references(
        &self,
        org_id: i64,
    ) -> Result<std::collections::HashMap<String, u64>> {
        dispatch!(self, count_harness_capability_references, org_id)
    }

    /// Count active agents referencing a single capability_id within an org.
    pub async fn count_agents_for_capability(
        &self,
        org_id: i64,
        capability_id: &str,
    ) -> Result<u64> {
        dispatch!(self, count_agents_for_capability, org_id, capability_id)
    }

    /// Count active harnesses referencing a single capability_id within an org.
    pub async fn count_harnesses_for_capability(
        &self,
        org_id: i64,
        capability_id: &str,
    ) -> Result<u64> {
        dispatch!(self, count_harnesses_for_capability, org_id, capability_id)
    }

    // ============================================
    // Session Files
    // ============================================

    pub async fn create_session_file(&self, input: CreateSessionFileRow) -> Result<SessionFileRow> {
        dispatch!(self, create_session_file, input)
    }

    pub async fn get_session_file(
        &self,
        session_id: Uuid,
        path: &str,
    ) -> Result<Option<SessionFileRow>> {
        dispatch!(self, get_session_file, session_id, path)
    }

    pub async fn get_session_file_info(
        &self,
        session_id: Uuid,
        path: &str,
    ) -> Result<Option<SessionFileInfoRow>> {
        dispatch!(self, get_session_file_info, session_id, path)
    }

    pub async fn list_session_files(
        &self,
        session_id: Uuid,
        parent_path: &str,
    ) -> Result<Vec<SessionFileInfoRow>> {
        dispatch!(self, list_session_files, session_id, parent_path)
    }

    pub async fn list_all_session_files(
        &self,
        session_id: Uuid,
    ) -> Result<Vec<SessionFileInfoRow>> {
        dispatch!(self, list_all_session_files, session_id)
    }

    pub async fn update_session_file(
        &self,
        session_id: Uuid,
        path: &str,
        input: UpdateSessionFile,
    ) -> Result<Option<SessionFileRow>> {
        dispatch!(self, update_session_file, session_id, path, input)
    }

    pub async fn update_session_file_if_content_matches(
        &self,
        session_id: Uuid,
        path: &str,
        expected_content: Vec<u8>,
        input: UpdateSessionFile,
    ) -> Result<Option<SessionFileRow>> {
        dispatch!(
            self,
            update_session_file_if_content_matches,
            session_id,
            path,
            expected_content,
            input
        )
    }

    pub async fn delete_session_file(&self, session_id: Uuid, path: &str) -> Result<bool> {
        dispatch!(self, delete_session_file, session_id, path)
    }

    pub async fn delete_session_file_recursive(&self, session_id: Uuid, path: &str) -> Result<u64> {
        dispatch!(self, delete_session_file_recursive, session_id, path)
    }

    pub async fn move_session_file(
        &self,
        session_id: Uuid,
        source_path: &str,
        dest_path: &str,
    ) -> Result<Option<SessionFileRow>> {
        dispatch!(self, move_session_file, session_id, source_path, dest_path)
    }

    pub async fn copy_session_file(
        &self,
        session_id: Uuid,
        source_path: &str,
        dest_path: &str,
    ) -> Result<Option<SessionFileRow>> {
        dispatch!(self, copy_session_file, session_id, source_path, dest_path)
    }

    pub async fn grep_session_files(
        &self,
        session_id: Uuid,
        pattern: &str,
        path_prefix: Option<&str>,
        excluded_path_prefix: Option<&str>,
        max_file_bytes: i64,
    ) -> Result<Vec<SessionFileInfoRow>> {
        dispatch!(
            self,
            grep_session_files,
            session_id,
            pattern,
            path_prefix,
            excluded_path_prefix,
            max_file_bytes
        )
    }

    pub async fn session_file_exists(&self, session_id: Uuid, path: &str) -> Result<bool> {
        dispatch!(self, session_file_exists, session_id, path)
    }

    pub async fn has_readonly_session_files(&self, session_id: Uuid, path: &str) -> Result<bool> {
        dispatch!(self, has_readonly_session_files, session_id, path)
    }

    pub async fn session_directory_has_children(
        &self,
        session_id: Uuid,
        path: &str,
    ) -> Result<bool> {
        dispatch!(self, session_directory_has_children, session_id, path)
    }

    /// Sum of size_bytes for all non-directory files in a session.
    pub async fn total_session_file_bytes(&self, session_id: Uuid) -> Result<i64> {
        dispatch!(self, total_session_file_bytes, session_id)
    }

    /// Load all non-directory files with content for a session (single query, for git commit).
    pub async fn load_all_session_files_with_content(
        &self,
        session_id: Uuid,
    ) -> Result<Vec<SessionFileRow>> {
        dispatch!(self, load_all_session_files_with_content, session_id)
    }

    // ============================================
    // Session Git Objects
    // ============================================

    pub async fn write_git_object(&self, input: CreateSessionGitObject) -> Result<()> {
        dispatch!(self, write_git_object, input)
    }

    pub async fn write_git_objects_batch(
        &self,
        objects: Vec<CreateSessionGitObject>,
    ) -> Result<()> {
        dispatch!(self, write_git_objects_batch, objects)
    }

    pub async fn read_git_object(
        &self,
        session_id: Uuid,
        oid: &[u8],
    ) -> Result<Option<SessionGitObjectRow>> {
        dispatch!(self, read_git_object, session_id, oid)
    }

    /// Load all git objects for a session in a single query (avoids N+1).
    pub async fn load_all_git_objects(&self, session_id: Uuid) -> Result<Vec<SessionGitObjectRow>> {
        dispatch!(self, load_all_git_objects, session_id)
    }

    pub async fn git_object_exists(&self, session_id: Uuid, oid: &[u8]) -> Result<bool> {
        dispatch!(self, git_object_exists, session_id, oid)
    }

    pub async fn read_git_object_header(
        &self,
        session_id: Uuid,
        oid: &[u8],
    ) -> Result<Option<(i16, i64)>> {
        dispatch!(self, read_git_object_header, session_id, oid)
    }

    pub async fn list_git_object_oids(&self, session_id: Uuid) -> Result<Vec<Vec<u8>>> {
        dispatch!(self, list_git_object_oids, session_id)
    }

    pub async fn fork_git_objects(
        &self,
        source_session_id: Uuid,
        target_session_id: Uuid,
    ) -> Result<u64> {
        dispatch!(self, fork_git_objects, source_session_id, target_session_id)
    }

    // ============================================
    // Session Git Refs
    // ============================================

    pub async fn write_git_ref(&self, input: CreateSessionGitRef) -> Result<()> {
        dispatch!(self, write_git_ref, input)
    }

    pub async fn read_git_ref(
        &self,
        session_id: Uuid,
        name: &str,
    ) -> Result<Option<SessionGitRefRow>> {
        dispatch!(self, read_git_ref, session_id, name)
    }

    pub async fn delete_git_ref(&self, session_id: Uuid, name: &str) -> Result<bool> {
        dispatch!(self, delete_git_ref, session_id, name)
    }

    pub async fn list_git_refs(&self, session_id: Uuid) -> Result<Vec<SessionGitRefRow>> {
        dispatch!(self, list_git_refs, session_id)
    }

    pub async fn fork_git_refs(
        &self,
        source_session_id: Uuid,
        target_session_id: Uuid,
    ) -> Result<u64> {
        dispatch!(self, fork_git_refs, source_session_id, target_session_id)
    }

    // ============================================
    // MCP Servers
    // ============================================

    pub async fn create_mcp_server(
        &self,
        org_id: i64,
        input: CreateMcpServerRow,
    ) -> Result<McpServerRow> {
        dispatch!(self, create_mcp_server, org_id, input)
    }

    pub async fn create_mcp_server_with_id(
        &self,
        org_id: i64,
        id: Uuid,
        input: CreateMcpServerRow,
    ) -> Result<Option<McpServerRow>> {
        dispatch!(self, create_mcp_server_with_id, org_id, id, input)
    }

    pub async fn get_mcp_server(&self, org_id: i64, id: Uuid) -> Result<Option<McpServerRow>> {
        dispatch!(self, get_mcp_server, org_id, id)
    }

    /// Batch fetch multiple MCP servers by IDs in a single query.
    pub async fn get_mcp_servers_batch(
        &self,
        org_id: i64,
        ids: &[Uuid],
    ) -> Result<Vec<McpServerRow>> {
        dispatch!(self, get_mcp_servers_batch, org_id, ids)
    }

    pub async fn get_mcp_server_by_name(
        &self,
        org_id: i64,
        name: &str,
    ) -> Result<Option<McpServerRow>> {
        dispatch!(self, get_mcp_server_by_name, org_id, name)
    }

    pub async fn list_mcp_servers(
        &self,
        org_id: i64,
        search: Option<&str>,
        include_archived: bool,
    ) -> Result<Vec<McpServerRow>> {
        dispatch!(self, list_mcp_servers, org_id, search, include_archived)
    }

    pub async fn list_active_mcp_servers(&self, org_id: i64) -> Result<Vec<McpServerRow>> {
        dispatch!(self, list_active_mcp_servers, org_id)
    }

    pub async fn update_mcp_server(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateMcpServer,
    ) -> Result<Option<McpServerRow>> {
        dispatch!(self, update_mcp_server, org_id, id, input)
    }

    pub async fn update_mcp_server_tools(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateMcpServerTools,
    ) -> Result<Option<McpServerRow>> {
        dispatch!(self, update_mcp_server_tools, org_id, id, input)
    }

    pub async fn delete_mcp_server(&self, org_id: i64, id: Uuid) -> Result<bool> {
        dispatch!(self, delete_mcp_server, org_id, id)
    }

    pub async fn destroy_mcp_server(&self, org_id: i64, id: Uuid) -> Result<bool> {
        dispatch!(self, destroy_mcp_server, org_id, id)
    }

    // ============================================
    // Skills
    // ============================================

    pub async fn create_skill(&self, org_id: i64, input: CreateSkillRow) -> Result<SkillRow> {
        dispatch!(self, create_skill, org_id, input)
    }

    pub async fn get_skill(&self, org_id: i64, id: Uuid) -> Result<Option<SkillRow>> {
        dispatch!(self, get_skill, org_id, id)
    }

    pub async fn get_skill_by_name(&self, org_id: i64, name: &str) -> Result<Option<SkillRow>> {
        dispatch!(self, get_skill_by_name, org_id, name)
    }

    pub async fn list_skills(
        &self,
        org_id: i64,
        search: Option<&str>,
        include_archived: bool,
    ) -> Result<Vec<SkillRow>> {
        dispatch!(self, list_skills, org_id, search, include_archived)
    }

    pub async fn list_non_deleted_skill_ids(&self, org_id: i64) -> Result<Vec<Uuid>> {
        dispatch!(self, list_non_deleted_skill_ids, org_id)
    }

    pub async fn update_skill(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateSkill,
    ) -> Result<Option<SkillRow>> {
        dispatch!(self, update_skill, org_id, id, input)
    }

    pub async fn delete_skill(&self, org_id: i64, id: Uuid) -> Result<bool> {
        dispatch!(self, delete_skill, org_id, id)
    }

    pub async fn destroy_skill(&self, org_id: i64, id: Uuid) -> Result<bool> {
        dispatch!(self, destroy_skill, org_id, id)
    }

    // ============================================
    // Declarative Capabilities
    // ============================================

    pub async fn create_declarative_capability(
        &self,
        org_id: i64,
        input: CreateDeclarativeCapabilityRow,
    ) -> Result<DeclarativeCapabilityRow> {
        dispatch!(self, create_declarative_capability, org_id, input)
    }

    pub async fn get_declarative_capability(
        &self,
        org_id: i64,
        id: Uuid,
    ) -> Result<Option<DeclarativeCapabilityRow>> {
        dispatch!(self, get_declarative_capability, org_id, id)
    }

    pub async fn get_declarative_capability_by_name(
        &self,
        org_id: i64,
        name: &str,
    ) -> Result<Option<DeclarativeCapabilityRow>> {
        dispatch!(self, get_declarative_capability_by_name, org_id, name)
    }

    pub async fn get_declarative_capability_by_public_id(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<DeclarativeCapabilityRow>> {
        dispatch!(
            self,
            get_declarative_capability_by_public_id,
            org_id,
            public_id
        )
    }

    pub async fn list_declarative_capabilities(
        &self,
        org_id: i64,
        search: Option<&str>,
        include_archived: bool,
    ) -> Result<Vec<DeclarativeCapabilityRow>> {
        dispatch!(
            self,
            list_declarative_capabilities,
            org_id,
            search,
            include_archived
        )
    }

    pub async fn update_declarative_capability(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateDeclarativeCapability,
    ) -> Result<Option<DeclarativeCapabilityRow>> {
        dispatch!(self, update_declarative_capability, org_id, id, input)
    }

    pub async fn delete_declarative_capability(&self, org_id: i64, id: Uuid) -> Result<bool> {
        dispatch!(self, delete_declarative_capability, org_id, id)
    }

    pub async fn destroy_declarative_capability(&self, org_id: i64, id: Uuid) -> Result<bool> {
        dispatch!(self, destroy_declarative_capability, org_id, id)
    }

    // ============================================
    // Plugin Marketplaces
    // ============================================

    pub async fn create_plugin_marketplace(
        &self,
        org_id: i64,
        input: CreatePluginMarketplaceRow,
    ) -> Result<PluginMarketplaceRow> {
        dispatch!(self, create_plugin_marketplace, org_id, input)
    }

    pub async fn get_plugin_marketplace(
        &self,
        org_id: i64,
        id: Uuid,
    ) -> Result<Option<PluginMarketplaceRow>> {
        dispatch!(self, get_plugin_marketplace, org_id, id)
    }

    pub async fn get_plugin_marketplace_by_public_id(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<PluginMarketplaceRow>> {
        dispatch!(self, get_plugin_marketplace_by_public_id, org_id, public_id)
    }

    pub async fn list_plugin_marketplaces(
        &self,
        org_id: i64,
        search: Option<&str>,
    ) -> Result<Vec<PluginMarketplaceRow>> {
        dispatch!(self, list_plugin_marketplaces, org_id, search)
    }

    pub async fn update_plugin_marketplace(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdatePluginMarketplace,
    ) -> Result<Option<PluginMarketplaceRow>> {
        dispatch!(self, update_plugin_marketplace, org_id, id, input)
    }

    pub async fn delete_plugin_marketplace(&self, org_id: i64, id: Uuid) -> Result<bool> {
        dispatch!(self, delete_plugin_marketplace, org_id, id)
    }

    // ============================================
    // Plugin Installs
    // ============================================

    pub async fn create_plugin_install(
        &self,
        org_id: i64,
        input: CreatePluginInstallRow,
    ) -> Result<PluginInstallRow> {
        dispatch!(self, create_plugin_install, org_id, input)
    }

    pub async fn get_plugin_install(
        &self,
        org_id: i64,
        id: Uuid,
    ) -> Result<Option<PluginInstallRow>> {
        dispatch!(self, get_plugin_install, org_id, id)
    }

    pub async fn get_plugin_install_by_public_id(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<PluginInstallRow>> {
        dispatch!(self, get_plugin_install_by_public_id, org_id, public_id)
    }

    pub async fn get_plugin_install_by_name(
        &self,
        org_id: i64,
        name: &str,
    ) -> Result<Option<PluginInstallRow>> {
        dispatch!(self, get_plugin_install_by_name, org_id, name)
    }

    pub async fn list_plugin_installs(
        &self,
        org_id: i64,
        search: Option<&str>,
    ) -> Result<Vec<PluginInstallRow>> {
        dispatch!(self, list_plugin_installs, org_id, search)
    }

    pub async fn list_active_plugin_installs(&self, org_id: i64) -> Result<Vec<PluginInstallRow>> {
        dispatch!(self, list_active_plugin_installs, org_id)
    }

    pub async fn update_plugin_install(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdatePluginInstall,
    ) -> Result<Option<PluginInstallRow>> {
        dispatch!(self, update_plugin_install, org_id, id, input)
    }

    pub async fn delete_plugin_install(&self, org_id: i64, id: Uuid) -> Result<bool> {
        dispatch!(self, delete_plugin_install, org_id, id)
    }

    // ============================================
    // Skill Files
    // ============================================

    pub async fn create_skill_file(&self, input: CreateSkillFileRow) -> Result<SkillFileRow> {
        dispatch!(self, create_skill_file, input)
    }

    pub async fn list_skill_files(&self, skill_id: Uuid) -> Result<Vec<SkillFileRow>> {
        dispatch!(self, list_skill_files, skill_id)
    }

    pub async fn delete_skill_files(&self, skill_id: Uuid) -> Result<u64> {
        dispatch!(self, delete_skill_files, skill_id)
    }

    // ============================================
    // LLM Generations (Usage Tracking)
    // ============================================

    #[allow(clippy::too_many_arguments)]
    pub async fn create_llm_generation(
        &self,
        org_id: i64,
        session_id: Option<Uuid>,
        turn_id: Option<Uuid>,
        event_id: Option<Uuid>,
        model: String,
        provider: Option<String>,
        input_tokens: i64,
        output_tokens: i64,
        cache_read_tokens: i64,
        cache_creation_tokens: i64,
        actual_cost_usd: Option<f64>,
        estimated_cost_usd: Option<f64>,
        duration_ms: Option<i32>,
        finish_reason: Option<String>,
        provider_response_id: Option<String>,
        created_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<()> {
        dispatch!(
            self,
            create_llm_generation,
            org_id,
            session_id,
            turn_id,
            event_id,
            model,
            provider,
            input_tokens,
            output_tokens,
            cache_read_tokens,
            cache_creation_tokens,
            actual_cost_usd,
            estimated_cost_usd,
            duration_ms,
            finish_reason,
            provider_response_id,
            created_at
        )
    }

    pub async fn list_unreconciled_llm_generations(
        &self,
        provider: &str,
        limit: i64,
    ) -> Result<Vec<crate::storage::models::UnreconciledGeneration>> {
        dispatch!(self, list_unreconciled_llm_generations, provider, limit)
    }

    pub async fn reconcile_llm_generation(
        &self,
        id: Uuid,
        input_tokens: Option<i64>,
        output_tokens: Option<i64>,
        actual_cost_usd: Option<f64>,
        reconciled_provider: Option<&str>,
        reconciled_model: Option<&str>,
    ) -> Result<()> {
        dispatch!(
            self,
            reconcile_llm_generation,
            id,
            input_tokens,
            output_tokens,
            actual_cost_usd,
            reconciled_provider,
            reconciled_model
        )
    }

    pub async fn mark_llm_generation_reconciliation_failed(
        &self,
        id: Uuid,
        retry_after_seconds: i32,
    ) -> Result<()> {
        dispatch!(
            self,
            mark_llm_generation_reconciliation_failed,
            id,
            retry_after_seconds
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn increment_session_usage(
        &self,
        session_id: Uuid,
        input_tokens: i64,
        output_tokens: i64,
        cache_read_tokens: i64,
        cache_creation_tokens: i64,
        actual_cost_usd: f64,
        estimated_cost_usd: f64,
        cost_usd: f64,
    ) -> Result<()> {
        dispatch!(
            self,
            increment_session_usage,
            session_id,
            input_tokens,
            output_tokens,
            cache_read_tokens,
            cache_creation_tokens,
            actual_cost_usd,
            estimated_cost_usd,
            cost_usd
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn increment_agent_usage(
        &self,
        agent_id: Uuid,
        input_tokens: i64,
        output_tokens: i64,
        cache_read_tokens: i64,
        cache_creation_tokens: i64,
        actual_cost_usd: f64,
        estimated_cost_usd: f64,
        cost_usd: f64,
    ) -> Result<()> {
        dispatch!(
            self,
            increment_agent_usage,
            agent_id,
            input_tokens,
            output_tokens,
            cache_read_tokens,
            cache_creation_tokens,
            actual_cost_usd,
            estimated_cost_usd,
            cost_usd
        )
    }

    // ============================================
    // Images
    // ============================================

    pub async fn create_image(&self, org_id: i64, input: CreateImageRow) -> Result<ImageRow> {
        dispatch!(self, create_image, org_id, input)
    }

    pub async fn get_image(&self, org_id: i64, id: Uuid) -> Result<Option<ImageRow>> {
        dispatch!(self, get_image, org_id, id)
    }

    pub async fn get_image_info(&self, org_id: i64, id: Uuid) -> Result<Option<ImageInfoRow>> {
        dispatch!(self, get_image_info, org_id, id)
    }

    pub async fn delete_image(&self, org_id: i64, id: Uuid) -> Result<bool> {
        dispatch!(self, delete_image, org_id, id)
    }

    pub async fn list_images(
        &self,
        org_id: i64,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<ImageInfoRow>> {
        dispatch!(self, list_images, org_id, limit, offset)
    }

    // ============================================
    // Organizations
    // ============================================

    pub async fn create_organization(
        &self,
        input: CreateOrganizationRow,
    ) -> Result<OrganizationRow> {
        dispatch!(self, create_organization, input)
    }

    /// Create organization with specific org_id (for seeding).
    /// Returns None if org_id already exists.
    pub async fn create_organization_with_id(
        &self,
        org_id: i64,
        input: CreateOrganizationRow,
    ) -> Result<Option<OrganizationRow>> {
        dispatch!(self, create_organization_with_id, org_id, input)
    }

    pub async fn get_organization(&self, org_id: i64) -> Result<Option<OrganizationRow>> {
        dispatch!(self, get_organization, org_id)
    }

    pub async fn get_organization_by_public_id(
        &self,
        public_id: &str,
    ) -> Result<Option<OrganizationRow>> {
        dispatch!(self, get_organization_by_public_id, public_id)
    }

    pub async fn list_organizations(&self) -> Result<Vec<OrganizationRow>> {
        dispatch!(self, list_organizations)
    }

    pub async fn update_organization(
        &self,
        org_id: i64,
        input: UpdateOrganization,
    ) -> Result<Option<OrganizationRow>> {
        dispatch!(self, update_organization, org_id, input)
    }

    pub async fn delete_organization(&self, org_id: i64) -> Result<bool> {
        dispatch!(self, delete_organization, org_id)
    }

    /// Idempotently mark an org's onboarding complete (no-op if already set).
    pub async fn mark_org_onboarding_complete(&self, org_id: i64) -> Result<()> {
        dispatch!(self, mark_org_onboarding_complete, org_id)
    }

    // ============================================
    // Organization Members
    // ============================================

    pub async fn add_organization_member(
        &self,
        org_id: i64,
        user_id: Uuid,
        role: &str,
    ) -> Result<OrganizationMemberRow> {
        dispatch!(self, add_organization_member, org_id, user_id, role)
    }

    pub async fn remove_organization_member(&self, org_id: i64, user_id: Uuid) -> Result<bool> {
        dispatch!(self, remove_organization_member, org_id, user_id)
    }

    pub async fn list_organization_members(
        &self,
        org_id: i64,
    ) -> Result<Vec<OrganizationMemberRow>> {
        dispatch!(self, list_organization_members, org_id)
    }

    pub async fn list_organization_members_with_users(
        &self,
        org_id: i64,
    ) -> Result<Vec<OrganizationMemberWithUserRow>> {
        dispatch!(self, list_organization_members_with_users, org_id)
    }

    pub async fn get_organization_member(
        &self,
        org_id: i64,
        user_id: Uuid,
    ) -> Result<Option<OrganizationMemberWithUserRow>> {
        dispatch!(self, get_organization_member, org_id, user_id)
    }

    pub async fn update_organization_member_role(
        &self,
        org_id: i64,
        user_id: Uuid,
        role: &str,
    ) -> Result<Option<OrganizationMemberRow>> {
        dispatch!(self, update_organization_member_role, org_id, user_id, role)
    }

    pub async fn count_organization_owners(&self, org_id: i64) -> Result<i64> {
        dispatch!(self, count_organization_owners, org_id)
    }

    pub async fn count_user_created_organizations(&self, user_id: Uuid) -> Result<i64> {
        dispatch!(self, count_user_created_organizations, user_id)
    }

    pub async fn count_organization_members(&self, org_id: i64) -> Result<i64> {
        dispatch!(self, count_organization_members, org_id)
    }

    pub async fn list_user_organizations(
        &self,
        user_id: Uuid,
    ) -> Result<Vec<OrganizationWithRoleRow>> {
        dispatch!(self, list_user_organizations, user_id)
    }

    pub async fn is_organization_member(&self, org_id: i64, user_id: Uuid) -> Result<bool> {
        dispatch!(self, is_organization_member, org_id, user_id)
    }

    // ============================================
    // External Identity (for SaaS auth providers)
    // ============================================

    pub async fn get_user_by_external_id(&self, external_id: &str) -> Result<Option<UserRow>> {
        dispatch!(self, get_user_by_external_id, external_id)
    }

    pub async fn get_organization_by_external_id(
        &self,
        external_id: &str,
    ) -> Result<Option<OrganizationRow>> {
        dispatch!(self, get_organization_by_external_id, external_id)
    }

    pub async fn upsert_org_by_external_id(
        &self,
        external_id: &str,
        public_id: &str,
        name: &str,
    ) -> Result<OrganizationRow> {
        dispatch!(
            self,
            upsert_org_by_external_id,
            external_id,
            public_id,
            name
        )
    }

    pub async fn ensure_membership(&self, user_id: Uuid, org_id: i64, role: &str) -> Result<()> {
        dispatch!(self, ensure_membership, user_id, org_id, role)
    }

    /// Reconcile org memberships to match an authoritative list from an external
    /// identity provider. Returns `(added, updated, removed)` counts.
    pub async fn reconcile_memberships(
        &self,
        org_id: i64,
        authoritative: &[(Uuid, String)],
    ) -> Result<(usize, usize, usize)> {
        dispatch!(self, reconcile_memberships, org_id, authoritative)
    }

    // ============================================
    // Organization Task Webhooks
    // ============================================

    pub async fn list_org_task_webhooks(&self, org_id: i64) -> Result<Vec<OrgTaskWebhookRow>> {
        dispatch!(self, list_org_task_webhooks, org_id)
    }

    pub async fn list_enabled_org_task_webhooks(
        &self,
        org_id: i64,
    ) -> Result<Vec<OrgTaskWebhookRow>> {
        dispatch!(self, list_enabled_org_task_webhooks, org_id)
    }

    pub async fn create_org_task_webhook(
        &self,
        input: CreateOrgTaskWebhook,
    ) -> Result<OrgTaskWebhookRow> {
        dispatch!(self, create_org_task_webhook, input)
    }

    pub async fn get_org_task_webhook(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<OrgTaskWebhookRow>> {
        dispatch!(self, get_org_task_webhook, org_id, public_id)
    }

    pub async fn update_org_task_webhook(
        &self,
        org_id: i64,
        public_id: &str,
        input: UpdateOrgTaskWebhook,
    ) -> Result<Option<OrgTaskWebhookRow>> {
        dispatch!(self, update_org_task_webhook, org_id, public_id, input)
    }

    pub async fn delete_org_task_webhook(&self, org_id: i64, public_id: &str) -> Result<bool> {
        dispatch!(self, delete_org_task_webhook, org_id, public_id)
    }

    // ============================================
    // Organization Invitations (EVE-602)
    // ============================================

    pub async fn create_org_invitation(
        &self,
        input: CreateOrgInvitation,
    ) -> Result<OrgInvitationRow> {
        dispatch!(self, create_org_invitation, input)
    }

    pub async fn list_pending_org_invitations(&self, org_id: i64) -> Result<Vec<OrgInvitationRow>> {
        dispatch!(self, list_pending_org_invitations, org_id)
    }

    pub async fn get_org_invitation_by_token_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<OrgInvitationRow>> {
        dispatch!(self, get_org_invitation_by_token_hash, token_hash)
    }

    pub async fn get_org_invitation_by_public_id(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<OrgInvitationRow>> {
        dispatch!(self, get_org_invitation_by_public_id, org_id, public_id)
    }

    pub async fn get_outstanding_org_invitation_by_email(
        &self,
        org_id: i64,
        email: &str,
    ) -> Result<Option<OrgInvitationRow>> {
        dispatch!(self, get_outstanding_org_invitation_by_email, org_id, email)
    }

    pub async fn revoke_org_invitation(&self, org_id: i64, public_id: &str) -> Result<bool> {
        dispatch!(self, revoke_org_invitation, org_id, public_id)
    }

    pub async fn accept_org_invitation(
        &self,
        invitation_id: i64,
        accepted_by: Uuid,
    ) -> Result<Option<OrgInvitationRow>> {
        dispatch!(self, accept_org_invitation, invitation_id, accepted_by)
    }

    // ============================================
    // Session Storage (Key-Value & Secrets)
    // ============================================

    pub async fn list_session_keys(&self, session_id: Uuid) -> Result<Vec<SessionKeyInfoRow>> {
        dispatch!(self, list_session_keys, session_id)
    }

    pub async fn upsert_session_key_value(
        &self,
        input: UpsertSessionKeyValue,
    ) -> Result<SessionKeyValueRow> {
        dispatch!(self, upsert_session_key_value, input)
    }

    pub async fn get_session_key_value(
        &self,
        session_id: Uuid,
        key: &str,
    ) -> Result<Option<SessionKeyValueRow>> {
        dispatch!(self, get_session_key_value, session_id, key)
    }

    pub async fn list_session_secrets(
        &self,
        session_id: Uuid,
    ) -> Result<Vec<SessionSecretInfoRow>> {
        dispatch!(self, list_session_secrets, session_id)
    }

    pub async fn get_session_secret(
        &self,
        session_id: Uuid,
        name: &str,
    ) -> Result<Option<SessionSecretRow>> {
        dispatch!(self, get_session_secret, session_id, name)
    }

    pub async fn upsert_session_secret(
        &self,
        input: UpsertSessionSecret,
    ) -> Result<SessionSecretRow> {
        dispatch!(self, upsert_session_secret, input)
    }

    pub async fn delete_session_secret(&self, session_id: Uuid, name: &str) -> Result<bool> {
        dispatch!(self, delete_session_secret, session_id, name)
    }

    pub async fn get_mcp_oauth_session_credentials(
        &self,
        session_id: SessionId,
        server_id: Uuid,
    ) -> Result<Option<McpOAuthSessionCredentialsRow>> {
        dispatch!(
            self,
            get_mcp_oauth_session_credentials,
            session_id,
            server_id
        )
    }

    pub async fn upsert_mcp_oauth_session_credentials(
        &self,
        input: UpsertMcpOAuthSessionCredentials,
    ) -> Result<()> {
        dispatch!(self, upsert_mcp_oauth_session_credentials, input)
    }

    // ============================================
    // User Connections
    // ============================================

    pub async fn upsert_user_connection(
        &self,
        input: CreateUserConnectionRow,
    ) -> Result<UserConnectionRow> {
        dispatch!(self, upsert_user_connection, input)
    }

    pub async fn get_user_connection(
        &self,
        user_id: Uuid,
        provider: &str,
    ) -> Result<Option<UserConnectionRow>> {
        dispatch!(self, get_user_connection, user_id, provider)
    }

    pub async fn list_user_connections(&self, user_id: Uuid) -> Result<Vec<UserConnectionRow>> {
        dispatch!(self, list_user_connections, user_id)
    }

    pub async fn update_user_connection_oauth_tokens(
        &self,
        input: UpdateUserConnectionOAuthTokens,
    ) -> Result<Option<UserConnectionRow>> {
        dispatch!(self, update_user_connection_oauth_tokens, input)
    }

    pub async fn delete_user_connection(&self, user_id: Uuid, provider: &str) -> Result<bool> {
        dispatch!(self, delete_user_connection, user_id, provider)
    }

    pub async fn list_user_preferences(
        &self,
        user_id: Uuid,
        limit: usize,
    ) -> Result<Vec<UserPreferenceRow>> {
        dispatch!(self, list_user_preferences, user_id, limit)
    }

    pub async fn get_user_preference(
        &self,
        user_id: Uuid,
        key: &str,
    ) -> Result<Option<UserPreferenceRow>> {
        dispatch!(self, get_user_preference, user_id, key)
    }

    pub async fn set_user_preference(
        &self,
        user_id: Uuid,
        key: &str,
        value: &str,
        max_preferences: usize,
    ) -> Result<UserPreferenceRow> {
        dispatch!(
            self,
            set_user_preference,
            user_id,
            key,
            value,
            max_preferences
        )
    }

    pub async fn delete_user_preference(&self, user_id: Uuid, key: &str) -> Result<bool> {
        dispatch!(self, delete_user_preference, user_id, key)
    }

    pub async fn get_connection_token_for_session(
        &self,
        session_id: SessionId,
        provider: &str,
    ) -> Result<Option<Vec<u8>>> {
        dispatch!(self, get_connection_token_for_session, session_id, provider)
    }

    pub async fn get_connection_metadata_for_session(
        &self,
        session_id: SessionId,
        provider: &str,
    ) -> Result<Option<serde_json::Value>> {
        dispatch!(
            self,
            get_connection_metadata_for_session,
            session_id,
            provider
        )
    }

    pub async fn get_connection_user_for_session(
        &self,
        session_id: SessionId,
        provider: &str,
    ) -> Result<Option<Uuid>> {
        dispatch!(self, get_connection_user_for_session, session_id, provider)
    }

    pub async fn get_connection_token_for_user(
        &self,
        user_id: Uuid,
        provider: &str,
    ) -> Result<Option<Vec<u8>>> {
        dispatch!(self, get_connection_token_for_user, user_id, provider)
    }

    pub async fn get_installation_id_for_session(
        &self,
        session_id: SessionId,
        provider: &str,
    ) -> Result<Option<i64>> {
        dispatch!(self, get_installation_id_for_session, session_id, provider)
    }

    pub async fn get_installation_id_for_user(
        &self,
        user_id: Uuid,
        provider: &str,
    ) -> Result<Option<i64>> {
        dispatch!(self, get_installation_id_for_user, user_id, provider)
    }

    pub async fn get_user_id_by_installation_id(
        &self,
        provider: &str,
        installation_id: i64,
    ) -> Result<Option<Uuid>> {
        dispatch!(
            self,
            get_user_id_by_installation_id,
            provider,
            installation_id
        )
    }

    // ============================================
    // Agent Identity Connections
    // ============================================

    pub async fn upsert_agent_identity_connection(
        &self,
        input: CreateAgentIdentityConnectionRow,
    ) -> Result<AgentIdentityConnectionRow> {
        dispatch!(self, upsert_agent_identity_connection, input)
    }

    pub async fn get_agent_identity_connection(
        &self,
        identity_id: AgentIdentityId,
        provider: &str,
    ) -> Result<Option<AgentIdentityConnectionRow>> {
        dispatch!(self, get_agent_identity_connection, identity_id, provider)
    }

    pub async fn list_agent_identity_connections(
        &self,
        identity_id: AgentIdentityId,
    ) -> Result<Vec<AgentIdentityConnectionRow>> {
        dispatch!(self, list_agent_identity_connections, identity_id)
    }

    pub async fn delete_agent_identity_connection(
        &self,
        identity_id: AgentIdentityId,
        provider: &str,
    ) -> Result<bool> {
        dispatch!(
            self,
            delete_agent_identity_connection,
            identity_id,
            provider
        )
    }

    // ============================================
    // Session Schedules
    // ============================================

    pub async fn create_session_schedule(
        &self,
        input: CreateSessionScheduleRow,
    ) -> Result<SessionScheduleRow> {
        dispatch!(self, create_session_schedule, input)
    }

    pub async fn get_session_schedule(
        &self,
        org_id: i64,
        schedule_id: ScheduleId,
    ) -> Result<Option<SessionScheduleRow>> {
        dispatch!(self, get_session_schedule, org_id, schedule_id)
    }

    pub async fn list_session_schedules(
        &self,
        org_id: i64,
        session_id: SessionId,
    ) -> Result<Vec<SessionScheduleRow>> {
        dispatch!(self, list_session_schedules, org_id, session_id)
    }

    pub async fn update_session_schedule(
        &self,
        org_id: i64,
        schedule_id: ScheduleId,
        input: UpdateSessionScheduleRow,
    ) -> Result<Option<SessionScheduleRow>> {
        dispatch!(self, update_session_schedule, org_id, schedule_id, input)
    }

    pub async fn delete_session_schedule(
        &self,
        org_id: i64,
        schedule_id: ScheduleId,
    ) -> Result<bool> {
        dispatch!(self, delete_session_schedule, org_id, schedule_id)
    }

    pub async fn create_session_schedule_with_limits(
        &self,
        input: CreateSessionScheduleRow,
        max_per_session: u32,
        max_per_org: i64,
    ) -> Result<Option<SessionScheduleRow>> {
        dispatch!(
            self,
            create_session_schedule_with_limits,
            input,
            max_per_session,
            max_per_org
        )
    }

    pub async fn count_active_session_schedules(&self, session_id: SessionId) -> Result<u32> {
        dispatch!(self, count_active_session_schedules, session_id)
    }

    pub async fn count_active_org_session_schedules(&self, org_id: i64) -> Result<u32> {
        dispatch!(self, count_active_org_session_schedules, org_id)
    }

    pub async fn claim_due_session_schedules(&self, limit: i32) -> Result<Vec<SessionScheduleRow>> {
        dispatch!(self, claim_due_session_schedules, limit)
    }

    // ============================================
    // Leased Resources
    // ============================================

    pub async fn get_session_organization_id(&self, session_id: SessionId) -> Result<Option<i64>> {
        dispatch!(self, get_session_organization_id, session_id)
    }

    // ============================================
    // Cross-Org Resource Resolution
    //
    // Lookup-by-public_id helpers that return the owning org without requiring
    // the caller to know it. Used only by the authenticated
    // GET /v1/resolve-org endpoint, which gates the result by the caller's
    // org memberships to preserve the 404-vs-403 enumeration guarantee.
    // See knowledge/security/multitenancy.md (Cross-Org Resource Resolution).
    // ============================================

    pub async fn get_agent_organization_id(&self, public_id: &str) -> Result<Option<i64>> {
        dispatch!(self, get_agent_organization_id, public_id)
    }

    pub async fn get_harness_organization_id(&self, public_id: &str) -> Result<Option<i64>> {
        dispatch!(self, get_harness_organization_id, public_id)
    }

    pub async fn get_app_organization_id(&self, public_id: &str) -> Result<Option<i64>> {
        dispatch!(self, get_app_organization_id, public_id)
    }

    pub async fn get_skill_organization_id(&self, public_id: &str) -> Result<Option<i64>> {
        dispatch!(self, get_skill_organization_id, public_id)
    }

    pub async fn get_mcp_server_organization_id(&self, public_id: &str) -> Result<Option<i64>> {
        dispatch!(self, get_mcp_server_organization_id, public_id)
    }

    pub async fn get_agent_identity_organization_id(&self, public_id: &str) -> Result<Option<i64>> {
        dispatch!(self, get_agent_identity_organization_id, public_id)
    }

    pub async fn get_eval_organization_id(&self, public_id: &str) -> Result<Option<i64>> {
        dispatch!(self, get_eval_organization_id, public_id)
    }

    pub async fn get_memory_organization_id(&self, public_id: &str) -> Result<Option<i64>> {
        dispatch!(self, get_memory_organization_id, public_id)
    }

    pub async fn upsert_leased_resource(
        &self,
        input: UpsertLeasedResourceRow,
    ) -> Result<LeasedResourceRow> {
        dispatch!(self, upsert_leased_resource, input)
    }

    pub async fn release_leased_resource(
        &self,
        input: ReleaseLeasedResourceRow,
    ) -> Result<Option<LeasedResourceRow>> {
        dispatch!(self, release_leased_resource, input)
    }

    pub async fn list_session_leased_resources(
        &self,
        session_id: SessionId,
    ) -> Result<Vec<LeasedResourceRow>> {
        dispatch!(self, list_session_leased_resources, session_id)
    }

    pub async fn claim_due_leased_resources(
        &self,
        limit: i32,
        stale_after_seconds: i32,
    ) -> Result<Vec<LeasedResourceRow>> {
        dispatch!(self, claim_due_leased_resources, limit, stale_after_seconds)
    }

    pub async fn mark_leased_resource_released(
        &self,
        resource_id: LeasedResourceId,
        expected_cleanup_started_at: DateTime<Utc>,
    ) -> Result<Option<LeasedResourceRow>> {
        dispatch!(
            self,
            mark_leased_resource_released,
            resource_id,
            expected_cleanup_started_at
        )
    }

    pub async fn mark_leased_resource_cleanup_failed(
        &self,
        resource_id: LeasedResourceId,
        expected_cleanup_started_at: DateTime<Utc>,
        retry_after_seconds: i32,
        error: &str,
    ) -> Result<Option<LeasedResourceRow>> {
        dispatch!(
            self,
            mark_leased_resource_cleanup_failed,
            resource_id,
            expected_cleanup_started_at,
            retry_after_seconds,
            error
        )
    }

    // ============================================
    // Session Resource Registry
    // ============================================

    pub async fn upsert_session_resource(
        &self,
        input: UpsertSessionResourceRow,
    ) -> Result<SessionResourceRow> {
        dispatch!(self, upsert_session_resource, input)
    }

    pub async fn update_session_resource_status(
        &self,
        session_id: SessionId,
        resource_id: &str,
        status: &str,
    ) -> Result<Option<SessionResourceRow>> {
        dispatch!(
            self,
            update_session_resource_status,
            session_id,
            resource_id,
            status
        )
    }

    pub async fn get_session_resource(
        &self,
        session_id: SessionId,
        resource_id: &str,
    ) -> Result<Option<SessionResourceRow>> {
        dispatch!(self, get_session_resource, session_id, resource_id)
    }

    pub async fn list_session_resources(
        &self,
        session_id: SessionId,
        kind: Option<&str>,
        status: Option<&str>,
    ) -> Result<Vec<SessionResourceRow>> {
        dispatch!(self, list_session_resources, session_id, kind, status)
    }

    pub async fn delete_session_resource(
        &self,
        session_id: SessionId,
        resource_id: &str,
    ) -> Result<bool> {
        dispatch!(self, delete_session_resource, session_id, resource_id)
    }

    // ============================================
    // Session Tasks
    // ============================================

    /// Insert a task. Idempotent on `id`; the `bool` is true when inserted.
    pub async fn create_session_task(
        &self,
        task: &everruns_core::SessionTask,
    ) -> Result<(SessionTaskRow, bool)> {
        dispatch!(self, create_session_task, task)
    }

    pub async fn get_session_task(
        &self,
        session_id: SessionId,
        task_id: &str,
    ) -> Result<Option<SessionTaskRow>> {
        dispatch!(self, get_session_task, session_id, task_id)
    }

    pub async fn list_session_tasks(
        &self,
        session_id: SessionId,
        kind: Option<&str>,
        state: Option<&str>,
    ) -> Result<Vec<SessionTaskRow>> {
        dispatch!(self, list_session_tasks, session_id, kind, state)
    }

    /// List tasks across every session owned by `org_id`, newest-first, with
    /// optional kind/state/age filters and a bounded limit. Org scoping is the
    /// authoritative multitenancy boundary (a semijoin on `sessions.org_id`):
    /// a task is only returned when its owning session belongs to the org.
    #[allow(clippy::too_many_arguments)]
    pub async fn list_org_session_tasks(
        &self,
        org_id: i64,
        kind: Option<&str>,
        state: Option<&str>,
        created_after: Option<DateTime<Utc>>,
        root_session_id: Option<SessionId>,
        limit: i64,
    ) -> Result<Vec<SessionTaskRow>> {
        dispatch!(
            self,
            list_org_session_tasks,
            org_id,
            kind,
            state,
            created_after,
            root_session_id,
            limit
        )
    }

    pub async fn update_session_task(
        &self,
        session_id: SessionId,
        task_id: &str,
        update: everruns_core::SessionTaskUpdate,
    ) -> Result<Option<SessionTaskRow>> {
        dispatch!(self, update_session_task, session_id, task_id, update)
    }

    pub async fn request_cancel_session_task(
        &self,
        session_id: SessionId,
        task_id: &str,
    ) -> Result<Option<(SessionTaskRow, bool)>> {
        dispatch!(self, request_cancel_session_task, session_id, task_id)
    }

    pub async fn insert_session_task_message(
        &self,
        input: NewSessionTaskMessageRow,
    ) -> Result<SessionTaskMessageRow> {
        dispatch!(self, insert_session_task_message, input)
    }

    // Per-task push-notification configs (EVE-682).

    pub async fn create_task_push_config(
        &self,
        input: crate::storage::models::CreateSessionTaskPushConfig,
    ) -> Result<crate::storage::models::SessionTaskPushConfigRow> {
        dispatch!(self, create_task_push_config, input)
    }

    pub async fn list_task_push_configs(
        &self,
        session_id: SessionId,
        task_id: &str,
    ) -> Result<Vec<crate::storage::models::SessionTaskPushConfigRow>> {
        dispatch!(self, list_task_push_configs, session_id, task_id)
    }

    pub async fn delete_task_push_config(
        &self,
        session_id: SessionId,
        task_id: &str,
        public_id: &str,
    ) -> Result<bool> {
        dispatch!(
            self,
            delete_task_push_config,
            session_id,
            task_id,
            public_id
        )
    }

    pub async fn list_session_task_messages(
        &self,
        session_id: SessionId,
        task_id: &str,
        limit: Option<u32>,
        after_id: Option<&str>,
    ) -> Result<Vec<SessionTaskMessageRow>> {
        dispatch!(
            self,
            list_session_task_messages,
            session_id,
            task_id,
            limit,
            after_id
        )
    }

    /// Return (session_id, task_id, schedule_id) triples for running monitor
    /// tasks whose linked schedule is inactive (missing or enabled=false).
    pub async fn list_monitor_tasks_with_inactive_schedules(
        &self,
        limit: i64,
    ) -> Result<Vec<(SessionId, String, String)>> {
        dispatch!(self, list_monitor_tasks_with_inactive_schedules, limit)
    }

    /// Return (session_id, task_id) pairs for tasks with a stale heartbeat.
    /// See individual backend impls for locking semantics.
    pub async fn list_orphaned_session_task_ids(
        &self,
        stale_after: chrono::Duration,
        limit: i64,
    ) -> Result<Vec<(SessionId, String)>> {
        dispatch!(self, list_orphaned_session_task_ids, stale_after, limit)
    }

    /// Prune a bounded batch of terminal session tasks older than `cutoff`,
    /// returning the `(session_id, task_id, result_path)` triples removed so
    /// the caller can delete their artifacts (EVE-580). Messages are deleted
    /// in both backends (PG via FK cascade, in-memory explicitly).
    pub async fn prune_terminal_session_tasks(
        &self,
        cutoff: chrono::DateTime<chrono::Utc>,
        limit: i64,
    ) -> Result<Vec<(SessionId, String, Option<String>)>> {
        dispatch!(self, prune_terminal_session_tasks, cutoff, limit)
    }

    /// Full retention prune (EVE-580): delete a bounded batch of terminal
    /// session tasks older than `now - ttl` (rows + messages), then remove
    /// each pruned task's recorded internal artifact subtree through the
    /// existing session-file deletion seam (which clears backing blobs for the
    /// object-storage backend). Returns the number of tasks pruned.
    ///
    /// Ordering: rows commit first, artifacts after, so a crash leaks at worst
    /// a dangling blob (reclaimed by blob GC) rather than a row pointing at a
    /// deleted artifact. Artifact deletion is best-effort and never fails the
    /// prune. Shared by the in-process Direct worker adapter and the gRPC
    /// `PruneTerminalSessionTasks` server handler.
    pub async fn prune_terminal_session_tasks_with_artifacts(
        &self,
        ttl: chrono::Duration,
        limit: i64,
    ) -> Result<usize> {
        // Defensive bound on a destructive query. Postgres treats `LIMIT <= 0`
        // (a negative value) as unbounded (`LIMIT ALL`), so a misconfigured or
        // legacy caller passing `limit <= 0` could turn this bounded retention
        // pass into an unlimited delete. Clamp to a positive, capped batch here
        // — the single chokepoint every caller (Direct adapter + gRPC handler)
        // funnels through — regardless of the caller's input. EVE-580 review.
        let limit = limit.clamp(1, MAX_RETENTION_PRUNE_LIMIT);
        let cutoff = chrono::Utc::now() - ttl;
        let pruned = self.prune_terminal_session_tasks(cutoff, limit).await?;

        for (session_id, task_id, result_path) in &pruned {
            let Some(result_path) = result_path.as_deref() else {
                continue;
            };
            let Some(dir) = task_artifact_delete_root(result_path) else {
                tracing::warn!(
                    session_id = %session_id,
                    task_id = %task_id,
                    result_path = %result_path,
                    "Retention prune: skipped non-task artifact result path"
                );
                continue;
            };
            if let Err(e) = self
                .delete_session_file_recursive(session_id.uuid(), dir)
                .await
            {
                tracing::warn!(
                    session_id = %session_id,
                    task_id = %task_id,
                    result_path = %result_path,
                    error = %e,
                    "Retention prune: failed to delete task artifacts (best-effort; blob GC will reclaim)"
                );
            }
        }

        Ok(pruned.len())
    }

    // ============================================
    // Audit Logs (TM-OBS-007, EVE-226)
    // ============================================

    pub async fn create_audit_log(&self, input: CreateAuditLogRow) -> Result<AuditLogRow> {
        dispatch!(self, create_audit_log, input)
    }

    pub async fn list_audit_logs(&self, query: AuditLogQuery<'_>) -> Result<Vec<AuditLogRow>> {
        dispatch!(self, list_audit_logs, query)
    }

    pub async fn delete_audit_logs_before(&self, before: DateTime<Utc>) -> Result<u64> {
        dispatch!(self, delete_audit_logs_before, before)
    }

    // ============================================
    // App CRUD
    // ============================================

    pub async fn create_app(&self, org_id: i64, input: CreateAppRow) -> Result<AppRow> {
        dispatch!(self, create_app, org_id, input)
    }

    pub async fn get_app_by_public_id(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<AppRow>> {
        dispatch!(self, get_app_by_public_id, org_id, public_id)
    }

    pub async fn get_app_by_id(&self, org_id: i64, id: Uuid) -> Result<Option<AppRow>> {
        dispatch!(self, get_app_by_id, org_id, id)
    }

    /// Lookup app by public_id without org scoping (for unauthenticated webhooks).
    pub async fn get_app_by_public_id_unscoped(&self, public_id: &str) -> Result<Option<AppRow>> {
        dispatch!(self, get_app_by_public_id_unscoped, public_id)
    }

    pub async fn list_apps(
        &self,
        org_id: i64,
        search: Option<&str>,
        include_archived: bool,
    ) -> Result<Vec<AppRow>> {
        dispatch!(self, list_apps, org_id, search, include_archived)
    }

    pub async fn count_apps_for_agent(&self, org_id: i64, agent_id: AgentId) -> Result<u64> {
        dispatch!(self, count_apps_for_agent, org_id, agent_id)
    }

    pub async fn count_apps_for_harness(&self, org_id: i64, harness_id: HarnessId) -> Result<u64> {
        dispatch!(self, count_apps_for_harness, org_id, harness_id)
    }

    pub async fn count_apps_for_harnesses(
        &self,
        org_id: i64,
        harness_ids: &[HarnessId],
    ) -> Result<Vec<(HarnessId, i64)>> {
        dispatch!(self, count_apps_for_harnesses, org_id, harness_ids)
    }

    pub async fn update_app(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateApp,
    ) -> Result<Option<AppRow>> {
        dispatch!(self, update_app, org_id, id, input)
    }

    pub async fn delete_app(&self, org_id: i64, id: Uuid) -> Result<bool> {
        dispatch!(self, delete_app, org_id, id)
    }

    pub async fn destroy_app(&self, org_id: i64, id: Uuid) -> Result<bool> {
        dispatch!(self, destroy_app, org_id, id)
    }

    // ============================================
    // App Channel CRUD
    // ============================================

    pub async fn create_app_channel(
        &self,
        app_id: Uuid,
        input: CreateAppChannelRow,
    ) -> Result<AppChannelRow> {
        dispatch!(self, create_app_channel, app_id, input)
    }

    pub async fn create_app_channel_enforcing_schedule_cap(
        &self,
        org_id: i64,
        app_id: Uuid,
        input: CreateAppChannelRow,
        max_enabled_schedule_channels: i64,
    ) -> Result<AppChannelRow> {
        dispatch!(
            self,
            create_app_channel_enforcing_schedule_cap,
            org_id,
            app_id,
            input,
            max_enabled_schedule_channels
        )
    }

    pub async fn list_app_channels(&self, app_id: Uuid) -> Result<Vec<AppChannelRow>> {
        dispatch!(self, list_app_channels, app_id)
    }

    pub async fn app_has_channels(&self, app_id: Uuid) -> Result<bool> {
        dispatch!(self, app_has_channels, app_id)
    }

    pub async fn get_app_channel_by_public_id(
        &self,
        public_id: &str,
    ) -> Result<Option<AppChannelRow>> {
        dispatch!(self, get_app_channel_by_public_id, public_id)
    }

    pub async fn update_app_channel(
        &self,
        id: Uuid,
        input: UpdateAppChannel,
    ) -> Result<Option<AppChannelRow>> {
        dispatch!(self, update_app_channel, id, input)
    }

    pub async fn update_app_channel_enforcing_schedule_cap(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateAppChannel,
        max_enabled_schedule_channels: i64,
    ) -> Result<Option<AppChannelRow>> {
        dispatch!(
            self,
            update_app_channel_enforcing_schedule_cap,
            org_id,
            id,
            input,
            max_enabled_schedule_channels
        )
    }

    pub async fn delete_app_channel(&self, id: Uuid) -> Result<bool> {
        dispatch!(self, delete_app_channel, id)
    }

    pub async fn count_enabled_schedule_channels_for_org(&self, org_id: i64) -> Result<i64> {
        dispatch!(self, count_enabled_schedule_channels_for_org, org_id)
    }

    // ============================================
    // Observers (online scoring — knowledge/evaluation/online-evals.md)
    // ============================================

    pub async fn create_observer(
        &self,
        org_id: i64,
        input: CreateObserverRow,
    ) -> Result<ObserverRow> {
        dispatch!(self, create_observer, org_id, input)
    }

    pub async fn get_observer_by_public_id(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<ObserverRow>> {
        dispatch!(self, get_observer_by_public_id, org_id, public_id)
    }

    pub async fn get_observer(&self, id: Uuid) -> Result<Option<ObserverRow>> {
        dispatch!(self, get_observer, id)
    }

    pub async fn list_observers(
        &self,
        org_id: i64,
        include_archived: bool,
    ) -> Result<Vec<ObserverRow>> {
        dispatch!(self, list_observers, org_id, include_archived)
    }

    pub async fn list_active_observers(&self, org_id: i64) -> Result<Vec<ObserverRow>> {
        dispatch!(self, list_active_observers, org_id)
    }

    pub async fn count_active_observers(&self, org_id: i64) -> Result<i64> {
        dispatch!(self, count_active_observers, org_id)
    }

    pub async fn update_observer(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateObserverRow,
    ) -> Result<Option<ObserverRow>> {
        dispatch!(self, update_observer, org_id, id, input)
    }

    pub async fn delete_observer(&self, org_id: i64, id: Uuid) -> Result<bool> {
        dispatch!(self, delete_observer, org_id, id)
    }

    pub async fn enqueue_trace_scores(
        &self,
        org_id: i64,
        inputs: &[CreateTraceScoreRow],
    ) -> Result<u64> {
        dispatch!(self, enqueue_trace_scores, org_id, inputs)
    }

    pub async fn claim_trace_scores(
        &self,
        limit: i64,
        stale_after_seconds: i64,
        max_attempts: i32,
    ) -> Result<Vec<TraceScoreRow>> {
        dispatch!(
            self,
            claim_trace_scores,
            limit,
            stale_after_seconds,
            max_attempts
        )
    }

    pub async fn complete_trace_score(
        &self,
        id: Uuid,
        input: CompleteTraceScoreRow,
    ) -> Result<Option<TraceScoreRow>> {
        dispatch!(self, complete_trace_score, id, input)
    }

    pub async fn list_trace_scores(
        &self,
        org_id: i64,
        params: ListTraceScoresParams,
    ) -> Result<Vec<TraceScoreRow>> {
        dispatch!(self, list_trace_scores, org_id, params)
    }

    // ============================================
    // Eval CRUD
    // ============================================

    pub async fn create_eval(&self, org_id: i64, input: CreateEvalRow) -> Result<EvalRow> {
        dispatch!(self, create_eval, org_id, input)
    }

    pub async fn get_eval_by_public_id(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<EvalRow>> {
        dispatch!(self, get_eval_by_public_id, org_id, public_id)
    }

    pub async fn list_evals(
        &self,
        org_id: i64,
        search: Option<&str>,
        include_archived: bool,
    ) -> Result<Vec<EvalRow>> {
        dispatch!(self, list_evals, org_id, search, include_archived)
    }

    pub async fn update_eval(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateEvalRow,
    ) -> Result<Option<EvalRow>> {
        dispatch!(self, update_eval, org_id, id, input)
    }

    pub async fn delete_eval(&self, org_id: i64, id: Uuid) -> Result<bool> {
        dispatch!(self, delete_eval, org_id, id)
    }

    // ============================================
    // Eval Case CRUD
    // ============================================

    pub async fn create_eval_case(
        &self,
        eval_id: Uuid,
        input: CreateEvalCaseRow,
    ) -> Result<EvalCaseRow> {
        dispatch!(self, create_eval_case, eval_id, input)
    }

    pub async fn list_eval_cases(&self, eval_id: Uuid) -> Result<Vec<EvalCaseRow>> {
        dispatch!(self, list_eval_cases, eval_id)
    }

    pub async fn get_eval_case(&self, id: Uuid) -> Result<Option<EvalCaseRow>> {
        dispatch!(self, get_eval_case, id)
    }

    pub async fn get_eval_case_by_public_id(
        &self,
        eval_id: Uuid,
        public_id: &str,
    ) -> Result<Option<EvalCaseRow>> {
        dispatch!(self, get_eval_case_by_public_id, eval_id, public_id)
    }

    pub async fn update_eval_case(
        &self,
        id: Uuid,
        input: UpdateEvalCaseRow,
    ) -> Result<Option<EvalCaseRow>> {
        dispatch!(self, update_eval_case, id, input)
    }

    pub async fn delete_eval_case(&self, id: Uuid) -> Result<bool> {
        dispatch!(self, delete_eval_case, id)
    }

    pub async fn count_eval_cases(&self, eval_id: Uuid) -> Result<i64> {
        dispatch!(self, count_eval_cases, eval_id)
    }

    pub async fn count_running_eval_runs_for_org(&self, org_id: i64) -> Result<i64> {
        dispatch!(self, count_running_eval_runs_for_org, org_id)
    }

    // ============================================
    // Eval Run CRUD
    // ============================================

    pub async fn create_eval_run(
        &self,
        org_id: i64,
        input: CreateEvalRunRow,
    ) -> Result<EvalRunRow> {
        dispatch!(self, create_eval_run, org_id, input)
    }

    pub async fn create_eval_run_with_case_results(
        &self,
        org_id: i64,
        input: CreateEvalRunRow,
        eval_target: Option<serde_json::Value>,
        max_concurrent_runs_per_org: usize,
        max_cases_per_run: usize,
    ) -> Result<EvalRunRow> {
        dispatch!(
            self,
            create_eval_run_with_case_results,
            org_id,
            input,
            eval_target,
            max_concurrent_runs_per_org,
            max_cases_per_run
        )
    }

    /// Ingest one externally-executed eval run (upsert eval + cases by name,
    /// replace any prior run sharing `source_run_id`, write a completed external
    /// run with fully-populated results). See `ImportEvalRunInput`.
    pub async fn import_eval_run(
        &self,
        org_id: i64,
        input: ImportEvalRunInput,
    ) -> Result<EvalRunRow> {
        dispatch!(self, import_eval_run, org_id, input)
    }

    pub async fn list_eval_runs(&self, eval_id: Uuid) -> Result<Vec<EvalRunRow>> {
        dispatch!(self, list_eval_runs, eval_id)
    }

    pub async fn get_eval_run_by_public_id(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<EvalRunRow>> {
        dispatch!(self, get_eval_run_by_public_id, org_id, public_id)
    }

    pub async fn get_eval_run_by_id(&self, id: Uuid) -> Result<Option<EvalRunRow>> {
        dispatch!(self, get_eval_run_by_id, id)
    }

    // Eval run share tokens (migration 091)

    pub async fn create_eval_run_share_token(
        &self,
        org_id: i64,
        input: CreateEvalRunShareTokenRow,
    ) -> Result<EvalRunShareTokenRow> {
        dispatch!(self, create_eval_run_share_token, org_id, input)
    }

    pub async fn revoke_eval_run_share_tokens(
        &self,
        org_id: i64,
        eval_run_id: Uuid,
    ) -> Result<u64> {
        dispatch!(self, revoke_eval_run_share_tokens, org_id, eval_run_id)
    }

    pub async fn eval_run_has_active_share(&self, org_id: i64, eval_run_id: Uuid) -> Result<bool> {
        dispatch!(self, eval_run_has_active_share, org_id, eval_run_id)
    }

    pub async fn get_eval_run_share_token_by_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<EvalRunShareTokenRow>> {
        dispatch!(self, get_eval_run_share_token_by_hash, token_hash)
    }

    pub async fn update_eval_run_status(
        &self,
        id: Uuid,
        status: &str,
        summary: Option<serde_json::Value>,
    ) -> Result<Option<EvalRunRow>> {
        dispatch!(self, update_eval_run_status, id, status, summary)
    }

    pub async fn get_latest_eval_run(&self, eval_id: Uuid) -> Result<Option<EvalRunRow>> {
        dispatch!(self, get_latest_eval_run, eval_id)
    }

    // ============================================
    // Agent Health Check Runs (knowledge/evaluation/agent-checks.md)
    // ============================================

    pub async fn create_agent_health_check_run(
        &self,
        org_id: i64,
        input: CreateAgentHealthCheckRunRow,
    ) -> Result<AgentHealthCheckRunRow> {
        dispatch!(self, create_agent_health_check_run, org_id, input)
    }

    pub async fn get_agent_health_check_run(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<AgentHealthCheckRunRow>> {
        dispatch!(self, get_agent_health_check_run, org_id, public_id)
    }

    pub async fn list_agent_health_check_runs(
        &self,
        org_id: i64,
        agent_id: Uuid,
        limit: i64,
    ) -> Result<Vec<AgentHealthCheckRunRow>> {
        dispatch!(self, list_agent_health_check_runs, org_id, agent_id, limit)
    }

    pub async fn latest_agent_health_check_run(
        &self,
        org_id: i64,
        agent_id: Uuid,
        config_hash: &str,
    ) -> Result<Option<AgentHealthCheckRunRow>> {
        dispatch!(
            self,
            latest_agent_health_check_run,
            org_id,
            agent_id,
            config_hash
        )
    }

    pub async fn update_agent_health_check_run(
        &self,
        id: Uuid,
        input: UpdateAgentHealthCheckRunRow,
    ) -> Result<Option<AgentHealthCheckRunRow>> {
        dispatch!(self, update_agent_health_check_run, id, input)
    }

    pub async fn reap_running_agent_health_check_runs(&self) -> Result<u64> {
        dispatch!(self, reap_running_agent_health_check_runs)
    }

    // ============================================
    // Agent Check Rules (knowledge/evaluation/agent-checks.md, phase 4)
    // ============================================

    pub async fn list_agent_check_rules(&self, org_id: i64) -> Result<Vec<AgentCheckRuleRow>> {
        dispatch!(self, list_agent_check_rules, org_id)
    }

    pub async fn count_custom_agent_check_rules_excluding(
        &self,
        org_id: i64,
        excluded_rule_id: &str,
    ) -> Result<i64> {
        dispatch!(
            self,
            count_custom_agent_check_rules_excluding,
            org_id,
            excluded_rule_id
        )
    }

    pub async fn upsert_agent_check_rule(
        &self,
        org_id: i64,
        input: UpsertAgentCheckRuleRow,
    ) -> Result<AgentCheckRuleRow> {
        dispatch!(self, upsert_agent_check_rule, org_id, input)
    }

    pub async fn delete_agent_check_rule(&self, org_id: i64, rule_id: &str) -> Result<bool> {
        dispatch!(self, delete_agent_check_rule, org_id, rule_id)
    }

    // ============================================
    // Eval Case Result CRUD
    // ============================================

    pub async fn create_eval_case_result(
        &self,
        input: CreateEvalCaseResultRow,
    ) -> Result<EvalCaseResultRow> {
        dispatch!(self, create_eval_case_result, input)
    }

    pub async fn list_eval_case_results(
        &self,
        eval_run_id: Uuid,
    ) -> Result<Vec<EvalCaseResultRow>> {
        dispatch!(self, list_eval_case_results, eval_run_id)
    }

    pub async fn update_eval_case_result(
        &self,
        id: Uuid,
        input: UpdateEvalCaseResultRow,
    ) -> Result<Option<EvalCaseResultRow>> {
        dispatch!(self, update_eval_case_result, id, input)
    }

    // ============================================
    // Eval Run Dataset (async export handles — knowledge/evaluation/dataset-export.md)
    // ============================================

    pub async fn create_eval_run_dataset(
        &self,
        org_id: i64,
        input: CreateEvalRunDatasetRow,
    ) -> Result<(EvalRunDatasetRow, bool)> {
        dispatch!(self, create_eval_run_dataset, org_id, input)
    }

    pub async fn find_eval_run_dataset_by_request(
        &self,
        org_id: i64,
        eval_run_id: Uuid,
        request: &serde_json::Value,
    ) -> Result<Option<EvalRunDatasetRow>> {
        dispatch!(
            self,
            find_eval_run_dataset_by_request,
            org_id,
            eval_run_id,
            request
        )
    }

    pub async fn get_eval_run_dataset(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<EvalRunDatasetRow>> {
        dispatch!(self, get_eval_run_dataset, org_id, public_id)
    }

    pub async fn update_eval_run_dataset(
        &self,
        id: Uuid,
        input: UpdateEvalRunDatasetRow,
    ) -> Result<Option<EvalRunDatasetRow>> {
        dispatch!(self, update_eval_run_dataset, id, input)
    }

    // ============================================
    // Budget CRUD
    // ============================================

    pub async fn create_budget(&self, input: CreateBudgetRow) -> Result<BudgetRow> {
        dispatch!(self, create_budget, input)
    }

    pub async fn get_budget(&self, org_id: i64, id: Uuid) -> Result<Option<BudgetRow>> {
        dispatch!(self, get_budget, org_id, id)
    }

    pub async fn list_budgets(
        &self,
        org_id: i64,
        subject_type: Option<&str>,
        subject_id: Option<&str>,
    ) -> Result<Vec<BudgetRow>> {
        dispatch!(self, list_budgets, org_id, subject_type, subject_id)
    }

    pub async fn get_active_budgets_for_session(
        &self,
        org_id: i64,
        session_id: &str,
        agent_id: Option<&str>,
        user_id: Option<&str>,
        org_public_id: Option<&str>,
    ) -> Result<Vec<BudgetRow>> {
        dispatch!(
            self,
            get_active_budgets_for_session,
            org_id,
            session_id,
            agent_id,
            user_id,
            org_public_id
        )
    }

    pub async fn get_active_budgets_for_subjects(
        &self,
        org_id: i64,
        lookup: crate::storage::repositories::BudgetSubjectLookup<'_>,
    ) -> Result<Vec<BudgetRow>> {
        dispatch!(self, get_active_budgets_for_subjects, org_id, lookup)
    }

    pub async fn reset_budget_period(
        &self,
        id: Uuid,
        period_started_at: DateTime<Utc>,
    ) -> Result<Option<BudgetRow>> {
        dispatch!(self, reset_budget_period, id, period_started_at)
    }

    pub async fn update_budget(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateBudgetRow,
    ) -> Result<Option<BudgetRow>> {
        dispatch!(self, update_budget, org_id, id, input)
    }

    pub async fn delete_budget(&self, org_id: i64, id: Uuid) -> Result<bool> {
        dispatch!(self, delete_budget, org_id, id)
    }

    // ============================================
    // Usage Journal + Ledger
    // ============================================

    pub async fn create_usage_journal_entry(
        &self,
        input: CreateUsageJournalRow,
    ) -> Result<UsageJournalRow> {
        dispatch!(self, create_usage_journal_entry, input)
    }

    pub async fn get_usage_journal(&self, id: Uuid) -> Result<Option<UsageJournalRow>> {
        dispatch!(self, get_usage_journal, id)
    }

    pub async fn create_usage_ledger_entry(
        &self,
        input: CreateUsageLedgerRow,
    ) -> Result<(UsageLedgerRow, Option<BudgetRow>)> {
        dispatch!(self, create_usage_ledger_entry, input)
    }

    pub async fn create_budget_ledger_entry(
        &self,
        input: CreateBudgetLedgerRow,
    ) -> Result<(BudgetLedgerRow, BudgetRow)> {
        dispatch!(self, create_budget_ledger_entry, input)
    }

    pub async fn list_usage_ledger_for_budget(
        &self,
        budget_id: Uuid,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<UsageLedgerRow>> {
        dispatch!(self, list_usage_ledger_for_budget, budget_id, limit, offset)
    }

    pub async fn list_budget_ledger(
        &self,
        budget_id: Uuid,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<BudgetLedgerRow>> {
        dispatch!(self, list_budget_ledger, budget_id, limit, offset)
    }

    pub async fn set_budget_status(&self, id: Uuid, status: &str) -> Result<Option<BudgetRow>> {
        dispatch!(self, set_budget_status, id, status)
    }

    // ============================================
    // Machine payments
    // ============================================

    pub async fn create_payment_account(
        &self,
        org_id: i64,
        input: CreatePaymentAccountRow,
    ) -> Result<PaymentAccountRow> {
        dispatch!(self, create_payment_account, org_id, input)
    }

    pub async fn list_payment_accounts(
        &self,
        org_id: i64,
        owner_type: Option<&str>,
        owner_id: Option<&str>,
    ) -> Result<Vec<PaymentAccountRow>> {
        dispatch!(self, list_payment_accounts, org_id, owner_type, owner_id)
    }

    pub async fn get_payment_account(
        &self,
        org_id: i64,
        id: Uuid,
    ) -> Result<Option<PaymentAccountRow>> {
        dispatch!(self, get_payment_account, org_id, id)
    }

    pub async fn update_payment_account(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdatePaymentAccountRow,
    ) -> Result<Option<PaymentAccountRow>> {
        dispatch!(self, update_payment_account, org_id, id, input)
    }

    pub async fn create_payment_policy(
        &self,
        org_id: i64,
        input: CreatePaymentPolicyRow,
    ) -> Result<PaymentPolicyRow> {
        dispatch!(self, create_payment_policy, org_id, input)
    }

    pub async fn list_payment_policies(
        &self,
        org_id: i64,
        payment_account_id: Option<Uuid>,
        subject_type: Option<&str>,
        subject_id: Option<&str>,
    ) -> Result<Vec<PaymentPolicyRow>> {
        dispatch!(
            self,
            list_payment_policies,
            org_id,
            payment_account_id,
            subject_type,
            subject_id
        )
    }

    pub async fn get_payment_policy(
        &self,
        org_id: i64,
        id: Uuid,
    ) -> Result<Option<PaymentPolicyRow>> {
        dispatch!(self, get_payment_policy, org_id, id)
    }

    pub async fn update_payment_policy(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdatePaymentPolicyRow,
    ) -> Result<Option<PaymentPolicyRow>> {
        dispatch!(self, update_payment_policy, org_id, id, input)
    }

    pub async fn list_payment_attempts(
        &self,
        org_id: i64,
        session_id: Option<Uuid>,
        limit: i64,
    ) -> Result<Vec<PaymentAttemptRow>> {
        dispatch!(self, list_payment_attempts, org_id, session_id, limit)
    }

    pub async fn create_payment_attempt(
        &self,
        org_id: i64,
        input: CreatePaymentAttemptRow,
    ) -> Result<PaymentAttemptRow> {
        dispatch!(self, create_payment_attempt, org_id, input)
    }
}

#[cfg(test)]
mod retention_tests {
    use super::*;
    use everruns_core::session_task::{
        CreateSessionTask, SessionTaskRegistry, SessionTaskState, SessionTaskUpdate, TaskLinks,
        TaskWakePolicy,
    };
    use everruns_provider::typed_id::SessionId;
    use std::sync::Arc;

    // The retention prune deletes a task's recorded internal artifact subtree
    // through the existing session-file deletion seam after the row commits
    // (EVE-580). Proven against the in-memory backend: a terminal task with a
    // result_path has its row removed AND its artifact file deleted, while a
    // live task and its file are untouched.
    #[tokio::test]
    async fn prune_with_artifacts_deletes_rows_and_artifact_files() {
        let db = Arc::new(StorageBackend::in_memory());
        let registry = crate::storage::DbSessionTaskRegistry::new(db.clone());
        let session_id = SessionId::new();
        let sid = session_id.uuid();

        // Terminal task with an artifact file under a production background artifact path.
        let terminal = registry
            .create(CreateSessionTask {
                session_id,
                id: Some("task_term".to_string()),
                kind: "background_tool".to_string(),
                display_name: "done".to_string(),
                spec: serde_json::json!({}),
                state: SessionTaskState::Running,
                links: TaskLinks::default(),
                wake_policy: TaskWakePolicy::Silent,
            })
            .await
            .unwrap();
        registry
            .update(
                session_id,
                &terminal.id,
                SessionTaskUpdate {
                    state: Some(SessionTaskState::Succeeded),
                    result_path: Some("/.background/bg_task_term/result.json".to_string()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        db.create_session_file(crate::storage::models::CreateSessionFileRow {
            session_id,
            path: "/.background/bg_task_term/result.json".to_string(),
            content: Some(b"{}".to_vec()),
            is_directory: false,
            is_readonly: false,
        })
        .await
        .unwrap();

        // Live task with a file — must survive.
        registry
            .create(CreateSessionTask {
                session_id,
                id: Some("task_live".to_string()),
                kind: "background_tool".to_string(),
                display_name: "live".to_string(),
                spec: serde_json::json!({}),
                state: SessionTaskState::Running,
                links: TaskLinks::default(),
                wake_policy: TaskWakePolicy::Silent,
            })
            .await
            .unwrap();
        db.create_session_file(crate::storage::models::CreateSessionFileRow {
            session_id,
            path: "/.background/bg_task_live/result.json".to_string(),
            content: Some(b"{}".to_vec()),
            is_directory: false,
            is_readonly: false,
        })
        .await
        .unwrap();

        // Negative TTL → cutoff just after now, so the just-finished terminal
        // task is eligible; the running task can never be (state guard).
        let pruned = db
            .prune_terminal_session_tasks_with_artifacts(chrono::Duration::seconds(-1), 100)
            .await
            .unwrap();
        assert_eq!(pruned, 1, "only the terminal task is pruned");

        // Terminal row + its artifact are gone.
        assert!(
            registry
                .get(session_id, "task_term")
                .await
                .unwrap()
                .is_none(),
            "terminal task row removed"
        );
        assert!(
            db.get_session_file(sid, "/.background/bg_task_term/result.json")
                .await
                .unwrap()
                .is_none(),
            "terminal task artifact deleted via the session-file seam"
        );

        // Live row + its artifact survive.
        assert!(
            registry
                .get(session_id, "task_live")
                .await
                .unwrap()
                .is_some(),
            "live task untouched"
        );
        assert!(
            db.get_session_file(sid, "/.background/bg_task_live/result.json")
                .await
                .unwrap()
                .is_some(),
            "live task artifact untouched"
        );
    }
}

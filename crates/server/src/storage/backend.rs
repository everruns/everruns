// Storage backend abstraction
// Decision: Use enum dispatch for simplicity over trait objects
//
// This module provides a unified StorageBackend enum that can work with
// either PostgreSQL (production) or in-memory (dev mode) storage.

use anyhow::Result;
use chrono::{DateTime, Utc};
use everruns_core::message_filter::MessageQuery;
use everruns_core::typed_id::{
    AgentId, AgentIdentityId, EventId, HarnessId, LeasedResourceId, MessageId, NotificationId,
    ScheduleId, SessionId,
};
use sqlx::PgPool;
use uuid::Uuid;

use super::memory::InMemoryDatabase;
use super::models::*;
use super::repositories::Database;
use crate::api::common::Pagination;

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
    // API Keys
    // ============================================

    pub async fn create_api_key(&self, input: CreateApiKeyRow) -> Result<ApiKeyRow> {
        dispatch!(self, create_api_key, input)
    }

    pub async fn get_api_key_by_hash(&self, key_hash: &str) -> Result<Option<ApiKeyRow>> {
        dispatch!(self, get_api_key_by_hash, key_hash)
    }

    pub async fn list_api_keys_for_user(&self, user_id: Uuid) -> Result<Vec<ApiKeyRow>> {
        dispatch!(self, list_api_keys_for_user, user_id)
    }

    pub async fn update_api_key_last_used(&self, id: Uuid) -> Result<()> {
        dispatch!(self, update_api_key_last_used, id)
    }

    pub async fn delete_api_key(&self, id: Uuid, user_id: Uuid) -> Result<bool> {
        dispatch!(self, delete_api_key, id, user_id)
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

    pub async fn delete_expired_refresh_tokens(&self) -> Result<u64> {
        dispatch!(self, delete_expired_refresh_tokens)
    }

    pub async fn delete_user_refresh_tokens(&self, user_id: Uuid) -> Result<u64> {
        dispatch!(self, delete_user_refresh_tokens, user_id)
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
        dispatch!(self, get_agent, org_id, id)
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
        dispatch!(self, get_agent_public_id, org_id, id)
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
        dispatch!(self, get_harness, org_id, id)
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
    // Sessions
    // ============================================

    pub async fn create_session(&self, input: CreateSessionRow) -> Result<SessionRow> {
        dispatch!(self, create_session, input)
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
        agent_id: Option<AgentId>,
        search: Option<&str>,
        pagination: Pagination,
    ) -> Result<(Vec<SessionRow>, u32)> {
        dispatch!(self, list_sessions, org_id, agent_id, search, pagination)
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

    pub async fn update_session(
        &self,
        org_id: i64,
        id: SessionId,
        input: UpdateSession,
    ) -> Result<Option<SessionRow>> {
        dispatch!(self, update_session, org_id, id, input)
    }

    pub async fn delete_session(&self, org_id: i64, id: SessionId) -> Result<bool> {
        dispatch!(self, delete_session, org_id, id)
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

    pub async fn unpin_session(&self, user_id: Uuid, session_id: SessionId) -> Result<bool> {
        dispatch!(self, unpin_session, user_id, session_id)
    }

    pub async fn list_pinned_session_ids(
        &self,
        user_id: Uuid,
        org_id: i64,
    ) -> Result<Vec<SessionId>> {
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

    /// Get preview text for multiple sessions (first user message)
    pub async fn get_session_previews(
        &self,
        session_ids: &[Uuid],
    ) -> Result<std::collections::HashMap<Uuid, String>> {
        dispatch!(self, get_session_previews, session_ids)
    }

    /// Get output preview text for multiple sessions (last agent message)
    pub async fn get_session_output_previews(
        &self,
        session_ids: &[Uuid],
    ) -> Result<std::collections::HashMap<Uuid, String>> {
        dispatch!(self, get_session_output_previews, session_ids)
    }

    // ============================================
    // LLM Providers
    // ============================================

    pub async fn create_llm_provider(
        &self,
        org_id: i64,
        input: CreateLlmProviderRow,
    ) -> Result<LlmProviderRow> {
        dispatch!(self, create_llm_provider, org_id, input)
    }

    /// Create a provider with a specific ID (for seeding)
    /// Returns None if provider already exists (idempotent)
    pub async fn create_llm_provider_with_id(
        &self,
        org_id: i64,
        id: Uuid,
        input: CreateLlmProviderRow,
    ) -> Result<Option<LlmProviderRow>> {
        dispatch!(self, create_llm_provider_with_id, org_id, id, input)
    }

    pub async fn get_llm_provider(&self, org_id: i64, id: Uuid) -> Result<Option<LlmProviderRow>> {
        dispatch!(self, get_llm_provider, org_id, id)
    }

    pub async fn list_llm_providers(&self, org_id: i64) -> Result<Vec<LlmProviderRow>> {
        dispatch!(self, list_llm_providers, org_id)
    }

    pub async fn update_llm_provider(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateLlmProvider,
    ) -> Result<Option<LlmProviderRow>> {
        dispatch!(self, update_llm_provider, org_id, id, input)
    }

    pub async fn delete_llm_provider(&self, org_id: i64, id: Uuid) -> Result<bool> {
        dispatch!(self, delete_llm_provider, org_id, id)
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
        provider: &LlmProviderRow,
        encryption: &super::EncryptionService,
    ) -> Result<LlmProviderWithApiKey> {
        match self {
            Self::Postgres(db) => db.get_provider_with_api_key(provider, encryption),
            Self::InMemory(db) => db.get_provider_with_api_key(provider, encryption),
        }
    }

    // ============================================
    // LLM Models
    // ============================================

    pub async fn get_default_llm_model(
        &self,
        org_id: i64,
    ) -> Result<Option<LlmModelWithProviderRow>> {
        dispatch!(self, get_default_llm_model, org_id)
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

    pub async fn create_llm_model(
        &self,
        org_id: i64,
        input: CreateLlmModelRow,
    ) -> Result<LlmModelRow> {
        dispatch!(self, create_llm_model, org_id, input)
    }

    /// Create a model with a specific ID (for seeding)
    /// Returns None if model already exists (idempotent)
    pub async fn create_llm_model_with_id(
        &self,
        org_id: i64,
        id: Uuid,
        input: CreateLlmModelRow,
    ) -> Result<Option<LlmModelRow>> {
        dispatch!(self, create_llm_model_with_id, org_id, id, input)
    }

    pub async fn get_llm_model(&self, org_id: i64, id: Uuid) -> Result<Option<LlmModelRow>> {
        dispatch!(self, get_llm_model, org_id, id)
    }

    pub async fn get_llm_model_with_provider(
        &self,
        org_id: i64,
        id: Uuid,
    ) -> Result<Option<LlmModelWithProviderRow>> {
        dispatch!(self, get_llm_model_with_provider, org_id, id)
    }

    pub async fn list_llm_models_for_provider(
        &self,
        org_id: i64,
        provider_id: Uuid,
    ) -> Result<Vec<LlmModelRow>> {
        dispatch!(self, list_llm_models_for_provider, org_id, provider_id)
    }

    pub async fn list_all_llm_models(&self, org_id: i64) -> Result<Vec<LlmModelWithProviderRow>> {
        dispatch!(self, list_all_llm_models, org_id)
    }

    pub async fn update_llm_model(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateLlmModel,
    ) -> Result<Option<LlmModelRow>> {
        dispatch!(self, update_llm_model, org_id, id, input)
    }

    pub async fn delete_llm_model(&self, org_id: i64, id: Uuid) -> Result<bool> {
        dispatch!(self, delete_llm_model, org_id, id)
    }

    pub async fn get_llm_model_by_model_id(
        &self,
        org_id: i64,
        model_id: &str,
    ) -> Result<Option<LlmModelWithProviderRow>> {
        dispatch!(self, get_llm_model_by_model_id, org_id, model_id)
    }

    // ============================================
    // Agent Capabilities
    // ============================================

    pub async fn get_agent_capabilities(&self, agent_id: Uuid) -> Result<Vec<AgentCapabilityRow>> {
        dispatch!(self, get_agent_capabilities, agent_id)
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
        dispatch!(self, get_harness_capabilities, harness_id)
    }

    pub async fn set_harness_capabilities(
        &self,
        harness_id: Uuid,
        capabilities: Vec<(String, i32, serde_json::Value)>,
    ) -> Result<Vec<HarnessCapabilityRow>> {
        dispatch!(self, set_harness_capabilities, harness_id, capabilities)
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

    pub async fn get_session_file_by_id(&self, id: Uuid) -> Result<Option<SessionFileRow>> {
        dispatch!(self, get_session_file_by_id, id)
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
    ) -> Result<Vec<SessionFileInfoRow>> {
        dispatch!(self, grep_session_files, session_id, pattern, path_prefix)
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
        session_id: Uuid,
        turn_id: Option<Uuid>,
        event_id: Option<Uuid>,
        model: String,
        provider: Option<String>,
        input_tokens: i64,
        output_tokens: i64,
        cache_read_tokens: i64,
        cache_creation_tokens: i64,
        duration_ms: Option<i32>,
        finish_reason: Option<String>,
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
            duration_ms,
            finish_reason,
            created_at
        )
    }

    pub async fn increment_session_usage(
        &self,
        session_id: Uuid,
        input_tokens: i64,
        output_tokens: i64,
        cache_read_tokens: i64,
        cache_creation_tokens: i64,
    ) -> Result<()> {
        dispatch!(
            self,
            increment_session_usage,
            session_id,
            input_tokens,
            output_tokens,
            cache_read_tokens,
            cache_creation_tokens
        )
    }

    pub async fn increment_agent_usage(
        &self,
        agent_id: Uuid,
        input_tokens: i64,
        output_tokens: i64,
        cache_read_tokens: i64,
        cache_creation_tokens: i64,
    ) -> Result<()> {
        dispatch!(
            self,
            increment_agent_usage,
            agent_id,
            input_tokens,
            output_tokens,
            cache_read_tokens,
            cache_creation_tokens
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

    pub async fn count_api_keys_for_user_in_org(&self, user_id: Uuid, org_id: i64) -> Result<i64> {
        dispatch!(self, count_api_keys_for_user_in_org, user_id, org_id)
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
    // Session Storage (Key-Value & Secrets)
    // ============================================

    pub async fn list_session_keys(&self, session_id: Uuid) -> Result<Vec<SessionKeyInfoRow>> {
        dispatch!(self, list_session_keys, session_id)
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

    pub async fn upsert_session_secret(
        &self,
        input: UpsertSessionSecret,
    ) -> Result<SessionSecretRow> {
        dispatch!(self, upsert_session_secret, input)
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

    pub async fn delete_user_connection(&self, user_id: Uuid, provider: &str) -> Result<bool> {
        dispatch!(self, delete_user_connection, user_id, provider)
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

    pub async fn count_active_session_schedules(&self, session_id: SessionId) -> Result<u32> {
        dispatch!(self, count_active_session_schedules, session_id)
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

    pub async fn list_app_channels(&self, app_id: Uuid) -> Result<Vec<AppChannelRow>> {
        dispatch!(self, list_app_channels, app_id)
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

    pub async fn delete_app_channel(&self, id: Uuid) -> Result<bool> {
        dispatch!(self, delete_app_channel, id)
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
    // Budget Ledger
    // ============================================

    pub async fn create_budget_ledger_entry(
        &self,
        input: CreateBudgetLedgerRow,
    ) -> Result<(BudgetLedgerRow, BudgetRow)> {
        dispatch!(self, create_budget_ledger_entry, input)
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
}

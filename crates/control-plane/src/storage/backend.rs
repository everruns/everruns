// Storage backend abstraction
// Decision: Use enum dispatch for simplicity over trait objects
//
// This module provides a unified StorageBackend enum that can work with
// either PostgreSQL (production) or in-memory (dev mode) storage.

use anyhow::Result;
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
    // Agents
    // ============================================

    pub async fn create_agent(&self, input: CreateAgentRow) -> Result<AgentRow> {
        dispatch!(self, create_agent, input)
    }

    pub async fn create_agent_with_id(
        &self,
        id: Uuid,
        input: CreateAgentRow,
    ) -> Result<Option<AgentRow>> {
        dispatch!(self, create_agent_with_id, id, input)
    }

    pub async fn get_agent(&self, id: Uuid) -> Result<Option<AgentRow>> {
        dispatch!(self, get_agent, id)
    }

    pub async fn list_agents(&self) -> Result<Vec<AgentRow>> {
        dispatch!(self, list_agents)
    }

    pub async fn get_agent_by_name(&self, name: &str) -> Result<Option<AgentRow>> {
        dispatch!(self, get_agent_by_name, name)
    }

    pub async fn update_agent(&self, id: Uuid, input: UpdateAgent) -> Result<Option<AgentRow>> {
        dispatch!(self, update_agent, id, input)
    }

    pub async fn delete_agent(&self, id: Uuid) -> Result<bool> {
        dispatch!(self, delete_agent, id)
    }

    // ============================================
    // Sessions
    // ============================================

    pub async fn create_session(&self, input: CreateSessionRow) -> Result<SessionRow> {
        dispatch!(self, create_session, input)
    }

    pub async fn get_session(&self, id: Uuid) -> Result<Option<SessionRow>> {
        dispatch!(self, get_session, id)
    }

    /// List sessions for an agent with pagination.
    /// Returns (sessions, total_count).
    pub async fn list_sessions(
        &self,
        agent_id: Uuid,
        pagination: Pagination,
    ) -> Result<(Vec<SessionRow>, u32)> {
        dispatch!(self, list_sessions, agent_id, pagination)
    }

    pub async fn update_session(
        &self,
        id: Uuid,
        input: UpdateSession,
    ) -> Result<Option<SessionRow>> {
        dispatch!(self, update_session, id, input)
    }

    pub async fn delete_session(&self, id: Uuid) -> Result<bool> {
        dispatch!(self, delete_session, id)
    }

    // ============================================
    // Events
    // ============================================

    pub async fn create_event(&self, input: CreateEventRow) -> Result<EventRow> {
        dispatch!(self, create_event, input)
    }

    pub async fn list_events(
        &self,
        session_id: Uuid,
        since_sequence: Option<i32>,
        since_id: Option<Uuid>,
    ) -> Result<Vec<EventRow>> {
        dispatch!(self, list_events, session_id, since_sequence, since_id)
    }

    pub async fn list_message_events(&self, session_id: Uuid) -> Result<Vec<EventRow>> {
        dispatch!(self, list_message_events, session_id)
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

    pub async fn create_llm_provider(&self, input: CreateLlmProviderRow) -> Result<LlmProviderRow> {
        dispatch!(self, create_llm_provider, input)
    }

    /// Create a provider with a specific ID (for seeding)
    /// Returns None if provider already exists (idempotent)
    pub async fn create_llm_provider_with_id(
        &self,
        id: Uuid,
        input: CreateLlmProviderRow,
    ) -> Result<Option<LlmProviderRow>> {
        dispatch!(self, create_llm_provider_with_id, id, input)
    }

    pub async fn get_llm_provider(&self, id: Uuid) -> Result<Option<LlmProviderRow>> {
        dispatch!(self, get_llm_provider, id)
    }

    pub async fn list_llm_providers(&self) -> Result<Vec<LlmProviderRow>> {
        dispatch!(self, list_llm_providers)
    }

    pub async fn update_llm_provider(
        &self,
        id: Uuid,
        input: UpdateLlmProvider,
    ) -> Result<Option<LlmProviderRow>> {
        dispatch!(self, update_llm_provider, id, input)
    }

    pub async fn delete_llm_provider(&self, id: Uuid) -> Result<bool> {
        dispatch!(self, delete_llm_provider, id)
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

    pub async fn get_default_llm_model(&self) -> Result<Option<LlmModelWithProviderRow>> {
        dispatch!(self, get_default_llm_model)
    }

    pub async fn clear_all_model_defaults(&self) -> Result<()> {
        dispatch!(self, clear_all_model_defaults)
    }

    pub async fn create_llm_model(&self, input: CreateLlmModelRow) -> Result<LlmModelRow> {
        dispatch!(self, create_llm_model, input)
    }

    /// Create a model with a specific ID (for seeding)
    /// Returns None if model already exists (idempotent)
    pub async fn create_llm_model_with_id(
        &self,
        id: Uuid,
        input: CreateLlmModelRow,
    ) -> Result<Option<LlmModelRow>> {
        dispatch!(self, create_llm_model_with_id, id, input)
    }

    pub async fn get_llm_model(&self, id: Uuid) -> Result<Option<LlmModelRow>> {
        dispatch!(self, get_llm_model, id)
    }

    pub async fn get_llm_model_with_provider(
        &self,
        id: Uuid,
    ) -> Result<Option<LlmModelWithProviderRow>> {
        dispatch!(self, get_llm_model_with_provider, id)
    }

    pub async fn list_llm_models_for_provider(
        &self,
        provider_id: Uuid,
    ) -> Result<Vec<LlmModelRow>> {
        dispatch!(self, list_llm_models_for_provider, provider_id)
    }

    pub async fn list_all_llm_models(&self) -> Result<Vec<LlmModelWithProviderRow>> {
        dispatch!(self, list_all_llm_models)
    }

    pub async fn update_llm_model(
        &self,
        id: Uuid,
        input: UpdateLlmModel,
    ) -> Result<Option<LlmModelRow>> {
        dispatch!(self, update_llm_model, id, input)
    }

    pub async fn delete_llm_model(&self, id: Uuid) -> Result<bool> {
        dispatch!(self, delete_llm_model, id)
    }

    pub async fn get_llm_model_by_model_id(
        &self,
        model_id: &str,
    ) -> Result<Option<LlmModelWithProviderRow>> {
        dispatch!(self, get_llm_model_by_model_id, model_id)
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

    pub async fn session_directory_has_children(
        &self,
        session_id: Uuid,
        path: &str,
    ) -> Result<bool> {
        dispatch!(self, session_directory_has_children, session_id, path)
    }

    // ============================================
    // MCP Servers
    // ============================================

    pub async fn create_mcp_server(&self, input: CreateMcpServerRow) -> Result<McpServerRow> {
        dispatch!(self, create_mcp_server, input)
    }

    pub async fn get_mcp_server(&self, id: Uuid) -> Result<Option<McpServerRow>> {
        dispatch!(self, get_mcp_server, id)
    }

    pub async fn get_mcp_server_by_name(&self, name: &str) -> Result<Option<McpServerRow>> {
        dispatch!(self, get_mcp_server_by_name, name)
    }

    pub async fn list_mcp_servers(&self) -> Result<Vec<McpServerRow>> {
        dispatch!(self, list_mcp_servers)
    }

    pub async fn update_mcp_server(
        &self,
        id: Uuid,
        input: UpdateMcpServer,
    ) -> Result<Option<McpServerRow>> {
        dispatch!(self, update_mcp_server, id, input)
    }

    pub async fn delete_mcp_server(&self, id: Uuid) -> Result<bool> {
        dispatch!(self, delete_mcp_server, id)
    }

    // ============================================
    // LLM Generations (Usage Tracking)
    // ============================================

    #[allow(clippy::too_many_arguments)]
    pub async fn create_llm_generation(
        &self,
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
}

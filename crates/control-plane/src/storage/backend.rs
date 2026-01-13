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
        match self {
            Self::Postgres(db) => db.create_user(input).await,
            Self::InMemory(db) => db.create_user(input).await,
        }
    }

    pub async fn get_user_by_email(&self, email: &str) -> Result<Option<UserRow>> {
        match self {
            Self::Postgres(db) => db.get_user_by_email(email).await,
            Self::InMemory(db) => db.get_user_by_email(email).await,
        }
    }

    pub async fn get_user(&self, id: Uuid) -> Result<Option<UserRow>> {
        match self {
            Self::Postgres(db) => db.get_user(id).await,
            Self::InMemory(db) => db.get_user(id).await,
        }
    }

    pub async fn get_user_by_oauth(
        &self,
        provider: &str,
        provider_id: &str,
    ) -> Result<Option<UserRow>> {
        match self {
            Self::Postgres(db) => db.get_user_by_oauth(provider, provider_id).await,
            Self::InMemory(db) => db.get_user_by_oauth(provider, provider_id).await,
        }
    }

    pub async fn update_user(&self, id: Uuid, input: UpdateUser) -> Result<Option<UserRow>> {
        match self {
            Self::Postgres(db) => db.update_user(id, input).await,
            Self::InMemory(db) => db.update_user(id, input).await,
        }
    }

    pub async fn list_users(&self, search: Option<&str>) -> Result<Vec<UserRow>> {
        match self {
            Self::Postgres(db) => db.list_users(search).await,
            Self::InMemory(db) => db.list_users(search).await,
        }
    }

    // ============================================
    // API Keys
    // ============================================

    pub async fn create_api_key(&self, input: CreateApiKeyRow) -> Result<ApiKeyRow> {
        match self {
            Self::Postgres(db) => db.create_api_key(input).await,
            Self::InMemory(db) => db.create_api_key(input).await,
        }
    }

    pub async fn get_api_key_by_hash(&self, key_hash: &str) -> Result<Option<ApiKeyRow>> {
        match self {
            Self::Postgres(db) => db.get_api_key_by_hash(key_hash).await,
            Self::InMemory(db) => db.get_api_key_by_hash(key_hash).await,
        }
    }

    pub async fn list_api_keys_for_user(&self, user_id: Uuid) -> Result<Vec<ApiKeyRow>> {
        match self {
            Self::Postgres(db) => db.list_api_keys_for_user(user_id).await,
            Self::InMemory(db) => db.list_api_keys_for_user(user_id).await,
        }
    }

    pub async fn update_api_key_last_used(&self, id: Uuid) -> Result<()> {
        match self {
            Self::Postgres(db) => db.update_api_key_last_used(id).await,
            Self::InMemory(db) => db.update_api_key_last_used(id).await,
        }
    }

    pub async fn delete_api_key(&self, id: Uuid, user_id: Uuid) -> Result<bool> {
        match self {
            Self::Postgres(db) => db.delete_api_key(id, user_id).await,
            Self::InMemory(db) => db.delete_api_key(id, user_id).await,
        }
    }

    // ============================================
    // Refresh Tokens
    // ============================================

    pub async fn create_refresh_token(
        &self,
        input: CreateRefreshTokenRow,
    ) -> Result<RefreshTokenRow> {
        match self {
            Self::Postgres(db) => db.create_refresh_token(input).await,
            Self::InMemory(db) => db.create_refresh_token(input).await,
        }
    }

    pub async fn get_refresh_token_by_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<RefreshTokenRow>> {
        match self {
            Self::Postgres(db) => db.get_refresh_token_by_hash(token_hash).await,
            Self::InMemory(db) => db.get_refresh_token_by_hash(token_hash).await,
        }
    }

    pub async fn delete_refresh_token(&self, id: Uuid) -> Result<bool> {
        match self {
            Self::Postgres(db) => db.delete_refresh_token(id).await,
            Self::InMemory(db) => db.delete_refresh_token(id).await,
        }
    }

    pub async fn delete_expired_refresh_tokens(&self) -> Result<u64> {
        match self {
            Self::Postgres(db) => db.delete_expired_refresh_tokens().await,
            Self::InMemory(db) => db.delete_expired_refresh_tokens().await,
        }
    }

    pub async fn delete_user_refresh_tokens(&self, user_id: Uuid) -> Result<u64> {
        match self {
            Self::Postgres(db) => db.delete_user_refresh_tokens(user_id).await,
            Self::InMemory(db) => db.delete_user_refresh_tokens(user_id).await,
        }
    }

    // ============================================
    // Agents
    // ============================================

    pub async fn create_agent(&self, input: CreateAgentRow) -> Result<AgentRow> {
        match self {
            Self::Postgres(db) => db.create_agent(input).await,
            Self::InMemory(db) => db.create_agent(input).await,
        }
    }

    pub async fn get_agent(&self, id: Uuid) -> Result<Option<AgentRow>> {
        match self {
            Self::Postgres(db) => db.get_agent(id).await,
            Self::InMemory(db) => db.get_agent(id).await,
        }
    }

    pub async fn list_agents(&self) -> Result<Vec<AgentRow>> {
        match self {
            Self::Postgres(db) => db.list_agents().await,
            Self::InMemory(db) => db.list_agents().await,
        }
    }

    pub async fn update_agent(&self, id: Uuid, input: UpdateAgent) -> Result<Option<AgentRow>> {
        match self {
            Self::Postgres(db) => db.update_agent(id, input).await,
            Self::InMemory(db) => db.update_agent(id, input).await,
        }
    }

    pub async fn delete_agent(&self, id: Uuid) -> Result<bool> {
        match self {
            Self::Postgres(db) => db.delete_agent(id).await,
            Self::InMemory(db) => db.delete_agent(id).await,
        }
    }

    // ============================================
    // Sessions
    // ============================================

    pub async fn create_session(&self, input: CreateSessionRow) -> Result<SessionRow> {
        match self {
            Self::Postgres(db) => db.create_session(input).await,
            Self::InMemory(db) => db.create_session(input).await,
        }
    }

    pub async fn get_session(&self, id: Uuid) -> Result<Option<SessionRow>> {
        match self {
            Self::Postgres(db) => db.get_session(id).await,
            Self::InMemory(db) => db.get_session(id).await,
        }
    }

    pub async fn list_sessions(&self, agent_id: Uuid) -> Result<Vec<SessionRow>> {
        match self {
            Self::Postgres(db) => db.list_sessions(agent_id).await,
            Self::InMemory(db) => db.list_sessions(agent_id).await,
        }
    }

    pub async fn update_session(
        &self,
        id: Uuid,
        input: UpdateSession,
    ) -> Result<Option<SessionRow>> {
        match self {
            Self::Postgres(db) => db.update_session(id, input).await,
            Self::InMemory(db) => db.update_session(id, input).await,
        }
    }

    pub async fn delete_session(&self, id: Uuid) -> Result<bool> {
        match self {
            Self::Postgres(db) => db.delete_session(id).await,
            Self::InMemory(db) => db.delete_session(id).await,
        }
    }

    // ============================================
    // Events
    // ============================================

    pub async fn create_event(&self, input: CreateEventRow) -> Result<EventRow> {
        match self {
            Self::Postgres(db) => db.create_event(input).await,
            Self::InMemory(db) => db.create_event(input).await,
        }
    }

    pub async fn list_events(
        &self,
        session_id: Uuid,
        since_sequence: Option<i32>,
        since_id: Option<Uuid>,
    ) -> Result<Vec<EventRow>> {
        match self {
            Self::Postgres(db) => db.list_events(session_id, since_sequence, since_id).await,
            Self::InMemory(db) => db.list_events(session_id, since_sequence, since_id).await,
        }
    }

    pub async fn list_message_events(&self, session_id: Uuid) -> Result<Vec<EventRow>> {
        match self {
            Self::Postgres(db) => db.list_message_events(session_id).await,
            Self::InMemory(db) => db.list_message_events(session_id).await,
        }
    }

    // ============================================
    // LLM Providers
    // ============================================

    pub async fn create_llm_provider(&self, input: CreateLlmProviderRow) -> Result<LlmProviderRow> {
        match self {
            Self::Postgres(db) => db.create_llm_provider(input).await,
            Self::InMemory(db) => db.create_llm_provider(input).await,
        }
    }

    pub async fn get_llm_provider(&self, id: Uuid) -> Result<Option<LlmProviderRow>> {
        match self {
            Self::Postgres(db) => db.get_llm_provider(id).await,
            Self::InMemory(db) => db.get_llm_provider(id).await,
        }
    }

    pub async fn list_llm_providers(&self) -> Result<Vec<LlmProviderRow>> {
        match self {
            Self::Postgres(db) => db.list_llm_providers().await,
            Self::InMemory(db) => db.list_llm_providers().await,
        }
    }

    pub async fn update_llm_provider(
        &self,
        id: Uuid,
        input: UpdateLlmProvider,
    ) -> Result<Option<LlmProviderRow>> {
        match self {
            Self::Postgres(db) => db.update_llm_provider(id, input).await,
            Self::InMemory(db) => db.update_llm_provider(id, input).await,
        }
    }

    pub async fn delete_llm_provider(&self, id: Uuid) -> Result<bool> {
        match self {
            Self::Postgres(db) => db.delete_llm_provider(id).await,
            Self::InMemory(db) => db.delete_llm_provider(id).await,
        }
    }

    /// Get a provider with its decrypted API key
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
        match self {
            Self::Postgres(db) => db.get_default_llm_model().await,
            Self::InMemory(db) => db.get_default_llm_model().await,
        }
    }

    pub async fn clear_all_model_defaults(&self) -> Result<()> {
        match self {
            Self::Postgres(db) => db.clear_all_model_defaults().await,
            Self::InMemory(db) => db.clear_all_model_defaults().await,
        }
    }

    pub async fn create_llm_model(&self, input: CreateLlmModelRow) -> Result<LlmModelRow> {
        match self {
            Self::Postgres(db) => db.create_llm_model(input).await,
            Self::InMemory(db) => db.create_llm_model(input).await,
        }
    }

    pub async fn get_llm_model(&self, id: Uuid) -> Result<Option<LlmModelRow>> {
        match self {
            Self::Postgres(db) => db.get_llm_model(id).await,
            Self::InMemory(db) => db.get_llm_model(id).await,
        }
    }

    pub async fn get_llm_model_with_provider(
        &self,
        id: Uuid,
    ) -> Result<Option<LlmModelWithProviderRow>> {
        match self {
            Self::Postgres(db) => db.get_llm_model_with_provider(id).await,
            Self::InMemory(db) => db.get_llm_model_with_provider(id).await,
        }
    }

    pub async fn list_llm_models_for_provider(
        &self,
        provider_id: Uuid,
    ) -> Result<Vec<LlmModelRow>> {
        match self {
            Self::Postgres(db) => db.list_llm_models_for_provider(provider_id).await,
            Self::InMemory(db) => db.list_llm_models_for_provider(provider_id).await,
        }
    }

    pub async fn list_all_llm_models(&self) -> Result<Vec<LlmModelWithProviderRow>> {
        match self {
            Self::Postgres(db) => db.list_all_llm_models().await,
            Self::InMemory(db) => db.list_all_llm_models().await,
        }
    }

    pub async fn update_llm_model(
        &self,
        id: Uuid,
        input: UpdateLlmModel,
    ) -> Result<Option<LlmModelRow>> {
        match self {
            Self::Postgres(db) => db.update_llm_model(id, input).await,
            Self::InMemory(db) => db.update_llm_model(id, input).await,
        }
    }

    pub async fn delete_llm_model(&self, id: Uuid) -> Result<bool> {
        match self {
            Self::Postgres(db) => db.delete_llm_model(id).await,
            Self::InMemory(db) => db.delete_llm_model(id).await,
        }
    }

    pub async fn get_llm_model_by_model_id(
        &self,
        model_id: &str,
    ) -> Result<Option<LlmModelWithProviderRow>> {
        match self {
            Self::Postgres(db) => db.get_llm_model_by_model_id(model_id).await,
            Self::InMemory(db) => db.get_llm_model_by_model_id(model_id).await,
        }
    }

    // ============================================
    // Agent Capabilities
    // ============================================

    pub async fn get_agent_capabilities(&self, agent_id: Uuid) -> Result<Vec<AgentCapabilityRow>> {
        match self {
            Self::Postgres(db) => db.get_agent_capabilities(agent_id).await,
            Self::InMemory(db) => db.get_agent_capabilities(agent_id).await,
        }
    }

    pub async fn set_agent_capabilities(
        &self,
        agent_id: Uuid,
        capabilities: Vec<(String, i32)>,
    ) -> Result<Vec<AgentCapabilityRow>> {
        match self {
            Self::Postgres(db) => db.set_agent_capabilities(agent_id, capabilities).await,
            Self::InMemory(db) => db.set_agent_capabilities(agent_id, capabilities).await,
        }
    }

    pub async fn add_agent_capability(
        &self,
        input: CreateAgentCapabilityRow,
    ) -> Result<AgentCapabilityRow> {
        match self {
            Self::Postgres(db) => db.add_agent_capability(input).await,
            Self::InMemory(db) => db.add_agent_capability(input).await,
        }
    }

    pub async fn remove_agent_capability(
        &self,
        agent_id: Uuid,
        capability_id: &str,
    ) -> Result<bool> {
        match self {
            Self::Postgres(db) => db.remove_agent_capability(agent_id, capability_id).await,
            Self::InMemory(db) => db.remove_agent_capability(agent_id, capability_id).await,
        }
    }

    // ============================================
    // Session Files
    // ============================================

    pub async fn create_session_file(&self, input: CreateSessionFileRow) -> Result<SessionFileRow> {
        match self {
            Self::Postgres(db) => db.create_session_file(input).await,
            Self::InMemory(db) => db.create_session_file(input).await,
        }
    }

    pub async fn get_session_file(
        &self,
        session_id: Uuid,
        path: &str,
    ) -> Result<Option<SessionFileRow>> {
        match self {
            Self::Postgres(db) => db.get_session_file(session_id, path).await,
            Self::InMemory(db) => db.get_session_file(session_id, path).await,
        }
    }

    pub async fn get_session_file_by_id(&self, id: Uuid) -> Result<Option<SessionFileRow>> {
        match self {
            Self::Postgres(db) => db.get_session_file_by_id(id).await,
            Self::InMemory(db) => db.get_session_file_by_id(id).await,
        }
    }

    pub async fn list_session_files(
        &self,
        session_id: Uuid,
        parent_path: &str,
    ) -> Result<Vec<SessionFileInfoRow>> {
        match self {
            Self::Postgres(db) => db.list_session_files(session_id, parent_path).await,
            Self::InMemory(db) => db.list_session_files(session_id, parent_path).await,
        }
    }

    pub async fn list_all_session_files(
        &self,
        session_id: Uuid,
    ) -> Result<Vec<SessionFileInfoRow>> {
        match self {
            Self::Postgres(db) => db.list_all_session_files(session_id).await,
            Self::InMemory(db) => db.list_all_session_files(session_id).await,
        }
    }

    pub async fn update_session_file(
        &self,
        session_id: Uuid,
        path: &str,
        input: UpdateSessionFile,
    ) -> Result<Option<SessionFileRow>> {
        match self {
            Self::Postgres(db) => db.update_session_file(session_id, path, input).await,
            Self::InMemory(db) => db.update_session_file(session_id, path, input).await,
        }
    }

    pub async fn delete_session_file(&self, session_id: Uuid, path: &str) -> Result<bool> {
        match self {
            Self::Postgres(db) => db.delete_session_file(session_id, path).await,
            Self::InMemory(db) => db.delete_session_file(session_id, path).await,
        }
    }

    pub async fn delete_session_file_recursive(&self, session_id: Uuid, path: &str) -> Result<u64> {
        match self {
            Self::Postgres(db) => db.delete_session_file_recursive(session_id, path).await,
            Self::InMemory(db) => db.delete_session_file_recursive(session_id, path).await,
        }
    }

    pub async fn move_session_file(
        &self,
        session_id: Uuid,
        source_path: &str,
        dest_path: &str,
    ) -> Result<Option<SessionFileRow>> {
        match self {
            Self::Postgres(db) => {
                db.move_session_file(session_id, source_path, dest_path)
                    .await
            }
            Self::InMemory(db) => {
                db.move_session_file(session_id, source_path, dest_path)
                    .await
            }
        }
    }

    pub async fn copy_session_file(
        &self,
        session_id: Uuid,
        source_path: &str,
        dest_path: &str,
    ) -> Result<Option<SessionFileRow>> {
        match self {
            Self::Postgres(db) => {
                db.copy_session_file(session_id, source_path, dest_path)
                    .await
            }
            Self::InMemory(db) => {
                db.copy_session_file(session_id, source_path, dest_path)
                    .await
            }
        }
    }

    pub async fn grep_session_files(
        &self,
        session_id: Uuid,
        pattern: &str,
        path_prefix: Option<&str>,
    ) -> Result<Vec<SessionFileInfoRow>> {
        match self {
            Self::Postgres(db) => {
                db.grep_session_files(session_id, pattern, path_prefix)
                    .await
            }
            Self::InMemory(db) => {
                db.grep_session_files(session_id, pattern, path_prefix)
                    .await
            }
        }
    }

    pub async fn session_file_exists(&self, session_id: Uuid, path: &str) -> Result<bool> {
        match self {
            Self::Postgres(db) => db.session_file_exists(session_id, path).await,
            Self::InMemory(db) => db.session_file_exists(session_id, path).await,
        }
    }

    pub async fn session_directory_has_children(
        &self,
        session_id: Uuid,
        path: &str,
    ) -> Result<bool> {
        match self {
            Self::Postgres(db) => db.session_directory_has_children(session_id, path).await,
            Self::InMemory(db) => db.session_directory_has_children(session_id, path).await,
        }
    }
}

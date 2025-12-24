// Database models (internal, may differ from public DTOs)

use chrono::{DateTime, Utc};
use everruns_core::ContentPart;
use sqlx::FromRow;
use uuid::Uuid;

// ============================================
// Auth models
// ============================================

/// User row from database
#[derive(Debug, Clone, FromRow)]
pub struct UserRow {
    pub id: Uuid,
    pub email: String,
    pub name: String,
    pub avatar_url: Option<String>,
    pub roles: sqlx::types::JsonValue,
    pub password_hash: Option<String>,
    pub email_verified: bool,
    pub auth_provider: Option<String>,
    pub auth_provider_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Auth session row (legacy, kept for backwards compatibility)
#[derive(Debug, Clone, FromRow)]
pub struct AuthSessionRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub token: String,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

/// API key row from database
#[derive(Debug, Clone, FromRow)]
pub struct ApiKeyRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub name: String,
    pub key_hash: String,
    pub key_prefix: String,
    pub scopes: sqlx::types::JsonValue,
    pub expires_at: Option<DateTime<Utc>>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// Refresh token row from database
#[derive(Debug, Clone, FromRow)]
pub struct RefreshTokenRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub token_hash: String,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

/// Input for creating a new user
#[derive(Debug, Clone)]
pub struct CreateUser {
    pub email: String,
    pub name: String,
    pub avatar_url: Option<String>,
    pub roles: Vec<String>,
    pub password_hash: Option<String>,
    pub email_verified: bool,
    pub auth_provider: Option<String>,
    pub auth_provider_id: Option<String>,
}

/// Input for updating a user
#[derive(Debug, Clone, Default)]
pub struct UpdateUser {
    pub name: Option<String>,
    pub avatar_url: Option<String>,
    pub roles: Option<Vec<String>>,
    pub password_hash: Option<String>,
    pub email_verified: Option<bool>,
}

/// Input for creating an auth session (legacy)
#[derive(Debug, Clone)]
pub struct CreateAuthSession {
    pub user_id: Uuid,
    pub token: String,
    pub expires_at: DateTime<Utc>,
}

/// Input for creating an API key
#[derive(Debug, Clone)]
pub struct CreateApiKey {
    pub user_id: Uuid,
    pub name: String,
    pub key_hash: String,
    pub key_prefix: String,
    pub scopes: Vec<String>,
    pub expires_at: Option<DateTime<Utc>>,
}

/// Input for creating a refresh token
#[derive(Debug, Clone)]
pub struct CreateRefreshToken {
    pub user_id: Uuid,
    pub token_hash: String,
    pub expires_at: DateTime<Utc>,
}

// ============================================
// Agent models (configuration for agentic loop)
// ============================================

#[derive(Debug, Clone, FromRow)]
pub struct AgentRow {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub system_prompt: String,
    pub default_model_id: Option<Uuid>,
    pub tags: Vec<String>,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CreateAgent {
    pub name: String,
    pub description: Option<String>,
    pub system_prompt: String,
    pub default_model_id: Option<Uuid>,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateAgent {
    pub name: Option<String>,
    pub description: Option<String>,
    pub system_prompt: Option<String>,
    pub default_model_id: Option<Uuid>,
    pub tags: Option<Vec<String>>,
    pub status: Option<String>,
}

// ============================================
// Session models (instance of agentic loop)
// ============================================

#[derive(Debug, Clone, FromRow)]
pub struct SessionRow {
    pub id: Uuid,
    pub agent_id: Uuid,
    pub title: Option<String>,
    pub tags: Vec<String>,
    pub model_id: Option<Uuid>,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Default)]
pub struct CreateSession {
    pub agent_id: Uuid,
    pub title: Option<String>,
    pub tags: Vec<String>,
    pub model_id: Option<Uuid>,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateSession {
    pub title: Option<String>,
    pub tags: Option<Vec<String>>,
    pub model_id: Option<Uuid>,
    pub status: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}

// ============================================
// Message models (PRIMARY conversation data)
// ============================================

/// Message row from database
/// Note: content is stored as JSONB in the database but typed as Vec<ContentPart> here.
/// The `sqlx(json)` attribute handles serialization/deserialization.
#[derive(Debug, Clone, FromRow)]
pub struct MessageRow {
    pub id: Uuid,
    pub session_id: Uuid,
    pub sequence: i32,
    pub role: String,
    #[sqlx(json)]
    pub content: Vec<ContentPart>,
    pub metadata: Option<serde_json::Value>,
    pub tags: Vec<String>,
    pub created_at: DateTime<Utc>,
}

/// Input for creating a message
#[derive(Debug, Clone)]
pub struct CreateMessageRow {
    pub session_id: Uuid,
    pub role: String,
    pub content: Vec<ContentPart>,
    pub metadata: Option<serde_json::Value>,
    pub tags: Vec<String>,
}

// ============================================
// Event models (SSE notification stream)
// ============================================

#[derive(Debug, Clone, FromRow)]
pub struct EventRow {
    pub id: Uuid,
    pub session_id: Uuid,
    pub sequence: i32,
    pub event_type: String,
    pub data: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CreateEventRow {
    pub session_id: Uuid,
    pub event_type: String,
    pub data: serde_json::Value,
}

// ============================================
// LLM Provider types
// ============================================

#[derive(Debug, Clone, FromRow)]
pub struct LlmProviderRow {
    pub id: Uuid,
    pub name: String,
    pub provider_type: String,
    pub base_url: Option<String>,
    pub api_key_encrypted: Option<Vec<u8>>,
    pub api_key_set: bool,
    pub is_default: bool,
    pub status: String,
    pub settings: sqlx::types::JsonValue,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow)]
pub struct LlmModelRow {
    pub id: Uuid,
    pub provider_id: Uuid,
    pub model_id: String,
    pub display_name: String,
    pub capabilities: sqlx::types::JsonValue,
    pub context_window: Option<i32>,
    pub is_default: bool,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Model with provider info joined
#[derive(Debug, Clone, FromRow)]
pub struct LlmModelWithProviderRow {
    pub id: Uuid,
    pub provider_id: Uuid,
    pub model_id: String,
    pub display_name: String,
    pub capabilities: sqlx::types::JsonValue,
    pub context_window: Option<i32>,
    pub is_default: bool,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub provider_name: String,
    pub provider_type: String,
}

/// LLM Provider with decrypted API key (used by worker activities)
#[derive(Debug, Clone)]
pub struct LlmProviderWithApiKey {
    pub id: Uuid,
    pub name: String,
    pub provider_type: String,
    pub base_url: Option<String>,
    /// Decrypted API key (only available when needed for LLM calls)
    pub api_key: Option<String>,
    pub settings: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct CreateLlmProvider {
    pub name: String,
    pub provider_type: String,
    pub base_url: Option<String>,
    pub api_key_encrypted: Option<Vec<u8>>,
    pub is_default: bool,
    pub settings: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct UpdateLlmProvider {
    pub name: Option<String>,
    pub provider_type: Option<String>,
    pub base_url: Option<String>,
    pub api_key_encrypted: Option<Vec<u8>>,
    pub is_default: Option<bool>,
    pub status: Option<String>,
    pub settings: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct CreateLlmModel {
    pub provider_id: Uuid,
    pub model_id: String,
    pub display_name: String,
    pub capabilities: Vec<String>,
    pub context_window: Option<i32>,
    pub is_default: bool,
}

#[derive(Debug, Clone)]
pub struct UpdateLlmModel {
    pub model_id: Option<String>,
    pub display_name: Option<String>,
    pub capabilities: Option<Vec<String>>,
    pub context_window: Option<i32>,
    pub is_default: Option<bool>,
    pub status: Option<String>,
}

// ============================================
// Agent Capability models
// ============================================

#[derive(Debug, Clone, FromRow)]
pub struct AgentCapabilityRow {
    pub id: Uuid,
    pub agent_id: Uuid,
    pub capability_id: String,
    pub position: i32,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CreateAgentCapability {
    pub agent_id: Uuid,
    pub capability_id: String,
    pub position: i32,
}

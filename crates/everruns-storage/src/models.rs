// Database models (internal, may differ from public DTOs)

use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

// ============================================
// Auth models (for future auth implementation)
// ============================================

#[derive(Debug, Clone, FromRow)]
pub struct UserRow {
    pub id: Uuid,
    pub email: String,
    pub name: String,
    pub avatar_url: Option<String>,
    pub roles: sqlx::types::JsonValue,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow)]
pub struct AuthSessionRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub token: String,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CreateUser {
    pub email: String,
    pub name: String,
    pub avatar_url: Option<String>,
    pub roles: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct CreateAuthSession {
    pub user_id: Uuid,
    pub token: String,
    pub expires_at: DateTime<Utc>,
}

// ============================================
// Harness models (M2)
// ============================================

#[derive(Debug, Clone, FromRow)]
pub struct HarnessRow {
    pub id: Uuid,
    pub slug: String,
    pub display_name: String,
    pub description: Option<String>,
    pub system_prompt: String,
    pub default_model_id: Option<Uuid>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<i32>,
    pub tags: Vec<String>,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CreateHarness {
    pub slug: String,
    pub display_name: String,
    pub description: Option<String>,
    pub system_prompt: String,
    pub default_model_id: Option<Uuid>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<i32>,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateHarness {
    pub slug: Option<String>,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub system_prompt: Option<String>,
    pub default_model_id: Option<Uuid>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<i32>,
    pub tags: Option<Vec<String>>,
    pub status: Option<String>,
}

// ============================================
// Session models (M2)
// ============================================

#[derive(Debug, Clone, FromRow)]
pub struct SessionRow {
    pub id: Uuid,
    pub harness_id: Uuid,
    pub title: Option<String>,
    pub tags: Vec<String>,
    pub model_id: Option<Uuid>,
    pub temporal_workflow_id: Option<String>,
    pub temporal_run_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Default)]
pub struct CreateSession {
    pub harness_id: Uuid,
    pub title: Option<String>,
    pub tags: Vec<String>,
    pub model_id: Option<Uuid>,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateSession {
    pub title: Option<String>,
    pub tags: Option<Vec<String>>,
    pub model_id: Option<Uuid>,
    pub temporal_workflow_id: Option<String>,
    pub temporal_run_id: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}

// ============================================
// Event models (M2)
// ============================================

#[derive(Debug, Clone, FromRow)]
pub struct EventRow {
    pub id: Uuid,
    pub session_id: Uuid,
    pub sequence: i32,
    pub event_type: String,
    pub data: sqlx::types::JsonValue,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CreateEvent {
    pub session_id: Uuid,
    pub event_type: String,
    pub data: serde_json::Value,
}

// ============================================
// Session Action models (M2)
// ============================================

#[derive(Debug, Clone, FromRow)]
pub struct SessionActionRow {
    pub id: Uuid,
    pub session_id: Uuid,
    pub kind: String,
    pub payload: sqlx::types::JsonValue,
    pub by_user_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CreateSessionAction {
    pub session_id: Uuid,
    pub kind: String,
    pub payload: serde_json::Value,
    pub by_user_id: Option<Uuid>,
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

#[derive(Debug, Clone)]
pub struct CreateLlmProvider {
    pub name: String,
    pub provider_type: String,
    pub base_url: Option<String>,
    pub api_key_encrypted: Option<Vec<u8>>,
    pub is_default: bool,
}

#[derive(Debug, Clone)]
pub struct UpdateLlmProvider {
    pub name: Option<String>,
    pub provider_type: Option<String>,
    pub base_url: Option<String>,
    pub api_key_encrypted: Option<Vec<u8>>,
    pub is_default: Option<bool>,
    pub status: Option<String>,
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

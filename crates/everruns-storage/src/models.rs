// Database models (internal, may differ from public DTOs)

use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

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
pub struct SessionRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub token: String,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow)]
pub struct AgentRow {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub default_model_id: String,
    pub definition: sqlx::types::JsonValue,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow)]
pub struct ThreadRow {
    pub id: Uuid,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow)]
pub struct MessageRow {
    pub id: Uuid,
    pub thread_id: Uuid,
    pub role: String,
    pub content: String,
    pub metadata: Option<sqlx::types::JsonValue>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow)]
pub struct RunRow {
    pub id: Uuid,
    pub agent_id: Uuid,
    pub thread_id: Uuid,
    pub status: String,
    pub temporal_workflow_id: Option<String>,
    pub temporal_run_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, FromRow)]
pub struct ActionRow {
    pub id: Uuid,
    pub run_id: Uuid,
    pub kind: String,
    pub payload: sqlx::types::JsonValue,
    pub by_user_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow)]
pub struct RunEventRow {
    pub id: Uuid,
    pub run_id: Uuid,
    pub sequence_number: i64,
    pub event_type: String,
    pub event_data: sqlx::types::JsonValue,
    pub created_at: DateTime<Utc>,
}

// Input structs for creates/updates

#[derive(Debug, Clone)]
pub struct CreateUser {
    pub email: String,
    pub name: String,
    pub avatar_url: Option<String>,
    pub roles: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct CreateSession {
    pub user_id: Uuid,
    pub token: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CreateAgent {
    pub name: String,
    pub description: Option<String>,
    pub default_model_id: String,
    pub definition: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct UpdateAgent {
    pub name: Option<String>,
    pub description: Option<String>,
    pub default_model_id: Option<String>,
    pub definition: Option<serde_json::Value>,
    pub status: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CreateThread {}

#[derive(Debug, Clone)]
pub struct CreateMessage {
    pub thread_id: Uuid,
    pub role: String,
    pub content: String,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct CreateRun {
    pub agent_id: Uuid,
    pub thread_id: Uuid,
}

#[derive(Debug, Clone)]
pub struct UpdateRun {
    pub status: Option<String>,
    pub temporal_workflow_id: Option<String>,
    pub temporal_run_id: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct CreateAction {
    pub run_id: Uuid,
    pub kind: String,
    pub payload: serde_json::Value,
    pub by_user_id: Option<Uuid>,
}

#[derive(Debug, Clone)]
pub struct CreateRunEvent {
    pub run_id: Uuid,
    pub event_type: String,
    pub event_data: serde_json::Value,
}

// LLM Provider types

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

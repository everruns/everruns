// Database models (internal, may differ from public DTOs)

use chrono::{DateTime, Utc};
use everruns_core::{
    AgentId, EventId, HarnessId, ImageId, LeasedResourceId, McpServerId, MessageId, ModelId,
    NotificationId, ProviderId, ScheduleId, SessionId, SkillId,
};
use everruns_durable::UpdateField;
use sqlx::FromRow;
use uuid::Uuid;

// ============================================
// Organization models
// ============================================

/// Organization row from database
#[derive(Debug, Clone, FromRow)]
pub struct OrganizationRow {
    pub org_id: i64,
    pub public_id: String,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// External identity provider ID (e.g., PropelAuth org ID). NULL for OSS.
    #[sqlx(default)]
    pub external_id: Option<String>,
    /// User who created this organization. NULL for seeded/external orgs.
    #[sqlx(default)]
    pub created_by: Option<Uuid>,
}

/// Organization member row from database
#[derive(Debug, Clone, FromRow)]
pub struct OrganizationMemberRow {
    pub org_id: i64,
    pub user_id: Uuid,
    pub role: String,
    pub created_at: DateTime<Utc>,
}

/// Organization member with user info (for API responses)
#[derive(Debug, Clone, FromRow)]
pub struct OrganizationMemberWithUserRow {
    pub user_id: Uuid,
    pub email: String,
    pub name: String,
    pub avatar_url: Option<String>,
    pub role: String,
    pub joined_at: DateTime<Utc>,
}

/// Organization with role (for user's org list)
#[derive(Debug, Clone, FromRow)]
pub struct OrganizationWithRoleRow {
    pub org_id: i64,
    pub public_id: String,
    pub name: String,
    pub role: String,
}

/// Input for creating an organization
#[derive(Debug, Clone)]
pub struct CreateOrganizationRow {
    pub public_id: String,
    pub name: String,
    pub created_by: Option<Uuid>,
}

/// Input for updating an organization
#[derive(Debug, Clone, Default)]
pub struct UpdateOrganization {
    pub name: Option<String>,
}

/// Organization settings row from database
#[derive(Debug, Clone, FromRow)]
pub struct OrganizationSettingsRow {
    pub org_id: i64,
    pub default_model_id: Option<ModelId>,
    pub default_harness_id: Option<HarnessId>,
    pub base_harness_id: Option<HarnessId>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateOrganizationSettings {
    pub default_model_id: UpdateField<ModelId>,
    pub default_harness_id: UpdateField<HarnessId>,
    pub base_harness_id: UpdateField<HarnessId>,
}

/// Input for creating an organization member
#[derive(Debug, Clone)]
pub struct CreateOrganizationMemberRow {
    pub org_id: i64,
    pub user_id: Uuid,
}

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
    /// External identity provider ID (e.g., PropelAuth user ID). NULL for OSS.
    #[sqlx(default)]
    pub external_id: Option<String>,
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
    pub org_id: i64,
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
pub struct CreateUserRow {
    pub email: String,
    pub name: String,
    pub avatar_url: Option<String>,
    pub roles: Vec<String>,
    pub password_hash: Option<String>,
    pub email_verified: bool,
    pub auth_provider: Option<String>,
    pub auth_provider_id: Option<String>,
    /// External identity provider ID (e.g., PropelAuth user ID). NULL for OSS.
    pub external_id: Option<String>,
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
pub struct CreateAuthSessionRow {
    pub user_id: Uuid,
    pub token: String,
    pub expires_at: DateTime<Utc>,
}

/// Input for creating an API key
#[derive(Debug, Clone)]
pub struct CreateApiKeyRow {
    pub org_id: i64,
    pub user_id: Uuid,
    pub name: String,
    pub key_hash: String,
    pub key_prefix: String,
    pub scopes: Vec<String>,
    pub expires_at: Option<DateTime<Utc>>,
}

/// Input for creating a refresh token
#[derive(Debug, Clone)]
pub struct CreateRefreshTokenRow {
    pub user_id: Uuid,
    pub token_hash: String,
    pub expires_at: DateTime<Utc>,
}

// ============================================
// Agent models (configuration for agentic loop)
// ============================================

#[derive(Debug, Clone, FromRow)]
pub struct AgentRow {
    pub id: AgentId,
    pub public_id: String,
    pub org_id: i64,
    pub name: String,
    pub description: Option<String>,
    pub system_prompt: String,
    pub default_model_id: Option<ModelId>,
    pub tags: Vec<String>,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
    /// Starter files copied into new sessions (JSONB in DB)
    #[sqlx(default)]
    pub initial_files: serde_json::Value,
    /// Client-side tools (JSONB in DB)
    #[sqlx(default)]
    pub tools: serde_json::Value,
    /// Cumulative input tokens across all sessions
    #[sqlx(default)]
    pub total_input_tokens: i64,
    /// Cumulative output tokens across all sessions
    #[sqlx(default)]
    pub total_output_tokens: i64,
    /// Cumulative cache read tokens across all sessions
    #[sqlx(default)]
    pub total_cache_read_tokens: i64,
    /// Cumulative cache creation tokens across all sessions
    #[sqlx(default)]
    pub total_cache_creation_tokens: i64,
}

#[derive(Debug, Clone)]
pub struct CreateAgentRow {
    pub public_id: String,
    pub name: String,
    pub description: Option<String>,
    pub system_prompt: String,
    pub default_model_id: Option<ModelId>,
    pub tags: Vec<String>,
    /// Starter files copied into new sessions (JSONB in DB)
    pub initial_files: serde_json::Value,
    /// Client-side tools (JSONB in DB)
    pub tools: serde_json::Value,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateAgent {
    pub name: Option<String>,
    pub description: Option<String>,
    pub system_prompt: Option<String>,
    pub default_model_id: Option<ModelId>,
    pub tags: Option<Vec<String>>,
    pub status: Option<String>,
    pub initial_files: Option<serde_json::Value>,
    pub tools: Option<serde_json::Value>,
}

// ============================================
// Harness models (base configuration for sessions)
// ============================================

#[derive(Debug, Clone, FromRow)]
pub struct HarnessRow {
    pub id: HarnessId,
    pub org_id: i64,
    pub name: String,
    pub description: Option<String>,
    pub system_prompt: String,
    pub parent_harness_id: Option<HarnessId>,
    pub default_model_id: Option<ModelId>,
    pub tags: Vec<String>,
    /// Starter files copied into new sessions (JSONB in DB)
    #[sqlx(default)]
    pub initial_files: serde_json::Value,
    pub is_built_in: bool,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct CreateHarnessRow {
    pub name: String,
    pub description: Option<String>,
    pub system_prompt: String,
    pub parent_harness_id: Option<HarnessId>,
    pub default_model_id: Option<ModelId>,
    pub tags: Vec<String>,
    /// Starter files copied into new sessions (JSONB in DB)
    pub initial_files: serde_json::Value,
    pub is_built_in: bool,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateHarness {
    pub name: Option<String>,
    pub description: Option<String>,
    pub system_prompt: Option<String>,
    pub parent_harness_id: Option<Option<HarnessId>>,
    pub default_model_id: Option<ModelId>,
    pub tags: Option<Vec<String>>,
    pub initial_files: Option<serde_json::Value>,
    pub status: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
pub struct HarnessCapabilityRow {
    pub id: Uuid,
    pub harness_id: HarnessId,
    pub capability_id: String,
    pub position: i32,
    pub config: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CreateHarnessCapabilityRow {
    pub harness_id: HarnessId,
    pub capability_id: String,
    pub position: i32,
    pub config: serde_json::Value,
}

// ============================================
// Session models (instance of agentic loop)
// ============================================

#[derive(Debug, Clone, FromRow)]
pub struct SessionRow {
    pub id: SessionId,
    pub org_id: i64,
    #[sqlx(default)]
    pub harness_id: Option<HarnessId>,
    pub agent_id: Option<AgentId>,
    pub title: Option<String>,
    #[sqlx(default)]
    pub locale: Option<String>,
    pub tags: Vec<String>,
    pub model_id: Option<ModelId>,
    /// Session-level capabilities (JSONB in DB)
    #[sqlx(default)]
    pub capabilities: serde_json::Value,
    /// Client-side tools (JSONB in DB)
    #[sqlx(default)]
    pub tools: serde_json::Value,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    /// Cumulative input tokens for all LLM calls in this session
    #[sqlx(default)]
    pub total_input_tokens: i64,
    /// Cumulative output tokens for all LLM calls in this session
    #[sqlx(default)]
    pub total_output_tokens: i64,
    /// Cumulative cache read tokens for all LLM calls in this session
    #[sqlx(default)]
    pub total_cache_read_tokens: i64,
    /// Cumulative cache creation tokens for all LLM calls in this session
    #[sqlx(default)]
    pub total_cache_creation_tokens: i64,
    // -- Subagent fields --
    #[sqlx(default)]
    pub parent_session_id: Option<SessionId>,
    #[sqlx(default)]
    pub subagent_name: Option<String>,
    #[sqlx(default)]
    pub subagent_task: Option<String>,
    #[sqlx(default)]
    pub subagent_status: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CreateSessionRow {
    pub org_id: i64,
    pub harness_id: Option<HarnessId>,
    pub agent_id: Option<AgentId>,
    pub title: Option<String>,
    pub locale: Option<String>,
    pub tags: Vec<String>,
    pub model_id: Option<ModelId>,
    /// Session-level capabilities (additive to agent capabilities)
    pub capabilities: serde_json::Value,
    /// Client-side tools (additive to agent tools, JSONB in DB)
    pub tools: serde_json::Value,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateSession {
    pub title: Option<String>,
    pub locale: Option<String>,
    pub tags: Option<Vec<String>>,
    pub model_id: Option<ModelId>,
    pub status: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}

// ============================================
// Event models (source of truth for messages)
// ============================================
//
// Messages are stored as events with type "message.*"
// The events table is the sole source of truth for conversation data.

#[derive(Debug, Clone, FromRow)]
pub struct EventRow {
    pub id: EventId,
    pub session_id: SessionId,
    pub sequence: i32,
    pub event_type: String,
    pub ts: DateTime<Utc>,
    pub context: serde_json::Value,
    pub data: serde_json::Value,
    pub metadata: Option<serde_json::Value>,
    pub tags: Option<Vec<String>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CreateEventRow {
    pub session_id: SessionId,
    pub event_type: String,
    pub ts: DateTime<Utc>,
    pub context: serde_json::Value,
    pub data: serde_json::Value,
    pub metadata: Option<serde_json::Value>,
    pub tags: Option<Vec<String>>,
}

// ============================================
// LLM Provider types
// ============================================

#[derive(Debug, Clone, FromRow)]
pub struct LlmProviderRow {
    pub id: ProviderId,
    pub org_id: i64,
    pub name: String,
    pub provider_type: String,
    pub base_url: Option<String>,
    pub api_key_encrypted: Option<Vec<u8>>,
    pub api_key_set: bool,
    pub status: String,
    pub settings: sqlx::types::JsonValue,
    /// When models were last synced from provider API
    pub last_synced_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow)]
pub struct LlmModelRow {
    pub id: ModelId,
    pub org_id: i64,
    pub provider_id: ProviderId,
    pub model_id: String,
    pub display_name: String,
    pub capabilities: sqlx::types::JsonValue,
    pub is_favorite: bool,
    /// Whether this model is installed (available in UI model pickers)
    pub installed: bool,
    pub status: String,
    /// How the model was added: manual, discovered, or predefined
    pub source: String,
    /// Last time model was seen in provider API response
    pub last_seen_at: Option<DateTime<Utc>>,
    /// Raw metadata from provider API response
    pub provider_metadata: Option<sqlx::types::JsonValue>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Model with provider info joined
#[derive(Debug, Clone, FromRow)]
pub struct LlmModelWithProviderRow {
    pub id: ModelId,
    pub org_id: i64,
    pub provider_id: ProviderId,
    pub model_id: String,
    pub display_name: String,
    pub capabilities: sqlx::types::JsonValue,
    pub is_favorite: bool,
    /// Whether this model is installed (available in UI model pickers)
    pub installed: bool,
    pub status: String,
    /// How the model was added: manual, discovered, or predefined
    pub source: String,
    /// Last time model was seen in provider API response
    pub last_seen_at: Option<DateTime<Utc>>,
    /// Raw metadata from provider API response
    pub provider_metadata: Option<sqlx::types::JsonValue>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub provider_name: String,
    pub provider_type: String,
}

/// LLM Provider with decrypted API key (used by worker activities)
#[derive(Debug, Clone)]
pub struct LlmProviderWithApiKey {
    pub id: ProviderId,
    pub name: String,
    pub provider_type: String,
    pub base_url: Option<String>,
    /// Decrypted API key (only available when needed for LLM calls)
    pub api_key: Option<String>,
    pub settings: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct CreateLlmProviderRow {
    pub name: String,
    pub provider_type: String,
    pub base_url: Option<String>,
    pub api_key_encrypted: Option<Vec<u8>>,
    pub settings: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct UpdateLlmProvider {
    pub name: Option<String>,
    pub provider_type: Option<String>,
    pub base_url: Option<String>,
    pub api_key_encrypted: Option<Vec<u8>>,
    pub status: Option<String>,
    pub settings: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct CreateLlmModelRow {
    pub provider_id: ProviderId,
    pub model_id: String,
    pub display_name: String,
    pub capabilities: Vec<String>,
    pub is_favorite: bool,
    /// Whether this model is installed (available in UI model pickers)
    pub installed: bool,
    /// How the model was added: manual, discovered, or predefined
    pub source: String,
    /// Raw metadata from provider API response
    pub provider_metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateLlmModel {
    pub model_id: Option<String>,
    pub display_name: Option<String>,
    pub capabilities: Option<Vec<String>>,
    pub is_favorite: Option<bool>,
    /// Update installed flag
    pub installed: Option<bool>,
    pub status: Option<String>,
    /// Update last_seen_at timestamp (for sync tracking)
    pub last_seen_at: Option<DateTime<Utc>>,
    /// Update provider metadata
    pub provider_metadata: Option<serde_json::Value>,
}

// ============================================
// Agent Capability models
// ============================================

#[derive(Debug, Clone, FromRow)]
pub struct AgentCapabilityRow {
    pub id: Uuid,
    pub agent_id: AgentId,
    pub capability_id: String,
    pub position: i32,
    /// Per-agent capability configuration (JSON)
    pub config: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CreateAgentCapabilityRow {
    pub agent_id: AgentId,
    pub capability_id: String,
    pub position: i32,
    /// Per-agent capability configuration (JSON)
    #[allow(dead_code)]
    pub config: serde_json::Value,
}

// ============================================
// Session File models (virtual filesystem)
// ============================================

/// Session file row from database
#[derive(Debug, Clone, FromRow)]
pub struct SessionFileRow {
    pub id: Uuid,
    pub session_id: SessionId,
    pub path: String,
    pub content: Option<Vec<u8>>,
    pub is_directory: bool,
    pub is_readonly: bool,
    pub size_bytes: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Input for creating a session file
#[derive(Debug, Clone)]
pub struct CreateSessionFileRow {
    pub session_id: SessionId,
    pub path: String,
    pub content: Option<Vec<u8>>,
    pub is_directory: bool,
    pub is_readonly: bool,
}

/// Input for updating a session file
#[derive(Debug, Clone, Default)]
pub struct UpdateSessionFile {
    pub content: Option<Vec<u8>>,
    pub is_readonly: Option<bool>,
}

/// Lightweight file info for listing (without content)
#[derive(Debug, Clone, FromRow)]
pub struct SessionFileInfoRow {
    pub id: Uuid,
    pub session_id: SessionId,
    pub path: String,
    pub is_directory: bool,
    pub is_readonly: bool,
    pub size_bytes: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ============================================
// MCP Server models
// ============================================

/// MCP Server row from database
#[derive(Debug, Clone, FromRow)]
pub struct McpServerRow {
    pub id: McpServerId,
    pub org_id: i64,
    pub name: String,
    pub description: Option<String>,
    pub url: String,
    pub transport_type: String,
    pub status: String,
    pub api_key_encrypted: Option<Vec<u8>>,
    pub api_key_set: bool,
    pub headers: sqlx::types::JsonValue,
    pub settings: sqlx::types::JsonValue,
    /// Cached tool definitions from MCP server
    pub cached_tools: sqlx::types::JsonValue,
    /// When tools were last fetched from MCP server
    pub tools_cached_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

/// Input for creating an MCP server
#[derive(Debug, Clone)]
pub struct CreateMcpServerRow {
    pub name: String,
    pub description: Option<String>,
    pub url: String,
    pub transport_type: String,
    pub api_key_encrypted: Option<Vec<u8>>,
    pub headers: Option<serde_json::Value>,
    pub settings: Option<serde_json::Value>,
}

/// Input for updating an MCP server
#[derive(Debug, Clone, Default)]
pub struct UpdateMcpServer {
    pub name: Option<String>,
    pub description: Option<String>,
    pub url: Option<String>,
    pub transport_type: Option<String>,
    pub status: Option<String>,
    pub api_key_encrypted: Option<Vec<u8>>,
    pub headers: Option<serde_json::Value>,
    pub settings: Option<serde_json::Value>,
}

// ============================================
// Image models (global image storage)
// ============================================

/// Image row from database
#[derive(Debug, Clone, FromRow)]
pub struct ImageRow {
    pub id: ImageId,
    pub org_id: i64,
    pub filename: String,
    pub content_type: String,
    pub size_bytes: i64,
    pub data: Vec<u8>,
    pub thumbnail_data: Option<Vec<u8>>,
    pub thumbnail_content_type: Option<String>,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

/// Image info without binary data (for listing)
#[derive(Debug, Clone, FromRow)]
pub struct ImageInfoRow {
    pub id: ImageId,
    pub org_id: i64,
    pub filename: String,
    pub content_type: String,
    pub size_bytes: i64,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

/// Input for creating an image
#[derive(Debug, Clone)]
pub struct CreateImageRow {
    pub org_id: i64,
    pub filename: String,
    pub content_type: String,
    pub size_bytes: i64,
    pub data: Vec<u8>,
    pub thumbnail_data: Option<Vec<u8>>,
    pub thumbnail_content_type: Option<String>,
    pub metadata: serde_json::Value,
}

/// Input for updating MCP server cached tools
#[derive(Debug, Clone)]
pub struct UpdateMcpServerTools {
    pub cached_tools: serde_json::Value,
}

// ============================================
// Session Key/Value Storage models
// ============================================

/// Notification row from database
#[derive(Debug, Clone, FromRow)]
pub struct NotificationRow {
    pub id: NotificationId,
    pub org_id: i64,
    pub user_id: Uuid,
    pub kind: String,
    pub title: String,
    pub body: String,
    pub target_type: Option<String>,
    pub target_id: Option<String>,
    pub href: Option<String>,
    pub payload: serde_json::Value,
    pub dedupe_key: Option<String>,
    pub occurrence_count: i32,
    pub viewed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Input for creating a notification
#[derive(Debug, Clone)]
pub struct CreateNotificationRow {
    pub org_id: i64,
    pub user_id: Uuid,
    pub kind: String,
    pub title: String,
    pub body: String,
    pub target_type: Option<String>,
    pub target_id: Option<String>,
    pub href: Option<String>,
    pub payload: serde_json::Value,
    pub dedupe_key: Option<String>,
}

/// Input for storing turn -> notification recipient mapping
#[derive(Debug, Clone)]
pub struct CreateNotificationTurnRequestRow {
    pub input_message_id: MessageId,
    pub org_id: i64,
    pub user_id: Uuid,
    pub session_id: SessionId,
}

/// Stored turn -> notification recipient mapping
#[derive(Debug, Clone, FromRow)]
pub struct NotificationTurnRequestRow {
    pub input_message_id: MessageId,
    pub org_id: i64,
    pub user_id: Uuid,
    pub session_id: SessionId,
    pub created_at: DateTime<Utc>,
}

/// Session key/value row from database
#[derive(Debug, Clone, FromRow)]
pub struct SessionKeyValueRow {
    pub id: Uuid,
    pub session_id: SessionId,
    pub key: String,
    pub value: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ============================================
// Audit Log models (TM-OBS-007)
// ============================================

/// Audit log row from database
#[derive(Debug, Clone, FromRow)]
pub struct AuditLogRow {
    pub id: Uuid,
    pub org_id: i64,
    pub actor_id: Option<Uuid>,
    pub event_type: String,
    pub ip_address: Option<String>,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

/// Input for creating an audit log entry
#[derive(Debug, Clone)]
pub struct CreateAuditLogRow {
    pub org_id: i64,
    pub actor_id: Option<Uuid>,
    pub event_type: String,
    pub ip_address: Option<String>,
    pub metadata: serde_json::Value,
}

/// Input for creating/updating a session key/value
#[derive(Debug, Clone)]
pub struct UpsertSessionKeyValue {
    pub session_id: SessionId,
    pub key: String,
    pub value: String,
}

/// Lightweight key info for listing (without value)
#[derive(Debug, Clone, FromRow)]
pub struct SessionKeyInfoRow {
    pub key: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ============================================
// Session Secret Storage models (encrypted)
// ============================================

/// Session secret row from database
#[derive(Debug, Clone, FromRow)]
pub struct SessionSecretRow {
    pub id: Uuid,
    pub session_id: SessionId,
    pub name: String,
    pub value_encrypted: Vec<u8>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Input for creating/updating a session secret
#[derive(Debug, Clone)]
pub struct UpsertSessionSecret {
    pub session_id: SessionId,
    pub name: String,
    pub value_encrypted: Vec<u8>,
}

/// Lightweight secret info for listing (without encrypted value)
#[derive(Debug, Clone, FromRow)]
pub struct SessionSecretInfoRow {
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ============================================
// Skill models (Agent Skills registry)
// ============================================

/// Skill row from database
#[derive(Debug, Clone, FromRow)]
pub struct SkillRow {
    pub id: SkillId,
    pub public_id: String,
    pub org_id: i64,
    pub name: String,
    pub description: String,
    pub license: Option<String>,
    pub compatibility: Option<String>,
    pub metadata: sqlx::types::JsonValue,
    pub allowed_tools: Option<String>,
    pub instructions: String,
    pub source_type: String,
    pub archive_data: Option<Vec<u8>>,
    pub status: String,
    pub version: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

/// Input for creating a skill
#[derive(Debug, Clone)]
pub struct CreateSkillRow {
    pub public_id: String,
    pub name: String,
    pub description: String,
    pub license: Option<String>,
    pub compatibility: Option<String>,
    pub metadata: serde_json::Value,
    pub allowed_tools: Option<String>,
    pub instructions: String,
    pub source_type: String,
    pub archive_data: Option<Vec<u8>>,
    pub version: String,
}

/// Input for updating a skill
#[derive(Debug, Clone, Default)]
pub struct UpdateSkill {
    pub name: Option<String>,
    pub description: Option<String>,
    pub license: Option<String>,
    pub compatibility: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub allowed_tools: Option<String>,
    pub instructions: Option<String>,
    pub status: Option<String>,
    pub version: Option<String>,
    pub archive_data: Option<Vec<u8>>,
    pub source_type: Option<String>,
}

/// Skill file row from database (extracted archive files)
#[derive(Debug, Clone, FromRow)]
pub struct SkillFileRow {
    pub id: Uuid,
    pub skill_id: Uuid,
    pub path: String,
    pub content: Option<String>,
    pub content_binary: Option<Vec<u8>>,
    pub is_binary: bool,
    pub size_bytes: i64,
    pub created_at: DateTime<Utc>,
}

/// Input for creating a skill file
#[derive(Debug, Clone)]
pub struct CreateSkillFileRow {
    pub skill_id: Uuid,
    pub path: String,
    pub content: Option<String>,
    pub content_binary: Option<Vec<u8>>,
    pub is_binary: bool,
    pub size_bytes: i64,
}

// ============================================
// User Connection models
// ============================================

/// User connection row from database
#[derive(Debug, Clone, FromRow)]
pub struct UserConnectionRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub provider: String,
    pub connection_type: String,
    pub provider_user_id: Option<String>,
    pub provider_username: Option<String>,
    /// Encrypted OAuth token (NULL for GitHub App connections)
    pub access_token_encrypted: Option<Vec<u8>>,
    pub refresh_token_encrypted: Option<Vec<u8>>,
    pub scopes: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
    /// GitHub App installation ID (tokens minted on demand)
    pub installation_id: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Input for creating a user connection
#[derive(Debug, Clone)]
pub struct CreateUserConnectionRow {
    pub user_id: Uuid,
    pub provider: String,
    pub connection_type: String,
    pub provider_user_id: Option<String>,
    pub provider_username: Option<String>,
    /// Encrypted OAuth token (None for GitHub App connections)
    pub access_token_encrypted: Option<Vec<u8>>,
    pub refresh_token_encrypted: Option<Vec<u8>>,
    pub scopes: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
    /// GitHub App installation ID (tokens minted on demand)
    pub installation_id: Option<i64>,
}

// ============================================
// Session Schedule models
// ============================================

#[derive(Debug, Clone, FromRow)]
pub struct SessionScheduleRow {
    pub id: ScheduleId,
    pub public_id: String,
    pub org_id: i64,
    pub session_id: SessionId,
    pub description: String,
    pub cron_expression: Option<String>,
    pub scheduled_at: Option<DateTime<Utc>>,
    pub timezone: String,
    pub enabled: bool,
    pub next_trigger_at: Option<DateTime<Utc>>,
    pub last_triggered_at: Option<DateTime<Utc>>,
    pub trigger_count: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CreateSessionScheduleRow {
    pub org_id: i64,
    pub session_id: SessionId,
    pub description: String,
    pub cron_expression: Option<String>,
    pub scheduled_at: Option<DateTime<Utc>>,
    pub timezone: String,
    pub next_trigger_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateSessionScheduleRow {
    pub enabled: Option<bool>,
    pub next_trigger_at: UpdateField<DateTime<Utc>>,
    pub last_triggered_at: Option<DateTime<Utc>>,
    pub trigger_count_increment: bool,
}

// ============================================
// Leased resource models
// ============================================

#[derive(Debug, Clone, FromRow)]
pub struct LeasedResourceRow {
    pub id: LeasedResourceId,
    pub public_id: String,
    pub org_id: i64,
    pub session_id: Option<SessionId>,
    pub provider: String,
    pub resource_type: String,
    pub external_id: String,
    pub display_name: Option<String>,
    pub status: String,
    pub owner_user_id: Option<Uuid>,
    pub lease_duration_seconds: i32,
    pub last_touched_at: DateTime<Utc>,
    pub lease_expires_at: DateTime<Utc>,
    pub cleanup_started_at: Option<DateTime<Utc>>,
    pub cleanup_completed_at: Option<DateTime<Utc>>,
    pub cleanup_attempts: i32,
    pub last_cleanup_error: Option<String>,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct UpsertLeasedResourceRow {
    pub org_id: i64,
    pub session_id: SessionId,
    pub provider: String,
    pub resource_type: String,
    pub external_id: String,
    pub display_name: Option<String>,
    pub owner_user_id: Option<Uuid>,
    pub lease_duration_seconds: i32,
    pub lease_expires_at: DateTime<Utc>,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct ReleaseLeasedResourceRow {
    pub org_id: i64,
    pub session_id: SessionId,
    pub provider: String,
    pub resource_type: String,
    pub external_id: String,
}

// ============================================
// App models (deployable agent+harness bundles)
// ============================================

/// App row from database
#[derive(Debug, Clone, FromRow)]
pub struct AppRow {
    pub id: Uuid,
    pub org_id: i64,
    pub public_id: String,
    pub name: String,
    pub description: Option<String>,
    pub harness_id: Uuid,
    pub agent_id: Uuid,
    pub channel_type: String,
    pub channel_config: serde_json::Value,
    pub status: String,
    pub published_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

/// Input for creating an app
#[derive(Debug, Clone)]
pub struct CreateAppRow {
    pub public_id: String,
    pub name: String,
    pub description: Option<String>,
    pub harness_id: Uuid,
    pub agent_id: Uuid,
    pub channel_type: String,
    pub channel_config: serde_json::Value,
}

/// Input for updating an app
#[derive(Debug, Clone, Default)]
pub struct UpdateApp {
    pub name: Option<String>,
    pub description: Option<String>,
    pub harness_id: Option<Uuid>,
    pub agent_id: Option<Uuid>,
    pub channel_type: Option<String>,
    pub channel_config: Option<serde_json::Value>,
    pub status: Option<String>,
    pub published_at: UpdateField<DateTime<Utc>>,
}

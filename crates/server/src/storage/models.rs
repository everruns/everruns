// Database models (internal, may differ from public DTOs)

use crate::kernel_imports::{
    everruns_provider::driver_registry::ServiceKind, everruns_provider::typed_id::AgentId,
    everruns_provider::typed_id::AgentIdentityId, everruns_provider::typed_id::EventId,
    everruns_provider::typed_id::HarnessId, everruns_provider::typed_id::ImageId,
    everruns_provider::typed_id::LeasedResourceId, everruns_provider::typed_id::McpServerId,
    everruns_provider::typed_id::MessageId, everruns_provider::typed_id::ModelId,
    everruns_provider::typed_id::NotificationId, everruns_provider::typed_id::PrincipalId,
    everruns_provider::typed_id::ProviderId, everruns_provider::typed_id::ScheduleId,
    everruns_provider::typed_id::SessionId, everruns_provider::typed_id::SessionParticipantId,
    everruns_provider::typed_id::SkillId, everruns_provider::typed_id::TriggerId,
};
use chrono::{DateTime, Utc};
use everruns_durable::UpdateField;
use everruns_platform::{SessionParticipant, SessionParticipantKind, SessionParticipantRole};
use sqlx::FromRow;
use uuid::Uuid;

/// Canonical form of an email address used as a user identity (EVE-704).
///
/// Email is the account identity key across register / login / OAuth linking /
/// password recovery, so it must be treated case-insensitively: `John@x.com`
/// and `john@x.com` are the same mailbox and must resolve to one account. We
/// canonicalize by trimming surrounding whitespace and lowercasing, matching
/// the normalization the rate limiters and org-invitation matching already use
/// (`req.email.trim().to_lowercase()`). Applied at the storage trust boundary
/// (both backends' `create_user*` and `get_user_by_email`) so every caller —
/// register, login, forgot/resend, verify, oauth_callback linking, admin
/// bootstrap — shares one identity notion, backed by a case-insensitive unique
/// index on `users(lower(email))`.
pub fn normalize_email(email: &str) -> String {
    email.trim().to_lowercase()
}

// ============================================
// Organization models
// ============================================

/// Organization row from database
#[derive(Debug, Clone, FromRow, serde::Serialize)]
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
    /// When the org's creator finished or skipped the setup wizard. NULL means
    /// onboarding is still incomplete and the resume redirect sends the current
    /// org's members back to /setup. Seeded/default and externally-synced orgs
    /// are created already-complete. See migration 090.
    #[sqlx(default)]
    pub onboarding_completed_at: Option<DateTime<Utc>>,
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

/// Org-level "default provider per service" map (EVE-569): tier-2 service
/// resolution defaults keyed by [`ServiceKind`]. Empty when no defaults are
/// configured. Persisted as a JSONB object (snake_case service kind -> provider
/// public id).
pub type ServiceProviderDefaults = std::collections::HashMap<ServiceKind, ProviderId>;

/// Organization settings row from database
#[derive(Debug, Clone, FromRow)]
pub struct OrganizationSettingsRow {
    pub org_id: i64,
    pub default_model_id: Option<ModelId>,
    pub default_harness_id: Option<HarnessId>,
    pub base_harness_id: Option<HarnessId>,
    /// Org-level default provider per service (EVE-569). Always present;
    /// an empty map means no org defaults are configured.
    pub default_provider_per_service: sqlx::types::Json<ServiceProviderDefaults>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateOrganizationSettings {
    pub default_model_id: UpdateField<ModelId>,
    pub default_harness_id: UpdateField<HarnessId>,
    pub base_harness_id: UpdateField<HarnessId>,
    /// Replaces the whole per-service default map. `Set` overwrites,
    /// `Clear` resets to empty, `Unchanged` leaves it as-is.
    pub default_provider_per_service: UpdateField<ServiceProviderDefaults>,
}

/// Organization task webhook row from database
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct OrgTaskWebhookRow {
    pub id: i64,
    pub public_id: String,
    pub org_id: i64,
    pub url: String,
    pub secret: Option<String>,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Input for creating a task webhook
#[derive(Debug, Clone)]
pub struct CreateOrgTaskWebhook {
    pub public_id: String,
    pub org_id: i64,
    pub url: String,
    pub secret: Option<String>,
    pub enabled: bool,
}

/// Input for updating a task webhook (all fields optional)
#[derive(Debug, Clone, Default)]
pub struct UpdateOrgTaskWebhook {
    pub url: Option<String>,
    pub secret: Option<Option<String>>,
    pub enabled: Option<bool>,
}

/// Per-task push-notification config row (EVE-682).
///
/// Session/task-scoped outbound webhook target. Has no `org_id`: authorization
/// is via the owning session's org. `event_filter` selects which task
/// transitions deliver ('terminal', 'awaiting_input', 'message').
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SessionTaskPushConfigRow {
    pub id: i64,
    pub public_id: String,
    pub session_id: SessionId,
    pub task_id: String,
    pub url: String,
    pub secret: Option<String>,
    pub event_filter: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Input for creating a per-task push-notification config.
#[derive(Debug, Clone)]
pub struct CreateSessionTaskPushConfig {
    pub public_id: String,
    pub session_id: SessionId,
    pub task_id: String,
    pub url: String,
    pub secret: Option<String>,
    pub event_filter: Vec<String>,
}

/// Input for creating an organization member
#[derive(Debug, Clone)]
pub struct CreateOrganizationMemberRow {
    pub org_id: i64,
    pub user_id: Uuid,
}

/// Organization invitation row from database (EVE-602).
///
/// The raw invite token is never stored; only `token_hash` (SHA-256) is kept.
/// Status is derived from the timestamp columns (`accepted_at`, `revoked_at`,
/// `expires_at`) rather than a separate enum so the row is the single source of
/// truth.
#[derive(Debug, Clone, FromRow)]
pub struct OrgInvitationRow {
    pub id: i64,
    pub public_id: String,
    pub org_id: i64,
    pub email: String,
    pub role: String,
    pub invited_by: Uuid,
    pub token_hash: String,
    pub expires_at: DateTime<Utc>,
    pub accepted_at: Option<DateTime<Utc>>,
    pub accepted_by: Option<Uuid>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Input for creating an organization invitation.
#[derive(Debug, Clone)]
pub struct CreateOrgInvitation {
    pub public_id: String,
    pub org_id: i64,
    pub email: String,
    pub role: String,
    pub invited_by: Uuid,
    pub token_hash: String,
    pub expires_at: DateTime<Utc>,
}

// ============================================
// Auth models
// ============================================

/// User row from database
#[derive(Debug, Clone, FromRow, serde::Serialize)]
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

/// Personal access token row from database
#[derive(Debug, Clone, FromRow)]
pub struct PersonalAccessTokenRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub name: String,
    pub token_hash: String,
    pub token_prefix: String,
    pub scopes: sqlx::types::JsonValue,
    pub expires_at: Option<DateTime<Utc>>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub metadata: sqlx::types::JsonValue,
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

/// Input for creating a personal access token
#[derive(Debug, Clone)]
pub struct CreatePersonalAccessTokenRow {
    pub user_id: Uuid,
    pub name: String,
    pub token_hash: String,
    pub token_prefix: String,
    pub scopes: Vec<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub metadata: serde_json::Value,
}

/// CLI auth session row from database
#[derive(Debug, Clone, FromRow)]
pub struct CliAuthSessionRow {
    pub id: Uuid,
    pub state: String,
    pub exchange_code: String,
    pub user_id: Option<Uuid>,
    pub redirect_port: i32,
    pub completed: bool,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

/// Input for creating a CLI auth session
#[derive(Debug, Clone)]
pub struct CreateCliAuthSessionRow {
    pub state: String,
    pub exchange_code: String,
    pub redirect_port: i32,
    pub expires_at: DateTime<Utc>,
}

/// Input for creating a refresh token
#[derive(Debug, Clone)]
pub struct CreateRefreshTokenRow {
    pub user_id: Uuid,
    pub token_hash: String,
    pub expires_at: DateTime<Utc>,
}

// ============================================
// OAuth models (MCP OAuth 2.1)
// ============================================

/// OAuth client row from database
#[derive(Debug, Clone, FromRow)]
pub struct OAuthClientRow {
    pub id: Uuid,
    pub client_id: String,
    pub client_secret_hash: String,
    pub client_name: String,
    pub redirect_uris: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

/// Input for creating an OAuth client
#[derive(Debug, Clone)]
pub struct CreateOAuthClientRow {
    pub client_id: String,
    pub client_secret_hash: String,
    pub client_name: String,
    pub redirect_uris: serde_json::Value,
}

/// OAuth authorization code row from database
#[derive(Debug, Clone, FromRow)]
pub struct OAuthAuthorizationCodeRow {
    pub id: Uuid,
    pub code_hash: String,
    pub client_id: String,
    pub user_id: Uuid,
    pub org_id: i64,
    pub redirect_uri: String,
    pub code_challenge: String,
    pub code_challenge_method: String,
    pub scope: String,
    pub consumed: bool,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

/// Input for creating an OAuth authorization code
#[derive(Debug, Clone)]
pub struct CreateOAuthAuthorizationCodeRow {
    pub code_hash: String,
    pub client_id: String,
    pub user_id: Uuid,
    pub org_id: i64,
    pub redirect_uri: String,
    pub code_challenge: String,
    pub code_challenge_method: String,
    pub scope: String,
    pub expires_at: DateTime<Utc>,
}

/// OAuth refresh token row from database
#[derive(Debug, Clone, FromRow)]
pub struct OAuthRefreshTokenRow {
    pub id: Uuid,
    pub token_hash: String,
    pub client_id: String,
    pub user_id: Uuid,
    pub org_id: i64,
    pub scope: String,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

/// Input for creating an OAuth refresh token
#[derive(Debug, Clone)]
pub struct CreateOAuthRefreshTokenRow {
    pub token_hash: String,
    pub client_id: String,
    pub user_id: Uuid,
    pub org_id: i64,
    pub scope: String,
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
    #[sqlx(default)]
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub system_prompt: String,
    pub default_model_id: Option<ModelId>,
    pub harness_id: HarnessId,
    /// Whether `harness_id` was explicitly selected or materialized from the
    /// organization default for backward-compatible API responses.
    #[sqlx(default)]
    pub harness_source: String,
    /// Lazily-created identity principal subject for this agent (EVE-758).
    /// NULL until the agent first acts unattended (e.g. an agent trigger fire),
    /// at which point an `agent_identities` row is created and linked so the
    /// agent owns its unattended sessions as itself. Storage-only: intentionally
    /// not surfaced on the public `everruns_platform::Agent` API.
    #[sqlx(default)]
    pub agent_identity_id: Option<AgentIdentityId>,
    #[sqlx(default)]
    pub default_version_id: Option<everruns_provider::typed_id::AgentVersionId>,
    #[sqlx(default)]
    pub forked_from_agent_id: Option<AgentId>,
    #[sqlx(default)]
    pub forked_from_version_id: Option<everruns_provider::typed_id::AgentVersionId>,
    #[sqlx(default)]
    pub root_agent_id: Option<AgentId>,
    pub tags: Vec<String>,
    pub status: String,
    /// Platform-supplied agent (mirrors `HarnessRow::is_built_in`). Its
    /// definition is immutable through the API and excluded from the per-org
    /// agent limit; bindings around it stay editable.
    #[sqlx(default)]
    pub is_built_in: bool,
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
    /// Scoped MCP server configs (JSONB in DB)
    #[sqlx(default)]
    pub mcp_servers: serde_json::Value,
    /// Network access list (JSONB in DB, nullable)
    #[sqlx(default)]
    pub network_access: Option<serde_json::Value>,
    /// Maximum iterations per turn
    #[sqlx(default)]
    pub max_iterations: Option<i32>,
    /// Request-level parallel tool calling preference (EVE-598)
    #[sqlx(default)]
    pub parallel_tool_calls: Option<bool>,
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
    /// Cumulative provider-reported actual cost in USD across all sessions
    #[sqlx(default)]
    pub total_actual_cost_usd: f64,
    /// Cumulative price-table estimated cost in USD across all sessions
    #[sqlx(default)]
    pub total_estimated_cost_usd: f64,
    /// Cumulative best-effort cost in USD across all sessions (actual where
    /// present, else estimated)
    #[sqlx(default)]
    pub total_cost_usd: f64,
}

#[derive(Debug, Clone, FromRow)]
pub struct AgentVersionRow {
    pub id: everruns_provider::typed_id::AgentVersionId,
    pub public_id: String,
    pub org_id: i64,
    pub agent_id: AgentId,
    pub version_number: i32,
    pub semver_major: i32,
    pub semver_minor: i32,
    pub semver_patch: i32,
    pub version: String,
    pub is_published: bool,
    pub parent_version_id: Option<everruns_provider::typed_id::AgentVersionId>,
    pub source_version_id: Option<everruns_provider::typed_id::AgentVersionId>,
    pub created_by_principal_id: Option<PrincipalId>,
    pub change_kind: String,
    pub summary: Option<String>,
    pub config_hash: String,
    pub authored_config: serde_json::Value,
    pub resolved_config: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow)]
pub struct AgentMcpSecretBindingRow {
    pub id: Uuid,
    pub org_id: i64,
    pub agent_id: AgentId,
    pub mcp_server_name: String,
    pub mcp_server_url: String,
    pub tool_name: String,
    pub parameter_name: String,
    pub label: String,
    pub description: Option<String>,
    pub value_encrypted: Option<Vec<u8>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct UpsertAgentMcpSecretBindingRow {
    pub org_id: i64,
    pub agent_id: AgentId,
    pub mcp_server_name: String,
    pub mcp_server_url: String,
    pub tool_name: String,
    pub parameter_name: String,
    pub label: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CreateAgentVersionRow {
    pub id: everruns_provider::typed_id::AgentVersionId,
    pub public_id: String,
    pub org_id: i64,
    pub agent_id: AgentId,
    pub version_number: i32,
    pub semver_major: i32,
    pub semver_minor: i32,
    pub semver_patch: i32,
    pub version: String,
    pub is_published: bool,
    pub parent_version_id: Option<everruns_provider::typed_id::AgentVersionId>,
    pub source_version_id: Option<everruns_provider::typed_id::AgentVersionId>,
    pub created_by_principal_id: Option<PrincipalId>,
    pub change_kind: String,
    pub summary: Option<String>,
    pub config_hash: String,
    pub authored_config: serde_json::Value,
    pub resolved_config: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct CreateAgentRow {
    pub public_id: String,
    pub name: String,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub system_prompt: String,
    pub default_model_id: Option<ModelId>,
    pub harness_id: HarnessId,
    pub tags: Vec<String>,
    /// Starter files copied into new sessions (JSONB in DB)
    pub initial_files: serde_json::Value,
    /// Client-side tools (JSONB in DB)
    pub tools: serde_json::Value,
    /// Scoped MCP server configs (JSONB in DB)
    pub mcp_servers: serde_json::Value,
    /// Network access list (JSONB in DB)
    pub network_access: Option<serde_json::Value>,
    /// Maximum iterations per turn
    pub max_iterations: Option<i32>,
    /// Request-level parallel tool calling preference (EVE-598)
    pub parallel_tool_calls: Option<bool>,
    /// Platform-supplied agent. Only org bootstrap sets this; every API-facing
    /// creation path leaves it false.
    pub is_built_in: bool,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateAgent {
    pub name: Option<String>,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub system_prompt: Option<String>,
    pub default_model_id: Option<ModelId>,
    pub harness_id: Option<HarnessId>,
    pub harness_source: Option<String>,
    pub default_version_id: Option<everruns_provider::typed_id::AgentVersionId>,
    pub forked_from_agent_id: Option<AgentId>,
    pub forked_from_version_id: Option<everruns_provider::typed_id::AgentVersionId>,
    pub root_agent_id: Option<AgentId>,
    pub tags: Option<Vec<String>>,
    pub status: Option<String>,
    pub initial_files: Option<serde_json::Value>,
    pub tools: Option<serde_json::Value>,
    pub mcp_servers: Option<serde_json::Value>,
    pub network_access: Option<Option<serde_json::Value>>,
    /// None = don't change, Some(None) = set to NULL, Some(Some(v)) = set to v
    pub max_iterations: Option<Option<i32>>,
    /// Request-level parallel tool calling preference (EVE-598).
    /// None = don't change, Some(None) = set to NULL, Some(Some(v)) = set to v
    pub parallel_tool_calls: Option<Option<bool>>,
}

// ============================================
// Harness models (base configuration for sessions)
// ============================================

#[derive(Debug, Clone, FromRow, serde::Serialize)]
pub struct HarnessRow {
    pub id: HarnessId,
    pub org_id: i64,
    pub name: String,
    #[sqlx(default)]
    pub display_name: Option<String>,
    pub description: Option<String>,
    /// Base system prompt. Nullable: a harness may contribute no base prompt
    /// and rely entirely on inheritance, agent, session, and capability layers.
    pub system_prompt: Option<String>,
    pub parent_harness_id: Option<HarnessId>,
    pub default_model_id: Option<ModelId>,
    pub tags: Vec<String>,
    /// Starter files copied into new sessions (JSONB in DB)
    #[sqlx(default)]
    pub initial_files: serde_json::Value,
    /// Network access list (JSONB in DB, nullable)
    #[sqlx(default)]
    pub network_access: Option<serde_json::Value>,
    /// Scoped MCP server configs (JSONB in DB)
    #[sqlx(default)]
    pub mcp_servers: serde_json::Value,
    /// Embedder-supplied metadata for LLM observability (JSONB in DB)
    #[sqlx(default)]
    pub embedder_metadata: serde_json::Value,
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
    pub display_name: Option<String>,
    pub description: Option<String>,
    /// Base system prompt; `None` means the harness contributes no base prompt.
    pub system_prompt: Option<String>,
    pub parent_harness_id: Option<HarnessId>,
    pub default_model_id: Option<ModelId>,
    pub tags: Vec<String>,
    /// Starter files copied into new sessions (JSONB in DB)
    pub initial_files: serde_json::Value,
    /// Scoped MCP server configs (JSONB in DB)
    pub mcp_servers: serde_json::Value,
    /// Network access list (JSONB in DB)
    pub network_access: Option<serde_json::Value>,
    /// Embedder-supplied metadata for LLM observability (JSONB in DB)
    pub embedder_metadata: serde_json::Value,
    pub is_built_in: bool,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateHarness {
    pub name: Option<String>,
    pub display_name: Option<String>,
    pub description: Option<String>,
    /// None = leave unchanged; Some(None) = clear to no base prompt;
    /// Some(Some(v)) = set to v.
    pub system_prompt: Option<Option<String>>,
    pub parent_harness_id: Option<Option<HarnessId>>,
    pub default_model_id: Option<ModelId>,
    pub tags: Option<Vec<String>>,
    pub initial_files: Option<serde_json::Value>,
    pub mcp_servers: Option<serde_json::Value>,
    pub network_access: Option<Option<serde_json::Value>>,
    pub embedder_metadata: Option<serde_json::Value>,
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
    /// Workspace this session is attached to (owns the virtual filesystem).
    /// `#[sqlx(default)]` so projections that don't select it (e.g. stats) still
    /// decode; all session-detail/list queries select it explicitly.
    #[sqlx(default)]
    pub workspace_id: Uuid,
    #[sqlx(default)]
    pub app_id: Option<Uuid>,
    #[sqlx(default)]
    pub harness_id: Option<HarnessId>,
    pub agent_id: Option<AgentId>,
    #[sqlx(default)]
    pub agent_version_id: Option<everruns_provider::typed_id::AgentVersionId>,
    #[sqlx(default)]
    pub agent_config_hash: Option<String>,
    #[sqlx(default)]
    pub agent_identity_id: Option<AgentIdentityId>,
    pub owner_principal_id: PrincipalId,
    #[sqlx(default)]
    pub resolved_owner_user_id: Option<Uuid>,
    pub title: Option<String>,
    #[sqlx(default)]
    pub goal: Option<String>,
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
    /// Scoped MCP server configs (JSONB in DB)
    #[sqlx(default)]
    pub mcp_servers: serde_json::Value,
    /// Session-level system prompt override
    #[sqlx(default)]
    pub system_prompt: Option<String>,
    /// Session-level initial files (JSONB in DB)
    #[sqlx(default)]
    pub initial_files: serde_json::Value,
    /// Session-level client hints (JSONB in DB, nullable)
    #[sqlx(default)]
    pub hints: Option<serde_json::Value>,
    /// Network access list (JSONB in DB, nullable)
    #[sqlx(default)]
    pub network_access: Option<serde_json::Value>,
    /// Maximum iterations per turn
    #[sqlx(default)]
    pub max_iterations: Option<i32>,
    /// Request-level parallel tool calling preference (EVE-598)
    #[sqlx(default)]
    pub parallel_tool_calls: Option<bool>,
    pub status: String,
    /// How the session was started (EVE-852). Stored as the closed-set string
    /// backing `everruns_platform::SessionSource`.
    #[sqlx(default)]
    pub source: String,
    /// Denormalized outcome of the most recent terminal turn: `completed`,
    /// `failed`, `cancelled`, or `None` when no turn has finished yet.
    #[sqlx(default)]
    pub last_turn_status: Option<String>,
    #[sqlx(default)]
    pub last_turn_at: Option<DateTime<Utc>>,
    /// Generated one-sentence description of what the run did (EVE-867).
    /// `None` whenever no summary exists, which is the resting state for chat
    /// threads and for deployments with no utility LLM.
    #[sqlx(default)]
    pub run_summary: Option<String>,
    /// `events.sequence` of the terminal turn `run_summary` describes. Fences a
    /// late out-of-band write for an older turn.
    #[sqlx(default)]
    pub run_summary_turn_sequence: Option<i64>,
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
    /// Denormalized count of turn.completed, turn.failed, and turn.cancelled events
    #[sqlx(default)]
    pub turn_count: i64,
    /// Denormalized count of tool.completed events
    #[sqlx(default)]
    pub tool_call_count: i64,
    /// Cumulative provider-reported actual cost in USD for this session
    #[sqlx(default)]
    pub total_actual_cost_usd: f64,
    /// Cumulative price-table estimated cost in USD for this session
    #[sqlx(default)]
    pub total_estimated_cost_usd: f64,
    /// Cumulative best-effort cost in USD for this session (actual where present,
    /// else estimated)
    #[sqlx(default)]
    pub total_cost_usd: f64,
    // -- Subagent nesting fields --
    #[sqlx(default)]
    pub parent_session_id: Option<SessionId>,
    /// Root of this session's delegation tree (EVE-680). A top-level session is
    /// its own root; a subagent child inherits its parent's root. Denormalized
    /// so a whole tree is one indexed query. Set by the storage layer at
    /// creation; `#[sqlx(default)]` because most SELECTs don't project it.
    #[sqlx(default)]
    pub root_session_id: Option<SessionId>,
    // -- Fork lineage fields (knowledge/runtime-resources/forking-sessions.md) --
    #[sqlx(default)]
    pub forked_from_session_id: Option<SessionId>,
    #[sqlx(default)]
    pub forked_from_sequence: Option<i32>,
    // -- Blueprint fields --
    #[sqlx(default)]
    pub blueprint_id: Option<String>,
    #[sqlx(default)]
    pub blueprint_config: Option<serde_json::Value>,
}

/// Ordering for the sessions list. The chat thread list wants last activity;
/// the operational list wants creation order.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SessionListOrder {
    #[default]
    CreatedAt,
    LastActivity,
}

/// Filter predicate shared by the sessions list and its facet aggregates
/// (EVE-852). Both read the same struct so a count can never describe a
/// different population than the page it annotates.
#[derive(Debug, Clone, Default)]
pub struct SessionListFilters {
    pub agent_id: Option<AgentId>,
    pub search: Option<String>,
    /// Empty means "any source".
    pub sources: Vec<everruns_platform::SessionSource>,
    /// Empty means "any activity".
    pub activities: Vec<everruns_platform::SessionActivity>,
    /// Restrict to sessions whose resolved human owner is this user (`mine`).
    pub owner_user_id: Option<Uuid>,
    pub created_after: Option<DateTime<Utc>>,
    pub created_before: Option<DateTime<Utc>>,
    pub order: SessionListOrder,
}

/// One bucket of a facet rail dimension.
#[derive(Debug, Clone, FromRow)]
pub struct SessionFacetBucket {
    pub value: String,
    pub count: i64,
}

/// Masthead metrics and facet-rail counts for the sessions surface.
#[derive(Debug, Clone, Default)]
pub struct SessionFacetsRow {
    pub total: i64,
    pub by_activity: Vec<SessionFacetBucket>,
    pub by_source: Vec<SessionFacetBucket>,
    /// `value` holds the agent's public id; `NULL` agents are omitted.
    pub by_agent: Vec<SessionFacetBucket>,
    pub active_now: i64,
    pub failed_today: i64,
    pub p95_duration_ms: i64,
    pub tokens_today: i64,
}

#[derive(Debug, Clone, Default, FromRow)]
pub struct SessionMastheadRow {
    pub active_now: i64,
    pub failed_today: i64,
    pub p95_duration_ms: i64,
    pub tokens_today: i64,
}

#[derive(Debug, Clone, Default, FromRow)]
pub struct SessionAggregateStatsRow {
    pub session_count: i64,
    pub active_session_count: i64,
    pub idle_session_count: i64,
    pub started_session_count: i64,
    pub waiting_for_tool_results_session_count: i64,
    pub execution_count: i64,
    pub total_session_duration_ms: i64,
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    pub total_cache_read_tokens: i64,
    pub total_cache_creation_tokens: i64,
    pub total_actual_cost_usd: f64,
    pub total_estimated_cost_usd: f64,
    pub total_cost_usd: f64,
    pub first_session_at: Option<DateTime<Utc>>,
    pub last_session_at: Option<DateTime<Utc>>,
    pub last_execution_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct CreateSessionRow {
    pub org_id: i64,
    /// How this session was started. Set by the creating ingress path, never
    /// taken from untrusted client input except for the two client-declarable
    /// variants (see `SessionSource::is_client_declarable`).
    pub source: everruns_platform::SessionSource,
    pub app_id: Option<Uuid>,
    pub harness_id: Option<HarnessId>,
    pub agent_id: Option<AgentId>,
    pub agent_version_id: Option<everruns_provider::typed_id::AgentVersionId>,
    pub agent_config_hash: Option<String>,
    pub agent_identity_id: Option<AgentIdentityId>,
    pub owner_principal_id: PrincipalId,
    pub resolved_owner_user_id: Option<Uuid>,
    pub title: Option<String>,
    pub locale: Option<String>,
    pub tags: Vec<String>,
    pub model_id: Option<ModelId>,
    /// Session-level capabilities (additive to agent capabilities)
    pub capabilities: serde_json::Value,
    /// Client-side tools (additive to agent tools, JSONB in DB)
    pub tools: serde_json::Value,
    /// Scoped MCP server configs (JSONB in DB)
    pub mcp_servers: serde_json::Value,
    /// Session-level system prompt override (prepended to agent prompt)
    pub system_prompt: Option<String>,
    /// Session-level initial files (JSONB in DB, additive to agent files)
    pub initial_files: serde_json::Value,
    /// Session-level client hints (JSONB in DB)
    pub hints: Option<serde_json::Value>,
    /// Network access list (JSONB in DB)
    pub network_access: Option<serde_json::Value>,
    /// Maximum iterations per turn
    pub max_iterations: Option<i32>,
    /// Request-level parallel tool calling preference (EVE-598)
    pub parallel_tool_calls: Option<bool>,
    /// Blueprint ID for blueprint-backed sessions.
    pub blueprint_id: Option<String>,
    /// Validated blueprint config (JSONB in DB).
    pub blueprint_config: Option<serde_json::Value>,
    /// Parent session ID for governed subagent depth tracking.
    pub parent_session_id: Option<everruns_provider::typed_id::SessionId>,
    /// Explicit internal-only budget/delegation root for detached peers.
    pub budget_root_session_id: Option<everruns_provider::typed_id::SessionId>,
    /// Internal id of an existing workspace to attach this session to. When
    /// `None`, `create_session` auto-creates a default 1:1 workspace whose id
    /// equals the new session id (the equality invariant). When `Some`, the
    /// session attaches to that workspace and no new workspace is created.
    pub workspace_id: Option<Uuid>,
}

#[derive(Debug, Clone, FromRow)]
pub struct SessionParticipantRow {
    pub id: SessionParticipantId,
    pub org_id: i64,
    pub session_id: SessionId,
    pub kind: String,
    pub agent_id: Option<AgentId>,
    pub agent_version_id: Option<everruns_provider::typed_id::AgentVersionId>,
    pub principal_id: PrincipalId,
    pub display_name: Option<String>,
    pub role: String,
    pub joined_at: DateTime<Utc>,
    pub left_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl SessionParticipantRow {
    pub fn to_core(&self) -> SessionParticipant {
        SessionParticipant {
            id: self.id,
            session_id: self.session_id,
            kind: SessionParticipantKind::from(self.kind.as_str()),
            agent_id: self.agent_id,
            agent_version_id: self.agent_version_id,
            principal_id: self.principal_id,
            display_name: self.display_name.clone(),
            role: SessionParticipantRole::from(self.role.as_str()),
            joined_at: self.joined_at,
            left_at: self.left_at,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CreateSessionParticipantRow {
    pub org_id: i64,
    pub session_id: SessionId,
    pub kind: SessionParticipantKind,
    pub agent_id: Option<AgentId>,
    pub agent_version_id: Option<everruns_provider::typed_id::AgentVersionId>,
    pub principal_id: PrincipalId,
    pub display_name: Option<String>,
    pub role: SessionParticipantRole,
    pub joined_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateSession {
    pub harness_id: Option<HarnessId>,
    pub agent_version_id: Option<everruns_provider::typed_id::AgentVersionId>,
    pub agent_config_hash: Option<String>,
    pub title: Option<String>,
    pub goal: Option<String>,
    pub agent_identity_id: UpdateField<AgentIdentityId>,
    pub owner_principal_id: Option<PrincipalId>,
    pub resolved_owner_user_id: UpdateField<Uuid>,
    pub locale: Option<String>,
    pub tags: Option<Vec<String>>,
    pub model_id: Option<ModelId>,
    pub status: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReserveActiveTurnSlotResult {
    /// Slot reserved (session marked `active`). Carries the status the session
    /// held beforehand so a caller can release the reservation on a later
    /// failure by restoring it.
    Reserved {
        previous_status: String,
    },
    AtCapacity {
        active_turns: i64,
    },
    SessionNotFound,
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

/// Parameters for the advanced event listing path used by debugging tools.
///
/// Keep field set additive: new debug filters should be added here, not as
/// new positional arguments on the legacy `list_events` storage method.
#[derive(Debug, Clone, Default)]
pub struct ListEventsParams {
    pub session_id: SessionId,
    /// Forward cursor: events with `sequence > after_sequence`.
    pub after_sequence: Option<i32>,
    /// Forward cursor by id; resolves to its sequence at query time.
    pub since_id: Option<EventId>,
    /// Backward cursor: events with `sequence < before_sequence`.
    pub before_sequence: Option<i32>,
    /// Window anchor: returns up to `window` events on each side of this id.
    /// Mutually exclusive with `before_sequence`, `after_sequence`, and `since_id`
    /// (the command layer rejects combinations with a 400).
    pub around_id: Option<EventId>,
    /// Window size for `around_id` (rows on each side). Defaults to 50; capped at 500.
    pub window: Option<i32>,
    /// Positive event-type filter (empty = no filter).
    pub filter_types: Vec<String>,
    /// Negative event-type filter (applied after `filter_types`).
    pub exclude_types: Vec<String>,
    /// `created_at >= from_ts`.
    pub from_ts: Option<DateTime<Utc>>,
    /// `created_at <= to_ts`.
    pub to_ts: Option<DateTime<Utc>>,
    /// `context->>'turn_id' = turn_id`.
    pub turn_id: Option<String>,
    /// `context->>'exec_id' = exec_id`.
    pub exec_id: Option<String>,
    /// `context->>'trace_id' = trace_id`.
    pub trace_id: Option<String>,
    /// `events.tags && tags` (any-match).
    pub tags: Vec<String>,
    /// Match `data->>'tool_name' = tool_name`.
    pub tool_name: Option<String>,
    /// Full-text search via `search_vector @@ plainto_tsquery('english', q)`.
    /// In-memory mode falls back to a case-insensitive substring scan.
    pub q: Option<String>,
    /// When true, return newest first (sequence DESC). Ignored when
    /// `around_id` is set — the window is always returned in ASC order.
    pub order_desc: bool,
    /// Max rows to return.
    pub limit: Option<i32>,
}

/// Per-type event counts plus aggregate timestamps for a session.
#[derive(Debug, Clone)]
pub struct EventTypeCount {
    pub event_type: String,
    pub count: i64,
}

/// One-shot debug summary for a session: counts by type, first/last timestamps.
#[derive(Debug, Clone, Default)]
pub struct EventsSummary {
    pub total: i64,
    pub by_type: Vec<EventTypeCount>,
    pub first_ts: Option<DateTime<Utc>>,
    pub last_ts: Option<DateTime<Utc>>,
}

// ============================================
// LLM Provider types
// ============================================

#[derive(Debug, Clone, FromRow, serde::Serialize)]
pub struct ProviderRow {
    pub id: ProviderId,
    pub org_id: i64,
    pub name: String,
    pub provider_type: String,
    pub base_url: Option<String>,
    pub api_key_encrypted: Option<Vec<u8>>,
    pub api_key_set: bool,
    pub status: String,
    pub settings: sqlx::types::JsonValue,
    /// Host-managed flag (EVE-810). When true, the OSS providers API refuses
    /// PATCH/DELETE on this row; the host owns it.
    pub managed: bool,
    /// When models were last synced from provider API
    pub last_synced_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, serde::Serialize)]
pub struct ModelRow {
    pub id: ModelId,
    pub org_id: i64,
    pub provider_id: ProviderId,
    pub model_id: String,
    pub display_name: String,
    pub capabilities: sqlx::types::JsonValue,
    pub is_favorite: bool,
    /// Whether this model is enabled (visible in UI model pickers)
    pub enabled: bool,
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
#[derive(Debug, Clone, FromRow, serde::Serialize)]
pub struct ModelWithProviderRow {
    pub id: ModelId,
    pub org_id: i64,
    pub provider_id: ProviderId,
    pub model_id: String,
    pub display_name: String,
    pub capabilities: sqlx::types::JsonValue,
    pub is_favorite: bool,
    /// Whether this model is enabled (visible in UI model pickers)
    pub enabled: bool,
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
    /// Joined from `providers.api_key_set`. Used to derive `healthy` on
    /// the public model shape; stays internal to the storage layer.
    pub provider_api_key_set: bool,
    /// Joined from `providers.status`.
    pub provider_status: String,
}

/// LLM Provider with decrypted API key (used by worker activities)
#[derive(Debug, Clone)]
pub struct ProviderWithApiKey {
    pub id: ProviderId,
    pub name: String,
    pub provider_type: String,
    pub base_url: Option<String>,
    /// Decrypted API key (only available when needed for LLM calls)
    pub api_key: Option<String>,
    pub settings: serde_json::Value,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct CreateProviderRow {
    pub name: String,
    pub provider_type: String,
    pub base_url: Option<String>,
    pub api_key_encrypted: Option<Vec<u8>>,
    pub settings: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateProvider {
    pub name: Option<String>,
    pub provider_type: Option<String>,
    pub base_url: Option<String>,
    pub api_key_encrypted: Option<Vec<u8>>,
    pub status: Option<String>,
    pub settings: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct CreateModelRow {
    pub provider_id: ProviderId,
    pub model_id: String,
    pub display_name: String,
    pub capabilities: Vec<String>,
    pub is_favorite: bool,
    /// Whether this model is enabled (visible in UI model pickers)
    pub enabled: bool,
    /// How the model was added: manual, discovered, or predefined
    pub source: String,
    /// Raw metadata from provider API response
    pub provider_metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateModel {
    pub provider_id: Option<ProviderId>,
    pub model_id: Option<String>,
    pub display_name: Option<String>,
    pub capabilities: Option<Vec<String>>,
    pub is_favorite: Option<bool>,
    /// Update enabled flag
    pub enabled: Option<bool>,
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
#[derive(Debug, Clone, FromRow, serde::Serialize)]
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
#[derive(Debug, Clone, FromRow, serde::Serialize)]
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
// Workspace Memory models
// ============================================

#[derive(Debug, Clone, FromRow, serde::Serialize)]
pub struct MemoryRow {
    pub id: Uuid,
    pub org_id: i64,
    pub public_id: String,
    pub name: String,
    pub description: Option<String>,
    pub scope: String,
    pub owner_agent_id: Option<AgentId>,
    pub owner_user_id: Option<Uuid>,
    pub source_type: String,
    pub source_config: serde_json::Value,
    pub is_readonly: bool,
    pub sync_status: String,
    pub last_synced_at: Option<DateTime<Utc>>,
    pub last_sync_error: Option<String>,
    pub owner_principal_id: Option<String>,
    pub resolved_owner_user_id: Option<Uuid>,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct CreateMemoryRow {
    pub public_id: String,
    pub name: String,
    pub description: Option<String>,
    pub scope: String,
    pub owner_agent_id: Option<AgentId>,
    pub owner_user_id: Option<Uuid>,
    pub source_type: String,
    pub source_config: serde_json::Value,
    pub is_readonly: bool,
    pub sync_status: String,
    pub owner_principal_id: Option<String>,
    pub resolved_owner_user_id: Option<Uuid>,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateMemory {
    pub name: Option<String>,
    pub description: Option<Option<String>>,
    pub status: Option<String>,
    pub source_type: Option<String>,
    pub source_config: Option<serde_json::Value>,
    pub is_readonly: Option<bool>,
    pub sync_status: Option<String>,
    pub last_synced_at: Option<Option<DateTime<Utc>>>,
    pub last_sync_error: Option<Option<String>>,
}

// ============================================
// Workspace models (see knowledge/runtime-resources/workspace.md)
// ============================================

#[derive(Debug, Clone, FromRow, serde::Serialize)]
pub struct WorkspaceRow {
    pub id: Uuid,
    pub org_id: i64,
    pub public_id: String,
    pub name: String,
    pub description: Option<String>,
    pub owner_principal_id: Option<String>,
    pub resolved_owner_user_id: Option<Uuid>,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct CreateWorkspaceRow {
    /// Optional explicit UUID. When None, the DB DEFAULT (uuidv7) is used.
    /// Sessions creating their default workspace pass their own id here so
    /// `workspace.id == session.id` (see knowledge/runtime-resources/workspace.md, Decision 3).
    pub id: Option<Uuid>,
    pub public_id: String,
    pub name: String,
    pub description: Option<String>,
    pub owner_principal_id: Option<String>,
    pub resolved_owner_user_id: Option<Uuid>,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateWorkspace {
    pub name: Option<String>,
    pub description: Option<Option<String>>,
    pub status: Option<String>,
}

#[derive(Debug, Clone, FromRow, serde::Serialize)]
pub struct MemoryFileRow {
    pub id: Uuid,
    pub memory_id: Uuid,
    pub path: String,
    pub content: Option<Vec<u8>>,
    pub is_directory: bool,
    pub size_bytes: i64,
    pub content_hash: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CreateMemoryFileRow {
    pub path: String,
    pub content: Option<Vec<u8>>,
    pub is_directory: bool,
    pub content_hash: Option<String>,
}

/// Input for updating a memory file. Optional fields are left unchanged when None.
#[derive(Debug, Clone, Default)]
pub struct UpdateMemoryFile {
    pub content: Option<Vec<u8>>,
    pub content_hash: Option<Option<String>>,
}

/// Lightweight memory file info for listings (no content).
#[derive(Debug, Clone, FromRow, serde::Serialize)]
pub struct MemoryFileInfoRow {
    pub id: Uuid,
    pub memory_id: Uuid,
    pub path: String,
    pub is_directory: bool,
    pub size_bytes: i64,
    pub content_hash: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ============================================
// Knowledge Base models (curated org knowledge — see knowledge/runtime-resources/knowledge-bases.md)
// ============================================

#[derive(Debug, Clone, FromRow, serde::Serialize)]
pub struct KnowledgeBaseRow {
    pub id: Uuid,
    pub org_id: i64,
    pub public_id: String,
    pub name: String,
    pub description: Option<String>,
    pub owner_principal_id: Option<String>,
    pub resolved_owner_user_id: Option<Uuid>,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
    /// Optional embedding model for hybrid retrieval. NULL = keyword search only.
    #[sqlx(default)]
    pub embedding_model_id: Option<ModelId>,
}

#[derive(Debug, Clone)]
pub struct CreateKnowledgeBaseRow {
    pub public_id: String,
    pub name: String,
    pub description: Option<String>,
    pub owner_principal_id: Option<String>,
    pub resolved_owner_user_id: Option<Uuid>,
    /// Optional embedding model for hybrid retrieval.
    pub embedding_model_id: Option<ModelId>,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateKnowledgeBase {
    pub name: Option<String>,
    pub description: Option<Option<String>>,
    pub status: Option<String>,
    /// Optional update to the embedding model. `None` = unchanged.
    pub embedding_model_id: Option<UpdateField<ModelId>>,
}

#[derive(Debug, Clone, FromRow, serde::Serialize)]
pub struct KnowledgeEntryRow {
    pub id: Uuid,
    pub kb_id: Uuid,
    pub public_id: String,
    pub title: String,
    pub body: String,
    pub kind: String,
    pub tags: Vec<String>,
    /// Optional OKF `resource` URI identifying the underlying asset.
    /// See knowledge/runtime-resources/okf-adoption.md.
    #[sqlx(default)]
    pub resource: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CreateKnowledgeEntryRow {
    pub public_id: String,
    pub title: String,
    pub body: String,
    pub kind: String,
    pub tags: Vec<String>,
    pub resource: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateKnowledgeEntry {
    pub title: Option<String>,
    pub body: Option<String>,
    pub kind: Option<String>,
    pub tags: Option<Vec<String>>,
    /// `None` = unchanged, `Some(None)` = clear, `Some(Some(v))` = set.
    pub resource: Option<Option<String>>,
}

// ============================================
// Knowledge Index models (source-backed embedded collections —
// see knowledge/runtime-resources/knowledge-indexes.md)
// ============================================

#[derive(Debug, Clone, FromRow, serde::Serialize)]
pub struct KnowledgeIndexRow {
    pub id: Uuid,
    pub org_id: i64,
    pub public_id: String,
    pub name: String,
    pub description: Option<String>,
    pub source_type: String,
    pub source_config: serde_json::Value,
    pub embedding_model_id: ModelId,
    pub vector_dim: Option<i32>,
    pub vector_namespace: Option<String>,
    pub owner_principal_id: Option<String>,
    pub resolved_owner_user_id: Option<Uuid>,
    pub status: String,
    pub sync_status: String,
    pub last_synced_at: Option<DateTime<Utc>>,
    pub last_sync_error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct CreateKnowledgeIndexRow {
    pub public_id: String,
    pub name: String,
    pub description: Option<String>,
    pub source_type: String,
    pub source_config: serde_json::Value,
    pub embedding_model_id: ModelId,
    /// Vector-store namespace assigned at creation. See `index_namespace`.
    pub vector_namespace: String,
    pub owner_principal_id: Option<String>,
    pub resolved_owner_user_id: Option<Uuid>,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateKnowledgeIndex {
    pub name: Option<String>,
    pub description: Option<Option<String>>,
    pub source_config: Option<serde_json::Value>,
    pub resolved_owner_user_id: UpdateField<Uuid>,
    /// Optional update to the embedding model. `None` = unchanged. The model is
    /// required on the index, so `Clear` is not representable.
    pub embedding_model_id: Option<ModelId>,
    /// Source or embedding changes invalidate the current projection and must
    /// atomically put the index back on the worker queue.
    pub enqueue_sync: bool,
    pub status: Option<String>,
}

#[derive(Debug, Clone, FromRow, serde::Serialize)]
pub struct KnowledgeIndexDocumentRow {
    pub id: Uuid,
    pub index_id: Uuid,
    pub public_id: String,
    pub source_uri: String,
    pub title: Option<String>,
    pub mime_type: Option<String>,
    pub content_hash: Option<String>,
    pub size_bytes: Option<i64>,
    pub chunk_count: i32,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, serde::Serialize)]
pub struct KnowledgeIndexChunkRow {
    pub id: Uuid,
    pub document_id: Uuid,
    pub index_id: Uuid,
    pub public_id: String,
    pub ordinal: i32,
    pub text: String,
    pub location: Option<serde_json::Value>,
    pub token_count: Option<i32>,
    pub created_at: DateTime<Utc>,
}

/// A chunk joined with its owning document's citation metadata, used to
/// hydrate `search_index` results from Postgres. See knowledge/runtime-resources/knowledge-indexes.md
/// ("Retrieval and citations").
#[derive(Debug, Clone)]
pub struct KnowledgeIndexChunkWithDocument {
    /// Chunk `public_id` (`kchk_…`). The stable citation id.
    pub chunk_public_id: String,
    /// Chunk passage text.
    pub text: String,
    /// Provenance within the document (line / char / page ranges).
    pub location: Option<serde_json::Value>,
    /// Position of the chunk within its document.
    pub ordinal: i32,
    /// Owning document stable locator.
    pub source_uri: String,
    /// Owning document title, if known.
    pub document_title: Option<String>,
}

/// A chunk to persist during a sync pass. The `public_id` (`kchk_…`) is the
/// stable citation id and the vector-store record key.
#[derive(Debug, Clone)]
pub struct CreateKnowledgeIndexChunkRow {
    pub public_id: String,
    pub ordinal: i32,
    pub text: String,
    pub location: Option<serde_json::Value>,
    pub token_count: Option<i32>,
}

/// A document plus its chunks to persist atomically during a sync pass.
#[derive(Debug, Clone)]
pub struct CreateKnowledgeIndexDocumentWithChunks {
    pub public_id: String,
    pub source_uri: String,
    pub title: Option<String>,
    pub mime_type: Option<String>,
    pub content_hash: Option<String>,
    pub size_bytes: Option<i64>,
    pub chunks: Vec<CreateKnowledgeIndexChunkRow>,
}

// ============================================
// Session Git models (libgit2 custom ODB/refdb over PostgreSQL)
// ============================================

/// Git object row from database (session-scoped)
#[derive(Debug, Clone, FromRow)]
pub struct SessionGitObjectRow {
    pub session_id: SessionId,
    pub oid: Vec<u8>,  // 20-byte SHA1
    pub obj_type: i16, // 1=commit, 2=tree, 3=blob, 4=tag
    pub size: i64,
    pub data: Vec<u8>,
}

/// Input for writing a git object
#[derive(Debug, Clone)]
pub struct CreateSessionGitObject {
    pub session_id: SessionId,
    pub oid: Vec<u8>,
    pub obj_type: i16,
    pub size: i64,
    pub data: Vec<u8>,
}

/// Git ref stored in PostgreSQL (session-scoped refdb)
#[derive(Debug, Clone, FromRow)]
pub struct SessionGitRefRow {
    pub session_id: SessionId,
    pub name: String,
    pub target: Vec<u8>, // 20-byte oid or symbolic target
    pub is_symbolic: bool,
}

/// Input for writing a git ref
#[derive(Debug, Clone)]
pub struct CreateSessionGitRef {
    pub session_id: SessionId,
    pub name: String,
    pub target: Vec<u8>,
    pub is_symbolic: bool,
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
#[derive(Debug, Clone, FromRow, serde::Serialize)]
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
#[derive(Debug, Clone, FromRow, serde::Serialize)]
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
    /// Audit domain: "management" or "agent"
    #[sqlx(default)]
    pub domain: String,
    /// Structured action (e.g. "management.member.invited")
    #[sqlx(default)]
    pub action: String,
    /// Target resource type (e.g. "harness", "agent", "member")
    #[sqlx(default)]
    pub target_type: Option<String>,
    /// Target resource ID (public ID or UUID)
    #[sqlx(default)]
    pub target_id: Option<String>,
}

/// Input for creating an audit log entry
#[derive(Debug, Clone)]
pub struct CreateAuditLogRow {
    pub org_id: i64,
    pub actor_id: Option<Uuid>,
    pub event_type: String,
    pub ip_address: Option<String>,
    pub metadata: serde_json::Value,
    pub domain: String,
    pub action: String,
    pub target_type: Option<String>,
    pub target_id: Option<String>,
}

/// Query parameters for listing audit logs
#[derive(Debug, Clone, Default)]
pub struct AuditLogQuery<'a> {
    pub org_id: i64,
    pub limit: i64,
    pub before: Option<DateTime<Utc>>,
    pub event_type_prefix: Option<&'a str>,
    pub actor_id: Option<Uuid>,
    pub domain: Option<&'a str>,
    pub action: Option<&'a str>,
}

/// Input for creating/updating a session key/value
#[derive(Debug, Clone)]
pub struct UpsertSessionKeyValue {
    pub session_id: SessionId,
    pub key: String,
    pub value: String,
}

/// Lightweight key info for listing (without value)
#[derive(Debug, Clone, FromRow, serde::Serialize)]
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
#[derive(Debug, Clone, FromRow, serde::Serialize)]
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

// ============================================
// Declarative capability models
// ============================================

#[derive(Debug, Clone, FromRow)]
pub struct DeclarativeCapabilityRow {
    pub id: Uuid,
    pub org_id: i64,
    pub public_id: String,
    pub name: String,
    pub display_name: Option<String>,
    pub description: String,
    pub status: String,
    pub definition: sqlx::types::JsonValue,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct CreateDeclarativeCapabilityRow {
    pub public_id: String,
    pub name: String,
    pub display_name: Option<String>,
    pub description: String,
    pub definition: serde_json::Value,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateDeclarativeCapability {
    pub name: Option<String>,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub status: Option<String>,
    pub definition: Option<serde_json::Value>,
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
// User Preference models
// ============================================

/// User preference (key/value) row from database
#[derive(Debug, Clone, FromRow)]
pub struct UserPreferenceRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub key: String,
    pub value: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
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
    /// Provider-specific metadata (e.g. Deno org slug for personal tokens)
    pub provider_metadata: Option<sqlx::types::JsonValue>,
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
    /// Provider-specific metadata (e.g. Deno org slug for personal tokens)
    pub provider_metadata: Option<serde_json::Value>,
}

/// Encrypted MCP OAuth credential bundle stored in session secrets.
#[derive(Debug, Clone)]
pub struct McpOAuthSessionCredentialsRow {
    pub access_token_encrypted: Vec<u8>,
    pub refresh_token_encrypted: Option<Vec<u8>>,
    pub expires_at_encrypted: Option<Vec<u8>>,
}

/// Atomic replacement for a session-scoped MCP OAuth grant.
#[derive(Debug, Clone)]
pub struct UpsertMcpOAuthSessionCredentials {
    pub session_id: SessionId,
    pub server_id: Uuid,
    pub access_token_encrypted: Vec<u8>,
    pub refresh_token_encrypted: Option<Vec<u8>>,
    pub expires_at_encrypted: Option<Vec<u8>>,
}

/// Atomic rotation of a persistent user OAuth connection.
#[derive(Debug, Clone)]
pub struct UpdateUserConnectionOAuthTokens {
    pub connection_id: Uuid,
    pub access_token_encrypted: Vec<u8>,
    pub refresh_token_encrypted: Vec<u8>,
    pub expires_at: Option<DateTime<Utc>>,
    pub scopes: Option<String>,
}

// ============================================
// Agent Identity Connection models
// ============================================

/// Agent identity connection row from database
#[derive(Debug, Clone, FromRow)]
pub struct AgentIdentityConnectionRow {
    pub id: Uuid,
    pub agent_identity_id: AgentIdentityId,
    pub provider: String,
    pub connection_type: String,
    pub provider_user_id: Option<String>,
    pub provider_username: Option<String>,
    pub access_token_encrypted: Option<Vec<u8>>,
    pub refresh_token_encrypted: Option<Vec<u8>>,
    pub scopes: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub installation_id: Option<i64>,
    /// Provider-specific metadata (e.g. Deno org slug for personal tokens)
    pub provider_metadata: Option<sqlx::types::JsonValue>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Input for creating an agent identity connection
#[derive(Debug, Clone)]
pub struct CreateAgentIdentityConnectionRow {
    pub agent_identity_id: AgentIdentityId,
    pub provider: String,
    pub connection_type: String,
    pub provider_user_id: Option<String>,
    pub provider_username: Option<String>,
    pub access_token_encrypted: Option<Vec<u8>>,
    pub refresh_token_encrypted: Option<Vec<u8>>,
    pub scopes: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub installation_id: Option<i64>,
    /// Provider-specific metadata (e.g. Deno org slug for personal tokens)
    pub provider_metadata: Option<serde_json::Value>,
}

// ============================================
// Session Schedule models
// ============================================

#[derive(Debug, Clone, FromRow, serde::Serialize)]
pub struct SessionScheduleRow {
    pub id: ScheduleId,
    pub public_id: String,
    pub org_id: i64,
    pub session_id: SessionId,
    pub owner_principal_id: PrincipalId,
    #[sqlx(default)]
    pub resolved_owner_user_id: Option<Uuid>,
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
    pub owner_principal_id: PrincipalId,
    pub resolved_owner_user_id: Option<Uuid>,
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
// Session resource registry models
// ============================================

#[derive(Debug, Clone, FromRow)]
pub struct SessionResourceRow {
    pub id: Uuid,
    pub session_id: SessionId,
    pub resource_id: String,
    pub kind: String,
    pub display_name: String,
    pub status: String,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct UpsertSessionResourceRow {
    pub session_id: SessionId,
    pub resource_id: String,
    pub kind: String,
    pub display_name: String,
    pub status: String,
    pub metadata: serde_json::Value,
}

// ============================================
// Session task models
// ============================================

/// Row in `session_tasks`. JSONB columns serialize the corresponding
/// `everruns_core::session_task` types; conversions live on this struct so
/// both backends share identical (de)serialization.
#[derive(Debug, Clone, FromRow)]
pub struct SessionTaskRow {
    pub id: String,
    pub session_id: SessionId,
    /// Root of the owning session's delegation tree (EVE-680). Denormalized
    /// from `sessions.root_session_id` at insert so `GET /v1/tasks` can filter a
    /// whole tree's work by a local column. DB-only — not part of the core
    /// `SessionTask`; `#[sqlx(default)]` so queries that omit it still map.
    #[sqlx(default)]
    pub root_session_id: Option<SessionId>,
    pub kind: String,
    pub display_name: String,
    pub spec: serde_json::Value,
    pub state: String,
    pub state_detail: Option<String>,
    pub progress: Option<serde_json::Value>,
    pub input_request: Option<serde_json::Value>,
    pub cancel_requested_at: Option<DateTime<Utc>>,
    pub summary: Option<String>,
    pub result_path: Option<String>,
    pub artifacts: serde_json::Value,
    pub error: Option<serde_json::Value>,
    pub attempt: i32,
    pub worker_id: Option<String>,
    pub heartbeat_at: Option<DateTime<Utc>>,
    pub links: serde_json::Value,
    pub wake_policy: String,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

impl SessionTaskRow {
    pub fn from_task(task: &everruns_core::SessionTask) -> anyhow::Result<Self> {
        use serde_json::to_value;
        Ok(Self {
            id: task.id.clone(),
            session_id: task.session_id,
            root_session_id: task.root_session_id,
            kind: task.kind.clone(),
            display_name: task.display_name.clone(),
            spec: task.spec.clone(),
            state: task.state.to_string(),
            state_detail: task.state_detail.clone(),
            progress: task.progress.as_ref().map(to_value).transpose()?,
            input_request: task.input_request.as_ref().map(to_value).transpose()?,
            cancel_requested_at: task.cancel_requested_at,
            summary: task.summary.clone(),
            result_path: task.result_path.clone(),
            artifacts: to_value(&task.artifacts)?,
            error: task.error.as_ref().map(to_value).transpose()?,
            attempt: task.attempt,
            worker_id: task.worker_id.clone(),
            heartbeat_at: task.heartbeat_at,
            links: to_value(&task.links)?,
            wake_policy: to_value(task.wake_policy)?
                .as_str()
                .unwrap_or("silent")
                .to_string(),
            created_at: task.created_at,
            started_at: task.started_at,
            finished_at: task.finished_at,
            updated_at: task.updated_at,
        })
    }

    pub fn to_task(&self) -> anyhow::Result<everruns_core::SessionTask> {
        use serde_json::from_value;
        Ok(everruns_core::SessionTask {
            id: self.id.clone(),
            session_id: self.session_id,
            root_session_id: self.root_session_id,
            kind: self.kind.clone(),
            display_name: self.display_name.clone(),
            spec: self.spec.clone(),
            state: everruns_core::SessionTaskState::from(self.state.as_str()),
            state_detail: self.state_detail.clone(),
            progress: self.progress.clone().map(from_value).transpose()?,
            input_request: self.input_request.clone().map(from_value).transpose()?,
            cancel_requested_at: self.cancel_requested_at,
            summary: self.summary.clone(),
            result_path: self.result_path.clone(),
            artifacts: from_value(self.artifacts.clone())?,
            error: self.error.clone().map(from_value).transpose()?,
            attempt: self.attempt,
            worker_id: self.worker_id.clone(),
            heartbeat_at: self.heartbeat_at,
            links: from_value(self.links.clone())?,
            wake_policy: from_value(serde_json::Value::String(self.wake_policy.clone()))
                .unwrap_or_default(),
            created_at: self.created_at,
            started_at: self.started_at,
            finished_at: self.finished_at,
            updated_at: self.updated_at,
        })
    }
}

/// Row in `session_task_messages`.
#[derive(Debug, Clone, FromRow)]
pub struct SessionTaskMessageRow {
    pub id: String,
    pub task_id: String,
    pub session_id: SessionId,
    pub direction: String,
    pub content: serde_json::Value,
    pub in_reply_to: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl SessionTaskMessageRow {
    pub fn to_message(&self) -> anyhow::Result<everruns_core::TaskMessage> {
        Ok(everruns_core::TaskMessage {
            id: self.id.clone(),
            task_id: self.task_id.clone(),
            direction: everruns_core::TaskMessageDirection::from(self.direction.as_str()),
            content: serde_json::from_value(self.content.clone())?,
            in_reply_to: self.in_reply_to.clone(),
            created_at: self.created_at,
        })
    }
}

/// Input for inserting a task message row.
#[derive(Debug, Clone)]
pub struct NewSessionTaskMessageRow {
    pub id: String,
    pub task_id: String,
    pub session_id: SessionId,
    pub direction: String,
    pub content: serde_json::Value,
    pub in_reply_to: Option<String>,
}

// ============================================
// Agent identity models (virtual principals)
// ============================================

#[derive(Debug, Clone, FromRow)]
pub struct AgentIdentityRow {
    pub id: AgentIdentityId,
    pub org_id: i64,
    pub name: String,
    pub description: Option<String>,
    pub avatar_url: Option<String>,
    pub locale: Option<String>,
    pub timezone: Option<String>,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct CreateAgentIdentityRow {
    pub org_id: i64,
    pub id: AgentIdentityId,
    pub name: String,
    pub description: Option<String>,
    pub avatar_url: Option<String>,
    pub locale: Option<String>,
    pub timezone: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateAgentIdentity {
    pub name: Option<String>,
    pub description: UpdateField<String>,
    pub avatar_url: UpdateField<String>,
    pub locale: UpdateField<String>,
    pub timezone: UpdateField<String>,
    pub status: Option<String>,
}

// ============================================
// Agent trigger models (agent-owned invocation triggers)
// ============================================

#[derive(Debug, Clone, FromRow)]
pub struct AgentTriggerRow {
    pub id: TriggerId,
    pub org_id: i64,
    pub agent_id: AgentId,
    pub trigger_type: String,
    pub config: serde_json::Value,
    pub enabled: bool,
    pub durable_schedule_id: Option<Uuid>,
    pub execution_harness_id: Option<HarnessId>,
    pub execution_owner_principal_id: Option<PrincipalId>,
    pub execution_resolved_owner_user_id: Option<Uuid>,
    pub execution_agent_identity_id: Option<AgentIdentityId>,
    pub execution_app_id: Option<Uuid>,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct CreateAgentTriggerRow {
    pub org_id: i64,
    pub id: TriggerId,
    pub agent_id: AgentId,
    pub trigger_type: String,
    pub config: serde_json::Value,
    pub enabled: bool,
    pub durable_schedule_id: Option<Uuid>,
    pub execution_harness_id: Option<HarnessId>,
    pub execution_owner_principal_id: Option<PrincipalId>,
    pub execution_resolved_owner_user_id: Option<Uuid>,
    pub execution_agent_identity_id: Option<AgentIdentityId>,
    pub execution_app_id: Option<Uuid>,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateAgentTrigger {
    pub trigger_type: Option<String>,
    pub config: Option<serde_json::Value>,
    pub enabled: Option<bool>,
    pub durable_schedule_id: UpdateField<Uuid>,
    pub status: Option<String>,
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
    pub agent_id: Option<Uuid>,
    #[sqlx(default)]
    pub agent_version_policy: String,
    #[sqlx(default)]
    pub agent_version_id: Option<Uuid>,
    pub agent_identity_id: Option<Uuid>,
    pub owner_principal_id: PrincipalId,
    #[sqlx(default)]
    pub resolved_owner_user_id: Option<Uuid>,
    pub channel_type: Option<String>,
    pub channel_config: serde_json::Value,
    pub channel_config_encrypted: Option<Vec<u8>>,
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
    pub agent_id: Option<Uuid>,
    pub agent_version_policy: String,
    pub agent_version_id: Option<Uuid>,
    pub agent_identity_id: Option<Uuid>,
    pub owner_principal_id: PrincipalId,
    pub resolved_owner_user_id: Option<Uuid>,
    pub channel_type: Option<String>,
    pub channel_config: serde_json::Value,
    /// Encrypted channel_config bytes (envelope-encrypted JSON).
    pub channel_config_encrypted: Option<Vec<u8>>,
}

/// Input for updating an app
#[derive(Debug, Clone, Default)]
pub struct UpdateApp {
    pub name: Option<String>,
    pub description: Option<String>,
    pub harness_id: Option<Uuid>,
    pub agent_id: Option<Uuid>,
    pub agent_version_policy: Option<String>,
    pub agent_version_id: UpdateField<Uuid>,
    pub agent_identity_id: UpdateField<Uuid>,
    pub owner_principal_id: Option<PrincipalId>,
    pub resolved_owner_user_id: UpdateField<Uuid>,
    pub channel_type: Option<String>,
    pub channel_config: Option<serde_json::Value>,
    /// Encrypted channel_config bytes (envelope-encrypted JSON).
    pub channel_config_encrypted: Option<Vec<u8>>,
    pub status: Option<String>,
    pub published_at: UpdateField<DateTime<Utc>>,
}

// ============================================
// App Channel models
// ============================================

/// App channel row from database
#[derive(Debug, Clone, FromRow)]
pub struct AppChannelRow {
    pub id: Uuid,
    pub app_id: Uuid,
    pub public_id: String,
    pub channel_type: String,
    pub channel_config: serde_json::Value,
    pub channel_config_encrypted: Option<Vec<u8>>,
    pub durable_schedule_id: Option<Uuid>,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ============================================
// Principal models
// ============================================

#[derive(Debug, Clone, FromRow)]
pub struct PrincipalRow {
    pub id: PrincipalId,
    pub public_id: String,
    pub org_id: i64,
    pub kind: String,
    pub subject_id: Option<Uuid>,
    pub parent_principal_id: Option<PrincipalId>,
    pub resolved_user_id: Option<Uuid>,
    pub metadata: serde_json::Value,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct CreatePrincipalRow {
    pub id: PrincipalId,
    pub org_id: i64,
    pub kind: String,
    pub subject_id: Option<Uuid>,
    pub parent_principal_id: Option<PrincipalId>,
    pub resolved_user_id: Option<Uuid>,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Default)]
pub struct UpdatePrincipalRow {
    pub parent_principal_id: UpdateField<PrincipalId>,
    pub resolved_user_id: UpdateField<Uuid>,
    pub metadata: Option<serde_json::Value>,
    pub status: Option<String>,
}

/// Input for creating an app channel
#[derive(Debug, Clone)]
pub struct CreateAppChannelRow {
    pub public_id: String,
    pub channel_type: String,
    pub channel_config: serde_json::Value,
    pub channel_config_encrypted: Option<Vec<u8>>,
    pub durable_schedule_id: Option<Uuid>,
    pub enabled: bool,
}

/// Input for updating an app channel
#[derive(Debug, Clone, Default)]
pub struct UpdateAppChannel {
    pub channel_type: Option<String>,
    pub channel_config: Option<serde_json::Value>,
    pub channel_config_encrypted: Option<Vec<u8>>,
    pub durable_schedule_id: UpdateField<Uuid>,
    pub enabled: Option<bool>,
}

// ============================================
// Eval models
// ============================================

/// Eval row from database
#[derive(Debug, Clone, FromRow)]
pub struct EvalRow {
    pub id: Uuid,
    pub org_id: i64,
    pub public_id: String,
    pub name: String,
    pub description: Option<String>,
    pub target: Option<serde_json::Value>,
    pub model_override: Option<String>,
    pub tags: Vec<String>,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

/// Input for creating an eval
#[derive(Debug, Clone)]
pub struct CreateEvalRow {
    pub public_id: String,
    pub name: String,
    pub description: Option<String>,
    pub target: Option<serde_json::Value>,
    pub model_override: Option<String>,
    pub tags: Vec<String>,
}

/// Input for updating an eval
#[derive(Debug, Clone, Default)]
pub struct UpdateEvalRow {
    pub name: Option<String>,
    pub description: Option<String>,
    pub target: Option<serde_json::Value>,
    pub model_override: Option<String>,
    pub tags: Option<Vec<String>>,
    pub status: Option<String>,
}

/// Eval case row from database
#[derive(Debug, Clone, FromRow)]
pub struct EvalCaseRow {
    pub id: Uuid,
    pub eval_id: Uuid,
    pub public_id: String,
    pub name: String,
    pub description: Option<String>,
    pub target: Option<serde_json::Value>,
    pub tags: Vec<String>,
    pub conversation: serde_json::Value,
    pub post: Option<serde_json::Value>,
    pub artifacts: Option<serde_json::Value>,
    pub scorers: serde_json::Value,
    pub max_turns: Option<i32>,
    pub timeout_seconds: Option<i32>,
    pub position: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Input for creating an eval case
#[derive(Debug, Clone)]
pub struct CreateEvalCaseRow {
    pub public_id: String,
    pub name: String,
    pub description: Option<String>,
    pub target: Option<serde_json::Value>,
    pub tags: Vec<String>,
    pub conversation: serde_json::Value,
    pub post: Option<serde_json::Value>,
    pub artifacts: Option<serde_json::Value>,
    pub scorers: serde_json::Value,
    pub max_turns: Option<i32>,
    pub timeout_seconds: Option<i32>,
    pub position: i32,
}

/// Input for updating an eval case
#[derive(Debug, Clone, Default)]
pub struct UpdateEvalCaseRow {
    pub name: Option<String>,
    pub description: Option<String>,
    pub target: Option<serde_json::Value>,
    pub tags: Option<Vec<String>>,
    pub conversation: Option<serde_json::Value>,
    pub post: Option<serde_json::Value>,
    pub artifacts: Option<serde_json::Value>,
    pub scorers: Option<serde_json::Value>,
    pub max_turns: Option<i32>,
    pub timeout_seconds: Option<i32>,
    pub position: Option<i32>,
}

/// Eval run row from database
#[derive(Debug, Clone, FromRow)]
pub struct EvalRunRow {
    pub id: Uuid,
    pub eval_id: Uuid,
    pub org_id: i64,
    pub public_id: String,
    pub target: Option<serde_json::Value>,
    pub model_override: Option<String>,
    pub filter_tags: Option<Vec<String>>,
    pub status: String,
    pub triggered_by: String,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub summary: Option<serde_json::Value>,
    /// 'internal' (everruns executed) or 'external' (imported). See migration 086.
    pub source: String,
    /// External system's run id: cross-eval group key and idempotency key.
    pub source_run_id: Option<String>,
    /// Open-vocab provenance for external runs (system, version, url, labels).
    pub attribution: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A read-only share token for an eval run (migration 091). The raw token is
/// never stored — only its hash. `revoked_at`/`expires_at` disable a link.
#[derive(Debug, Clone, FromRow)]
pub struct EvalRunShareTokenRow {
    pub id: Uuid,
    pub org_id: i64,
    pub public_id: String,
    pub eval_run_id: Uuid,
    pub token_hash: String,
    pub token_prefix: String,
    pub created_by: Option<Uuid>,
    pub expires_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Input for minting an eval-run share token.
#[derive(Debug, Clone)]
pub struct CreateEvalRunShareTokenRow {
    pub public_id: String,
    pub org_id: i64,
    pub eval_run_id: Uuid,
    pub token_hash: String,
    pub token_prefix: String,
    pub created_by: Option<Uuid>,
    pub expires_at: Option<DateTime<Utc>>,
}

/// Input for creating an eval run
#[derive(Debug, Clone)]
pub struct CreateEvalRunRow {
    pub public_id: String,
    pub eval_id: Uuid,
    pub target: Option<serde_json::Value>,
    pub model_override: Option<String>,
    pub filter_tags: Option<Vec<String>>,
    pub triggered_by: String,
}

/// Typed failures from `create_eval_run_with_case_results`. Returned (boxed in
/// `anyhow`) so the service layer can `downcast_ref` to map them to HTTP 400
/// instead of relying on brittle error-message substring matching.
#[derive(Debug, thiserror::Error)]
pub enum CreateEvalRunError {
    #[error("Too many concurrent eval runs: org has {active} active runs (limit {limit})")]
    TooManyConcurrentRuns { active: i64, limit: usize },
    #[error("Eval run too large: {cases} cases exceeds the per-run limit of {limit}")]
    TooManyCases { cases: usize, limit: usize },
    #[error("no eval target configured — set a target on the eval, case, or run")]
    NoTarget,
}

/// Eval case result row from database
#[derive(Debug, Clone, FromRow)]
pub struct EvalCaseResultRow {
    pub id: Uuid,
    pub eval_run_id: Uuid,
    pub eval_case_id: Uuid,
    pub public_id: String,
    pub session_id: Option<Uuid>,
    pub target: Option<serde_json::Value>,
    pub target_snapshot: Option<serde_json::Value>,
    pub status: String,
    pub scores: Option<serde_json::Value>,
    pub metadata: Option<serde_json::Value>,
    pub turns: Option<i32>,
    pub latency_ms: Option<i64>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub error_message: Option<String>,
    pub artifacts: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Input for creating an eval case result
#[derive(Debug, Clone)]
pub struct CreateEvalCaseResultRow {
    pub public_id: String,
    pub eval_run_id: Uuid,
    pub eval_case_id: Uuid,
    pub target: Option<serde_json::Value>,
    pub target_snapshot: Option<serde_json::Value>,
    pub artifacts: Option<serde_json::Value>,
}

/// Input for importing one externally-executed eval run (a single eval's worth
/// of a Mira-style run group). The storage layer upserts the eval and its cases
/// by name, replaces any prior run sharing `source_run_id`, then writes a
/// completed external run with fully-populated results. See `import_eval_run`.
#[derive(Debug, Clone)]
pub struct ImportEvalRunInput {
    pub eval_name: String,
    pub eval_description: Option<String>,
    pub eval_tags: Vec<String>,
    pub run_public_id: String,
    pub source: String,
    pub source_run_id: String,
    pub attribution: Option<serde_json::Value>,
    pub triggered_by: String,
    pub summary: Option<serde_json::Value>,
    pub cases: Vec<ImportEvalCaseInput>,
}

/// One case-result within an imported external run. The case is identity-only
/// (name + optional display conversation); everruns never re-executes it, so
/// scorers are left empty. Transcript and open-vocab metrics ride inside
/// `metadata` rather than dedicated columns.
#[derive(Debug, Clone)]
pub struct ImportEvalCaseInput {
    pub case_name: String,
    pub case_description: Option<String>,
    pub conversation: serde_json::Value,
    pub target_snapshot: Option<serde_json::Value>,
    pub status: String,
    pub scores: Option<serde_json::Value>,
    pub metadata: Option<serde_json::Value>,
    pub turns: Option<i32>,
    pub latency_ms: Option<i64>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub error_message: Option<String>,
    pub artifacts: Option<serde_json::Value>,
}

/// Input for updating an eval case result
#[derive(Debug, Clone, Default)]
pub struct UpdateEvalCaseResultRow {
    pub session_id: Option<Uuid>,
    pub target: Option<serde_json::Value>,
    pub target_snapshot: Option<serde_json::Value>,
    pub status: Option<String>,
    pub scores: Option<serde_json::Value>,
    pub metadata: Option<serde_json::Value>,
    pub turns: Option<i32>,
    pub latency_ms: Option<i64>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub error_message: Option<String>,
    pub artifacts: Option<serde_json::Value>,
}

// ============================================================================
// Agent health check models (knowledge/evaluation/agent-checks.md, tier-3)
// ============================================================================

/// Agent health check run row from database.
#[derive(Debug, Clone, FromRow)]
pub struct AgentHealthCheckRunRow {
    pub id: Uuid,
    pub org_id: i64,
    pub public_id: String,
    pub agent_id: Option<Uuid>,
    pub config_hash: String,
    pub model_id: Option<String>,
    pub status: String,
    pub summary: Option<serde_json::Value>,
    pub results: Option<serde_json::Value>,
    pub error_message: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Input for creating an agent health check run.
#[derive(Debug, Clone)]
pub struct CreateAgentHealthCheckRunRow {
    pub public_id: String,
    pub agent_id: Option<Uuid>,
    pub config_hash: String,
    pub model_id: Option<String>,
}

/// Input for updating an agent health check run. `None` fields are left
/// unchanged; `started_at`/`completed_at` are set from `status` transitions.
#[derive(Debug, Clone, Default)]
pub struct UpdateAgentHealthCheckRunRow {
    pub status: Option<String>,
    pub summary: Option<serde_json::Value>,
    pub results: Option<serde_json::Value>,
    pub error_message: Option<String>,
}

// ============================================================================
// Eval run dataset models (async dataset export — knowledge/evaluation/dataset-export.md)
// ============================================================================

/// Async dataset-export handle row from database.
#[derive(Debug, Clone, FromRow)]
pub struct EvalRunDatasetRow {
    pub id: Uuid,
    pub org_id: i64,
    pub public_id: String,
    pub eval_run_id: Option<Uuid>,
    pub request: serde_json::Value,
    pub status: String,
    pub body: Option<String>,
    pub record_count: Option<i64>,
    pub error_message: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Input for creating an eval run dataset export handle.
#[derive(Debug, Clone)]
pub struct CreateEvalRunDatasetRow {
    pub public_id: String,
    pub eval_run_id: Uuid,
    pub request: serde_json::Value,
}

/// Input for updating an eval run dataset export handle. `None` fields are left
/// unchanged; `started_at`/`completed_at` are set from `status` transitions.
#[derive(Debug, Clone, Default)]
pub struct UpdateEvalRunDatasetRow {
    pub status: Option<String>,
    pub body: Option<String>,
    pub record_count: Option<i64>,
    pub error_message: Option<String>,
}

/// Org-configurable agent check rule (knowledge/evaluation/agent-checks.md, phase 4).
#[derive(Debug, Clone, FromRow)]
pub struct AgentCheckRuleRow {
    pub id: Uuid,
    pub org_id: i64,
    pub rule_id: String,
    pub kind: String,
    pub enabled: bool,
    pub severity_override: Option<String>,
    pub config: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Upsert input for an agent check rule (keyed by org_id + rule_id).
#[derive(Debug, Clone)]
pub struct UpsertAgentCheckRuleRow {
    pub rule_id: String,
    pub kind: String,
    pub enabled: bool,
    pub severity_override: Option<String>,
    pub config: Option<serde_json::Value>,
}

// ============================================================================
// Observer models (online scoring — see knowledge/evaluation/online-evals.md)
// ============================================================================

/// Observer row from database
#[derive(Debug, Clone, FromRow)]
pub struct ObserverRow {
    pub id: Uuid,
    pub org_id: i64,
    pub public_id: String,
    pub name: String,
    pub description: Option<String>,
    pub match_config: serde_json::Value,
    pub sampling_rate: f64,
    pub scorers: serde_json::Value,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
}

/// Input for creating an observer
#[derive(Debug, Clone)]
pub struct CreateObserverRow {
    pub public_id: String,
    pub name: String,
    pub description: Option<String>,
    pub match_config: serde_json::Value,
    pub sampling_rate: f64,
    pub scorers: serde_json::Value,
}

/// Input for updating an observer
#[derive(Debug, Clone, Default)]
pub struct UpdateObserverRow {
    pub name: Option<String>,
    pub description: Option<String>,
    pub match_config: Option<serde_json::Value>,
    pub sampling_rate: Option<f64>,
    pub scorers: Option<serde_json::Value>,
    pub status: Option<String>,
}

/// Trace score row from database. Pending rows double as the scoring queue.
#[derive(Debug, Clone, FromRow)]
pub struct TraceScoreRow {
    pub id: Uuid,
    pub org_id: i64,
    pub public_id: String,
    pub observer_id: Uuid,
    pub scorer_key: String,
    pub session_id: Uuid,
    pub turn_id: String,
    pub agent_id: Option<Uuid>,
    pub agent_version_id: Option<Uuid>,
    pub harness_id: Option<Uuid>,
    pub status: String,
    pub attempts: i32,
    pub pass: Option<bool>,
    pub value: Option<f64>,
    pub label: Option<String>,
    pub reason: Option<String>,
    pub judge_input_tokens: Option<i64>,
    pub judge_output_tokens: Option<i64>,
    pub judge_cost_usd: Option<f64>,
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Input for enqueueing a trace score (one per matched scorer).
#[derive(Debug, Clone)]
pub struct CreateTraceScoreRow {
    pub public_id: String,
    pub observer_id: Uuid,
    pub scorer_key: String,
    pub session_id: Uuid,
    pub turn_id: String,
    pub agent_id: Option<Uuid>,
    pub agent_version_id: Option<Uuid>,
    pub harness_id: Option<Uuid>,
}

/// Terminal result written back by the scoring worker.
#[derive(Debug, Clone, Default)]
pub struct CompleteTraceScoreRow {
    /// `completed`, `errored`, or `skipped`.
    pub status: String,
    pub pass: Option<bool>,
    pub value: Option<f64>,
    pub label: Option<String>,
    pub reason: Option<String>,
    pub judge_input_tokens: Option<i64>,
    pub judge_output_tokens: Option<i64>,
    pub judge_cost_usd: Option<f64>,
    pub error_message: Option<String>,
}

/// Filters for listing trace scores.
#[derive(Debug, Clone, Default)]
pub struct ListTraceScoresParams {
    pub observer_id: Uuid,
    pub session_id: Option<Uuid>,
    pub limit: i64,
    pub offset: i64,
}

// ============================================================================
// Budget models
// ============================================================================

/// Budget row (maps to `budgets` table).
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct BudgetRow {
    pub id: Uuid,
    pub org_id: i64,
    pub subject_type: String,
    pub subject_id: String,
    pub currency: String,
    pub limit: f64,
    pub soft_limit: Option<f64>,
    pub balance: f64,
    pub period: Option<sqlx::types::JsonValue>,
    /// When the current `period` window started. NULL for one-shot budgets.
    /// Set on insert when `period` is provided, refreshed when the period
    /// rolls over.
    #[sqlx(default)]
    pub period_started_at: Option<DateTime<Utc>>,
    pub metadata: Option<sqlx::types::JsonValue>,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Input for creating a budget.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct CreateBudgetRow {
    pub org_id: i64,
    pub subject_type: String,
    pub subject_id: String,
    pub currency: String,
    pub limit: f64,
    pub soft_limit: Option<f64>,
    pub period: Option<serde_json::Value>,
    pub metadata: Option<serde_json::Value>,
}

/// Input for updating a budget.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct UpdateBudgetRow {
    pub limit: Option<f64>,
    pub soft_limit: Option<Option<f64>>,
    pub status: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

/// Immutable activity journal row (maps to `usage_journal` table).
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct UsageJournalRow {
    pub id: Uuid,
    pub org_id: i64,
    pub kind: String,
    pub source_type: Option<String>,
    pub source_id: Option<String>,
    pub event_id: Option<Uuid>,
    pub session_id: Option<Uuid>,
    pub turn_id: Option<Uuid>,
    pub user_id: Option<Uuid>,
    pub principal_id: Option<Uuid>,
    pub agent_id: Option<Uuid>,
    pub harness_id: Option<Uuid>,
    pub measures: sqlx::types::JsonValue,
    pub metadata: sqlx::types::JsonValue,
    pub created_at: DateTime<Utc>,
}

/// Input for creating a journal row.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct CreateUsageJournalRow {
    pub org_id: i64,
    pub kind: String,
    pub source_type: Option<String>,
    pub source_id: Option<String>,
    pub event_id: Option<Uuid>,
    pub session_id: Option<Uuid>,
    pub turn_id: Option<Uuid>,
    pub user_id: Option<Uuid>,
    pub principal_id: Option<Uuid>,
    pub agent_id: Option<Uuid>,
    pub harness_id: Option<Uuid>,
    pub measures: serde_json::Value,
    pub metadata: serde_json::Value,
}

/// Rated ledger row (maps to `usage_ledger` table).
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct UsageLedgerRow {
    pub id: Uuid,
    pub journal_id: Uuid,
    pub budget_id: Option<Uuid>,
    pub org_id: i64,
    pub session_id: Option<Uuid>,
    pub user_id: Option<Uuid>,
    pub principal_id: Option<Uuid>,
    pub agent_id: Option<Uuid>,
    pub harness_id: Option<Uuid>,
    pub currency: String,
    pub amount: f64,
    pub meter_source: String,
    pub ref_type: Option<String>,
    pub ref_id: Option<Uuid>,
    pub description: Option<String>,
    pub rating_metadata: Option<sqlx::types::JsonValue>,
    pub created_at: DateTime<Utc>,
}

/// Input for creating a rated ledger row.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct CreateUsageLedgerRow {
    pub journal_id: Uuid,
    pub budget_id: Option<Uuid>,
    pub org_id: i64,
    pub session_id: Option<Uuid>,
    pub user_id: Option<Uuid>,
    pub principal_id: Option<Uuid>,
    pub agent_id: Option<Uuid>,
    pub harness_id: Option<Uuid>,
    pub currency: String,
    pub amount: f64,
    pub meter_source: String,
    pub ref_type: Option<String>,
    pub ref_id: Option<Uuid>,
    pub description: Option<String>,
    pub rating_metadata: Option<serde_json::Value>,
}

/// Compatibility input for callers that still think in budget-specific postings.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct CreateBudgetLedgerRow {
    pub budget_id: Uuid,
    pub amount: f64,
    pub meter_source: String,
    pub ref_type: Option<String>,
    pub ref_id: Option<Uuid>,
    pub session_id: Option<Uuid>,
    pub description: Option<String>,
}

/// Compatibility alias for budget-scoped ledger reads.
pub type BudgetLedgerRow = UsageLedgerRow;

// ============================================================================
// Machine payment models
// ============================================================================

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct PaymentAccountRow {
    pub id: Uuid,
    pub org_id: i64,
    pub owner_type: String,
    pub owner_id: String,
    pub rail: String,
    pub label: String,
    pub public_address: Option<String>,
    pub credential_encrypted: Option<Vec<u8>>,
    pub status: String,
    pub metadata: sqlx::types::JsonValue,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct CreatePaymentAccountRow {
    pub owner_type: String,
    pub owner_id: String,
    pub rail: String,
    pub label: String,
    pub public_address: Option<String>,
    pub credential_encrypted: Option<Vec<u8>>,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct UpdatePaymentAccountRow {
    pub label: Option<String>,
    pub public_address: Option<Option<String>>,
    pub credential_encrypted: Option<Vec<u8>>,
    pub status: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct PaymentPolicyRow {
    pub id: Uuid,
    pub org_id: i64,
    pub payment_account_id: Uuid,
    pub subject_type: String,
    pub subject_id: String,
    pub allowed_capabilities: Vec<String>,
    pub allowed_hosts: Vec<String>,
    pub rail_preference: Vec<String>,
    pub max_amount_usd_per_request: Option<f64>,
    pub max_amount_usd_per_turn: Option<f64>,
    pub max_amount_usd_per_day: Option<f64>,
    pub require_approval_above_usd: Option<f64>,
    pub status: String,
    pub metadata: sqlx::types::JsonValue,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct CreatePaymentPolicyRow {
    pub payment_account_id: Uuid,
    pub subject_type: String,
    pub subject_id: String,
    pub allowed_capabilities: Vec<String>,
    pub allowed_hosts: Vec<String>,
    pub rail_preference: Vec<String>,
    pub max_amount_usd_per_request: Option<f64>,
    pub max_amount_usd_per_turn: Option<f64>,
    pub max_amount_usd_per_day: Option<f64>,
    pub require_approval_above_usd: Option<f64>,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct UpdatePaymentPolicyRow {
    pub allowed_capabilities: Option<Vec<String>>,
    pub allowed_hosts: Option<Vec<String>>,
    pub rail_preference: Option<Vec<String>>,
    pub max_amount_usd_per_request: Option<Option<f64>>,
    pub max_amount_usd_per_turn: Option<Option<f64>>,
    pub max_amount_usd_per_day: Option<Option<f64>>,
    pub require_approval_above_usd: Option<Option<f64>>,
    pub status: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct PaymentAttemptRow {
    pub id: Uuid,
    pub org_id: i64,
    pub payment_account_id: Option<Uuid>,
    pub session_id: Option<Uuid>,
    pub capability: String,
    pub operation: String,
    pub rail: Option<String>,
    pub amount_usd: f64,
    pub currency: String,
    pub target_url: String,
    pub request_hash: Option<String>,
    pub status: String,
    pub error_message: Option<String>,
    pub receipt: sqlx::types::JsonValue,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct CreatePaymentAttemptRow {
    pub payment_account_id: Option<Uuid>,
    pub session_id: Option<Uuid>,
    pub capability: String,
    pub operation: String,
    pub rail: Option<String>,
    pub amount_usd: f64,
    pub currency: String,
    pub target_url: String,
    pub request_hash: Option<String>,
    pub status: String,
    pub error_message: Option<String>,
    pub receipt: serde_json::Value,
}

// ============================================
// Plugin Marketplace models
// ============================================

#[derive(Debug, Clone, FromRow)]
pub struct PluginMarketplaceRow {
    pub id: Uuid,
    pub org_id: i64,
    pub public_id: String,
    pub name: String,
    pub source_type: String,
    pub source: serde_json::Value,
    pub status: String,
    pub catalog: Option<serde_json::Value>,
    pub last_synced_at: Option<DateTime<Utc>>,
    pub last_synced_sha: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CreatePluginMarketplaceRow {
    pub public_id: String,
    pub name: String,
    pub source_type: String,
    pub source: serde_json::Value,
}

#[derive(Debug, Clone, Default)]
pub struct UpdatePluginMarketplace {
    pub name: Option<String>,
    pub source_type: Option<String>,
    pub source: Option<serde_json::Value>,
    pub status: Option<String>,
    pub catalog: Option<serde_json::Value>,
    pub last_synced_at: Option<DateTime<Utc>>,
    pub last_synced_sha: Option<Option<String>>,
}

// ============================================
// Plugin Install models
// ============================================

#[derive(Debug, Clone, FromRow)]
pub struct PluginInstallRow {
    pub id: Uuid,
    pub org_id: i64,
    pub public_id: String,
    pub name: String,
    pub marketplace_id: Option<Uuid>,
    pub source: serde_json::Value,
    pub version: Option<String>,
    pub pinned_sha: Option<String>,
    pub manifest: serde_json::Value,
    pub definition: serde_json::Value,
    pub warnings: serde_json::Value,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CreatePluginInstallRow {
    pub public_id: String,
    pub name: String,
    pub marketplace_id: Option<Uuid>,
    pub source: serde_json::Value,
    pub version: Option<String>,
    pub pinned_sha: Option<String>,
    pub manifest: serde_json::Value,
    pub definition: serde_json::Value,
    pub warnings: serde_json::Value,
}

#[derive(Debug, Clone, Default)]
pub struct UpdatePluginInstall {
    pub status: Option<String>,
    pub source: Option<serde_json::Value>,
    pub version: Option<String>,
    pub pinned_sha: Option<String>,
    pub manifest: Option<serde_json::Value>,
    pub definition: Option<serde_json::Value>,
    pub warnings: Option<serde_json::Value>,
}

/// Minimal projection returned by `list_unreconciled_llm_generations`.
/// Contains only the fields needed to perform a reconciliation lookup.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct UnreconciledGeneration {
    pub id: uuid::Uuid,
    pub org_id: i64,
    pub provider_response_id: String,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct CompactionCheckpointRow {
    pub id: uuid::Uuid,
    pub session_id: everruns_provider::typed_id::SessionId,
    pub source_sequence: i32,
    pub provider_type: String,
    pub model: String,
    pub format_version: i32,
    pub payload_encrypted: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct InstallCompactionCheckpointRow {
    pub id: uuid::Uuid,
    pub session_id: everruns_provider::typed_id::SessionId,
    pub source_sequence: i32,
    pub provider_type: String,
    pub model: String,
    pub format_version: i32,
    pub payload_encrypted: Vec<u8>,
}

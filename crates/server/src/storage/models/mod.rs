// Database rows not yet moved next to their repository.
//
// Decision: a row lives beside the repository that reads and writes it
// (`repositories/<entity>/rows.rs`, re-exported as `crate::storage::*`). The
// rows still here are the widely shared ones (organizations, users and auth,
// agents, harnesses, sessions, events, providers, MCP servers, files,
// connections, principals). New callers name rows through `crate::storage::*`.
// The `storage::models` path itself is kept for saas (`CreateUserRow`,
// `UpdateUser`, `UserRow`, `CreateRefreshTokenRow`, `CreateOrganizationRow`,
// `CreatePrincipalRow`, `CreateSessionRow`, `SessionListFilters`); remove the
// path after adoption.
pub use super::session_turn_claim::*;

use crate::kernel_imports::{
    contracts::driver_registry::ServiceKind, contracts::typed_id::AgentId,
    contracts::typed_id::EventId, contracts::typed_id::FileId, contracts::typed_id::HarnessId,
    contracts::typed_id::ImageId, contracts::typed_id::McpServerId, contracts::typed_id::ModelId,
    contracts::typed_id::PrincipalId, contracts::typed_id::ProviderId,
    contracts::typed_id::SessionId, contracts::typed_id::SessionParticipantId,
    contracts::typed_id::VirtualUserId,
};
use crate::records::{SessionParticipant, SessionParticipantKind, SessionParticipantRole};
use crate::storage::UpdateField;
use chrono::{DateTime, Utc};
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

// Organization models

/// Organization row from database
#[derive(Debug, Clone, FromRow, serde::Serialize, everruns_server_macros::Columns)]
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
#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
pub struct OrganizationMemberRow {
    pub org_id: i64,
    pub user_id: Uuid,
    pub role: String,
    pub created_at: DateTime<Utc>,
}
/// Result of adding an organization member under the member-capacity guard.
#[derive(Debug, Clone)]
pub enum AddOrganizationMemberOutcome {
    Added(OrganizationMemberRow),
    AlreadyMember(OrganizationMemberRow),
    MemberLimitReached,
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
#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
pub struct OrganizationSettingsRow {
    pub org_id: i64,
    pub default_model_id: Option<ModelId>,
    pub default_harness_id: Option<HarnessId>,
    pub base_harness_id: Option<HarnessId>,
    /// Org-level default provider per service (EVE-569); empty means none.
    pub default_provider_per_service: sqlx::types::Json<ServiceProviderDefaults>,
    pub system_decisions: String, // `SystemDecisions` as stored
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateOrganizationSettings {
    pub default_model_id: UpdateField<ModelId>,
    pub default_harness_id: UpdateField<HarnessId>,
    pub base_harness_id: UpdateField<HarnessId>,
    /// Replaces the whole per-service map: `Set` overwrites, `Clear` empties.
    pub default_provider_per_service: UpdateField<ServiceProviderDefaults>,
    pub system_decisions: Option<super::SystemDecisions>, // `None` keeps it
}

/// Organization task webhook row from database
#[derive(Debug, Clone, sqlx::FromRow, everruns_server_macros::Columns)]
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
#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
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

/// Actionable invitation with organization display data.
#[derive(Debug, Clone, FromRow)]
pub struct OutstandingOrgInvitationRow {
    pub public_id: String,
    pub org_id: i64,
    pub org_name: String,
    pub email: String,
    pub role: String,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

/// Result of atomically claiming an invitation and ensuring its membership.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcceptOrgInvitationOutcome {
    Accepted { org_id: i64, role: String },
    MemberLimitReached,
    NotActionable,
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
#[derive(Debug, Clone, FromRow, serde::Serialize, everruns_server_macros::Columns)]
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
#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
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
#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
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
#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
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
#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
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
#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
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
#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
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

// ==================== Agent models (configuration for agentic loop) ====================

#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
pub struct AgentRow {
    pub id: AgentId,
    pub public_id: String,
    pub org_id: i64,
    pub name: String,
    #[sqlx(default)]
    pub display_name: Option<String>,
    pub description: Option<String>,
    /// Markdown intro for fresh Platform Chat threads. Wins over the harness
    /// intro. Hidden once the user inputs.
    #[sqlx(default)]
    pub intro_markdown: Option<String>,
    /// One-line description in simplified Markdown. Wins over the harness value.
    #[sqlx(default)]
    pub short_description: Option<String>,
    /// Conversation starters (JSONB in DB). Win over harness starters when
    /// non-empty.
    #[sqlx(default)]
    pub starters: serde_json::Value,
    pub system_prompt: String,
    pub default_model_id: Option<ModelId>,
    pub harness_id: HarnessId,
    /// Whether `harness_id` was explicitly selected or materialized from the
    /// organization default for backward-compatible API responses.
    #[sqlx(default)]
    pub harness_source: String,
    /// Lazily-created identity principal subject for this agent (EVE-758).
    /// NULL until the agent first acts unattended (e.g. an agent trigger fire),
    /// at which point an `virtual_users` row is created and linked so the
    /// agent owns its unattended sessions as itself. Storage-only: intentionally
    /// not surfaced on the public `crate::records::Agent` API.
    #[sqlx(default)]
    pub virtual_user_id: Option<VirtualUserId>,
    #[sqlx(default)]
    pub forked_from_agent_id: Option<AgentId>,
    #[sqlx(default)]
    pub root_agent_id: Option<AgentId>,
    pub tags: Vec<String>,
    pub status: String,
    /// Incident switch: when true no endpoint on this agent accepts traffic,
    /// without rewriting the per-endpoint status it must restore to (EVE-1007).
    pub exposures_suspended: bool,
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
    #[sqlx(default)]
    pub environments: Option<serde_json::Value>,
    /// Current avatar (`agent_avatars.id`), `None` when the agent has none.
    #[sqlx(default)]
    pub avatar_id: Option<Uuid>,
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
    /// Cumulative best-effort cost in USD across all sessions (actual if present, else estimated)
    #[sqlx(default)]
    pub total_cost_usd: f64,
}

#[derive(Debug, Clone)]
pub struct CreateAgentRow {
    pub public_id: String,
    pub name: String,
    pub display_name: Option<String>,
    pub description: Option<String>,
    /// Markdown intro for fresh Platform Chat threads (agent wins).
    pub intro_markdown: Option<String>,
    /// One-line description in simplified Markdown (agent wins).
    pub short_description: Option<String>,
    /// Conversation starters (JSONB in DB, agent wins when non-empty).
    pub starters: serde_json::Value,
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
    pub environments: Option<serde_json::Value>,
    /// Platform-supplied agent. Only org bootstrap sets this; every API-facing
    /// creation path leaves it false.
    pub is_built_in: bool,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateAgent {
    pub virtual_user_id: Option<Option<VirtualUserId>>,
    pub name: Option<String>,
    pub display_name: Option<String>,
    pub description: Option<String>,
    /// Markdown intro for fresh Platform Chat threads (agent wins).
    pub intro_markdown: Option<Option<String>>,
    /// One-line description in simplified Markdown (agent wins).
    pub short_description: Option<Option<String>>,
    /// Conversation starters (JSONB); None = leave unchanged.
    pub starters: Option<serde_json::Value>,
    pub system_prompt: Option<String>,
    pub default_model_id: Option<ModelId>,
    pub harness_id: Option<HarnessId>,
    pub harness_source: Option<String>,
    pub forked_from_agent_id: Option<AgentId>,
    pub root_agent_id: Option<AgentId>,
    pub tags: Option<Vec<String>>,
    pub status: Option<String>,
    /// Agent-level exposure incident switch (EVE-1007).
    pub exposures_suspended: Option<bool>,
    pub initial_files: Option<serde_json::Value>,
    pub tools: Option<serde_json::Value>,
    pub mcp_servers: Option<serde_json::Value>,
    pub network_access: Option<Option<serde_json::Value>>,
    /// None = don't change, Some(None) = set to NULL, Some(Some(v)) = set to v
    pub max_iterations: Option<Option<i32>>,
    /// Request-level parallel tool calling preference (EVE-598).
    /// None = don't change, Some(None) = set to NULL, Some(Some(v)) = set to v
    pub parallel_tool_calls: Option<Option<bool>>,
    pub environments: Option<Option<serde_json::Value>>,
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
    /// Display glyph name rendered by the UI. Set from built-in definitions.
    #[sqlx(default)]
    pub icon: Option<String>,
    pub description: Option<String>,
    /// Markdown intro for fresh Platform Chat threads (agent wins).
    #[sqlx(default)]
    pub intro_markdown: Option<String>,
    /// One-line description in simplified Markdown (agent wins).
    #[sqlx(default)]
    pub short_description: Option<String>,
    /// Conversation starters (JSONB in DB, agent wins when non-empty).
    #[sqlx(default)]
    pub starters: serde_json::Value,
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
    /// Display glyph name rendered by the UI.
    pub icon: Option<String>,
    pub description: Option<String>,
    /// Markdown intro for fresh Platform Chat threads (agent wins).
    pub intro_markdown: Option<String>,
    /// One-line description in simplified Markdown (agent wins).
    pub short_description: Option<String>,
    /// Conversation starters (JSONB in DB, agent wins when non-empty).
    pub starters: serde_json::Value,
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
    /// None = leave unchanged; Some(None) = clear; Some(Some(v)) = set.
    pub intro_markdown: Option<Option<String>>,
    /// None = leave unchanged; Some(None) = clear; Some(Some(v)) = set.
    pub short_description: Option<Option<String>>,
    /// Conversation starters (JSONB); None = leave unchanged.
    pub starters: Option<serde_json::Value>,
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

#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
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
    /// Endpoint whose ingress created this session (EVE-1004). `app_id` says
    /// which bundle; this says which door. NULL for user, API, and
    /// platform-created sessions, and for app-channel sessions predating the
    /// routing tag the backfill reads.
    #[sqlx(default)]
    pub channel_id: Option<Uuid>,
    /// Agent trigger whose ingress created this session (EVE-1138). The
    /// structural successor to the `app_channel:` tag for budget attribution:
    /// migration 138 deleted the endpoint rows a webhook trigger's budgets had
    /// been keyed on, leaving the tag as the only identifier until 153.
    #[sqlx(default)]
    pub trigger_id: Option<Uuid>,
    #[sqlx(default)]
    pub harness_id: Option<HarnessId>,
    pub agent_id: Option<AgentId>,
    /// Revision of the agent's history the session started on.
    #[sqlx(default)]
    pub agent_revision: Option<i64>,
    #[sqlx(default)]
    pub virtual_user_id: Option<VirtualUserId>,
    #[sqlx(default)]
    pub playground_user_id: Option<VirtualUserId>,
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
    /// backing `crate::records::SessionSource`.
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
    /// Live events in the session (EVE-868), derived from `event_sequences` (191).
    /// Backs the Events tab badge; `#[sqlx(default)]` because most SELECTs don't project it.
    #[sqlx(default)]
    pub event_count: i64,
    /// Denormalized count of `session_tasks` rows owned by this session
    /// (EVE-868). Backs the Work tab badge.
    #[sqlx(default)]
    pub task_count: i64,
    /// Denormalized count of non-directory files in this session's workspace
    /// (EVE-868). Lives on `workspaces`, so session-detail SELECTs project it
    /// through a join and everything else leaves it at zero.
    #[sqlx(default)]
    pub workspace_file_count: i64,
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
    /// When the session was archived; `None` means active. See migration 124.
    #[sqlx(default)]
    pub archived_at: Option<DateTime<Utc>>,
}

pub use super::session_rows::{SessionListFilters, SessionListOrder};

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

pub use super::session_rows::CreateSessionRow;

#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
pub struct SessionParticipantRow {
    pub id: SessionParticipantId,
    pub org_id: i64,
    pub session_id: SessionId,
    pub kind: String,
    pub agent_id: Option<AgentId>,
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
    pub principal_id: PrincipalId,
    pub display_name: Option<String>,
    pub role: SessionParticipantRole,
    pub joined_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateSession {
    pub harness_id: Option<HarnessId>,
    pub title: Option<String>,
    pub goal: Option<String>,
    pub virtual_user_id: UpdateField<VirtualUserId>,
    pub owner_principal_id: Option<PrincipalId>,
    pub resolved_owner_user_id: UpdateField<Uuid>,
    pub locale: Option<String>,
    pub tags: Option<Vec<String>>,
    pub model_id: Option<ModelId>,
    pub status: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub tools: Option<serde_json::Value>,
}

// ============================================
// Event models (source of truth for messages)
// ============================================
// Messages are stored as events with type "message.*"; the events table is
// the sole source of truth for conversation data.

#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
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

#[derive(Debug, Clone, FromRow, serde::Serialize, everruns_server_macros::Columns)]
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

#[derive(Debug, Clone, FromRow, serde::Serialize, everruns_server_macros::Columns)]
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

#[derive(Debug, Clone, Default, serde::Serialize)]
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

#[derive(Debug, Clone, Default, serde::Serialize)]
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

#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
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
// Workspace models (see knowledge/runtime-resources/workspace.md)
// ============================================

#[derive(Debug, Clone, FromRow, serde::Serialize, everruns_server_macros::Columns)]
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
    /// `workspace.id == session.id`.
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

// ============================================
// MCP Server models
// ============================================

/// MCP Server row from database
#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
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
#[derive(Debug, Clone, FromRow, serde::Serialize, everruns_server_macros::Columns)]
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
#[derive(Debug, Clone, FromRow, serde::Serialize, everruns_server_macros::Columns)]
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

// ============================================
// File models (model-input file attachments, e.g. PDFs)
// ============================================

/// File row from database
#[derive(Debug, Clone, FromRow, serde::Serialize)]
pub struct FileRow {
    pub id: FileId,
    pub org_id: i64,
    pub filename: Option<String>,
    pub content_type: String,
    pub size_bytes: i64,
    pub data: Vec<u8>,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

/// File info without binary data (for listing)
#[derive(Debug, Clone, FromRow, serde::Serialize, everruns_server_macros::Columns)]
pub struct FileInfoRow {
    pub id: FileId,
    pub org_id: i64,
    pub filename: Option<String>,
    pub content_type: String,
    pub size_bytes: i64,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

/// Input for creating a file
#[derive(Debug, Clone)]
pub struct CreateFileRow {
    pub org_id: i64,
    pub filename: Option<String>,
    pub content_type: String,
    pub size_bytes: i64,
    pub data: Vec<u8>,
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

/// Session key/value row from database
#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
pub struct SessionKeyValueRow {
    pub id: Uuid,
    pub session_id: SessionId,
    pub key: String,
    pub value: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ============================================
// Session Key/Value Storage inputs
// ============================================

/// Input for creating/updating a session key/value
#[derive(Debug, Clone)]
pub struct UpsertSessionKeyValue {
    pub session_id: SessionId,
    pub key: String,
    pub value: String,
}

/// Lightweight key info for listing (without value)
#[derive(Debug, Clone, FromRow, serde::Serialize, everruns_server_macros::Columns)]
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
    #[sqlx(default)]
    pub virtual_user_id: Option<VirtualUserId>,
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
#[derive(Debug, Clone, FromRow, serde::Serialize, everruns_server_macros::Columns)]
pub struct SessionSecretInfoRow {
    pub name: String,
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
    pub virtual_user_id: Option<VirtualUserId>,
    pub access_token_encrypted: Vec<u8>,
    pub refresh_token_encrypted: Option<Vec<u8>>,
    pub expires_at_encrypted: Option<Vec<u8>>,
}

/// Atomic replacement for a session-scoped MCP OAuth grant.
#[derive(Debug, Clone)]
pub struct UpsertMcpOAuthSessionCredentials {
    pub virtual_user_id: Option<VirtualUserId>,
    pub session_id: SessionId,
    pub server_id: Uuid,
    pub access_token_encrypted: Vec<u8>,
    pub refresh_token_encrypted: Option<Vec<u8>>,
    pub expires_at_encrypted: Option<Vec<u8>>,
}

/// Atomic rotation of a persistent OAuth connection.
#[derive(Debug, Clone)]
pub struct UpdateOAuthConnectionTokens {
    pub connection_id: Uuid,
    pub access_token_encrypted: Vec<u8>,
    pub refresh_token_encrypted: Vec<u8>,
    pub expires_at: Option<DateTime<Utc>>,
    pub scopes: Option<String>,
}

// Virtual User Connection models

/// Agent identity connection row from database
#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
pub struct VirtualUserConnectionRow {
    pub id: Uuid,
    pub virtual_user_id: VirtualUserId,
    pub provider: String,
    pub owner_scope: String,
    pub name: Option<String>,
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

/// Input for creating an virtual user connection
#[derive(Debug, Clone)]
pub struct CreateVirtualUserConnectionRow {
    pub virtual_user_id: VirtualUserId,
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
// App models (deployable agent+harness bundles)
// ============================================

/// App row from database
#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
pub struct AppRow {
    pub id: Uuid,
    pub org_id: i64,
    pub public_id: String,
    pub name: String,
    pub description: Option<String>,
    pub harness_id: Uuid,
    pub agent_id: Option<Uuid>,
    pub virtual_user_id: Option<Uuid>,
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
    pub virtual_user_id: Option<Uuid>,
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
    pub virtual_user_id: UpdateField<Uuid>,
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
// Principal models
// ============================================

#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
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

// ============================================
// LLM generation reconciliation
// ============================================

/// Minimal projection returned by `list_unreconciled_llm_generations`.
/// Contains only the fields needed to perform a reconciliation lookup.
#[derive(Debug, Clone, sqlx::FromRow, everruns_server_macros::Columns)]
pub struct UnreconciledGeneration {
    pub id: uuid::Uuid,
    pub org_id: i64,
    pub provider_response_id: String,
}

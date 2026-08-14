// App domain types
//
// Design Decision: Dual-ID pattern (see knowledge/foundations/id-schema.md)
// - public_id: AppId (external, API-facing, client-supplied or auto-generated)
// - internal_id: Uuid (internal PK, used for FK references, never exposed in API)
//
// An App binds a Harness + Agent to a distribution channel (Slack, AG-UI, etc.)
// with a publish/unpublish lifecycle.
//
// Design Decision: Channel ingress is app-scoped because the App defines the
// agent, harness, identity, and channel-specific configuration.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use everruns_core::principal::PrincipalSummary;
use everruns_provider::typed_id::{
    AgentId, AgentIdentityId, AgentVersionId, AppChannelId, AppId, HarnessId, PrincipalId,
};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// App lifecycle status.
/// - `draft`: App is configured but not accepting requests
/// - `published`: App is live, accepting incoming requests
/// - `archived`: App is hidden from listings and cannot be modified or assigned
/// - `deleted`: App is a tombstone kept only for historical references
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[cfg_attr(feature = "openapi", schema(example = "published"))]
#[serde(rename_all = "lowercase")]
pub enum AppStatus {
    Draft,
    Published,
    Archived,
    Deleted,
}

/// How an App resolves the Agent version it runs.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[cfg_attr(feature = "openapi", schema(example = "pinned"))]
#[serde(rename_all = "lowercase")]
pub enum AgentVersionPolicy {
    /// Resolve the agent's default_version_id at session creation/invocation time.
    #[default]
    Default,
    /// Resolve the newest agent_versions row for the app's agent.
    Latest,
    /// Use the app's pinned agent_version_id.
    Pinned,
}

impl std::fmt::Display for AgentVersionPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentVersionPolicy::Default => write!(f, "default"),
            AgentVersionPolicy::Latest => write!(f, "latest"),
            AgentVersionPolicy::Pinned => write!(f, "pinned"),
        }
    }
}

impl From<&str> for AgentVersionPolicy {
    fn from(s: &str) -> Self {
        match s {
            "latest" => AgentVersionPolicy::Latest,
            "pinned" => AgentVersionPolicy::Pinned,
            _ => AgentVersionPolicy::Default,
        }
    }
}

impl std::fmt::Display for AppStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppStatus::Draft => write!(f, "draft"),
            AppStatus::Published => write!(f, "published"),
            AppStatus::Archived => write!(f, "archived"),
            AppStatus::Deleted => write!(f, "deleted"),
        }
    }
}

impl From<&str> for AppStatus {
    fn from(s: &str) -> Self {
        match s {
            "published" => AppStatus::Published,
            "archived" => AppStatus::Archived,
            "deleted" => AppStatus::Deleted,
            _ => AppStatus::Draft,
        }
    }
}

/// Supported channel types for app distribution.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[cfg_attr(feature = "openapi", schema(example = "webhook"))]
#[serde(rename_all = "lowercase")]
pub enum ChannelType {
    Slack,
    #[serde(rename = "ag_ui")]
    AgUi,
    Schedule,
    Webhook,
    /// Agent2Agent (A2A) protocol channel — JSON-RPC + API key.
    A2a,
    /// Free Communication Protocol channel — text-first HTTP ingress with an
    /// optional handshake. See `knowledge/integrations/fcp-channel.md` and the upstream FCP
    /// specification.
    Fcp,
    /// App-scoped, execution-only API key over native session routes.
    /// See `knowledge/integrations/app-api-keys.md`.
    #[serde(rename = "api_endpoint")]
    ApiEndpoint,
    /// Public Chat channel — an isolated, public-facing chat web app bound to a
    /// single App's agent. Anonymous by default, with optional Google sign-in
    /// and Cloudflare Turnstile bot mitigation. Reuses AG-UI streaming and the
    /// shared App endpoint auth verifier. See `knowledge/integrations/public-chat.md`.
    #[serde(rename = "public_chat")]
    PublicChat,
}

impl std::fmt::Display for ChannelType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChannelType::Slack => write!(f, "slack"),
            ChannelType::AgUi => write!(f, "ag_ui"),
            ChannelType::Schedule => write!(f, "schedule"),
            ChannelType::Webhook => write!(f, "webhook"),
            ChannelType::A2a => write!(f, "a2a"),
            ChannelType::Fcp => write!(f, "fcp"),
            ChannelType::ApiEndpoint => write!(f, "api_endpoint"),
            ChannelType::PublicChat => write!(f, "public_chat"),
        }
    }
}

impl ChannelType {
    pub fn from_str_opt(s: &str) -> Option<Self> {
        match s {
            "slack" => Some(ChannelType::Slack),
            "ag_ui" => Some(ChannelType::AgUi),
            "schedule" => Some(ChannelType::Schedule),
            "webhook" => Some(ChannelType::Webhook),
            "a2a" => Some(ChannelType::A2a),
            "fcp" => Some(ChannelType::Fcp),
            "api_endpoint" => Some(ChannelType::ApiEndpoint),
            "public_chat" => Some(ChannelType::PublicChat),
            _ => None,
        }
    }
}

/// App configuration for deploying agents to channels.
/// An app binds a harness and optional agent to distribution channels with a
/// publish lifecycle.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct App {
    /// External identifier (app_<32-hex>). Shown as "id" in API.
    #[serde(rename = "id")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "app_01933b5a000070008000000000000001"))]
    pub public_id: AppId,
    /// Internal UUID primary key. Used for FK references. Never exposed in API.
    #[serde(skip, default = "Uuid::nil")]
    pub internal_id: Uuid,
    /// Organization ID. Internal only, not exposed in API.
    #[serde(skip, default)]
    pub org_id: i64,
    /// Display name of the app.
    pub name: String,
    /// Human-readable description of what the app does.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// ID of the harness to use (format: harness_{32-hex}).
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "harness_01933b5a00007000800000000000001"))]
    pub harness_id: HarnessId,
    /// Optional ID of the agent to use (format: agent_{32-hex}).
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, example = "agent_01933b5a00007000800000000000001"))]
    pub agent_id: Option<AgentId>,
    /// Version resolution policy for the optional agent.
    #[serde(default)]
    pub agent_version_policy: AgentVersionPolicy,
    /// Pinned agent version. Required when policy is `pinned`.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, example = "agentver_01933b5a00007000800000000000001"))]
    pub agent_version_id: Option<AgentVersionId>,
    /// Optional virtual identity that represents the app in unattended/channel execution.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, example = "identity_01933b5a00007000800000000000001"))]
    pub agent_identity_id: Option<AgentIdentityId>,
    /// Owning principal for this app.
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "principal_01933b5a000070008000000000000001"))]
    pub owner_principal_id: PrincipalId,
    /// Denormalized effective human owner of the owning principal lineage.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_owner_user_id: Option<Uuid>,
    /// Owning principal summary.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<PrincipalSummary>,
    /// Effective human owner summary.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective_owner: Option<PrincipalSummary>,
    /// Distribution channels attached to this app.
    #[serde(default)]
    pub channels: Vec<AppChannel>,
    /// Current lifecycle status.
    pub status: AppStatus,
    /// Timestamp when the app was last published.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub published_at: Option<DateTime<Utc>>,
    /// Timestamp when the app was created.
    pub created_at: DateTime<Utc>,
    /// Timestamp when the app was last updated.
    pub updated_at: DateTime<Utc>,
    /// Timestamp when the app was archived.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<DateTime<Utc>>,
    /// Timestamp when the app was deleted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<DateTime<Utc>>,
}

/// A single distribution channel attached to an App.
/// Each channel has its own type, config, and lifecycle status.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct AppChannel {
    /// External identifier (appchan_<32-hex>). Shown as "id" in API.
    #[serde(rename = "id")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "appchan_01933b5a000070008000000000000001"))]
    pub public_id: AppChannelId,
    /// Internal UUID primary key. Never exposed in API.
    #[serde(skip, default = "Uuid::nil")]
    pub internal_id: Uuid,
    /// Channel type (e.g. slack).
    pub channel_type: ChannelType,
    /// Channel-specific configuration (validated per channel type).
    #[serde(default)]
    pub channel_config: serde_json::Value,
    /// Whether this channel is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Timestamp when this channel was created.
    pub created_at: DateTime<Utc>,
    /// Timestamp when this channel was last updated.
    pub updated_at: DateTime<Utc>,
}

fn default_true() -> bool {
    true
}

impl AppChannel {
    /// Parse channel_config as SlackChannelConfig. Returns None if not a Slack channel
    /// or if the config is invalid.
    pub fn slack_config(&self) -> Option<SlackChannelConfig> {
        if self.channel_type != ChannelType::Slack {
            return None;
        }
        serde_json::from_value(self.channel_config.clone()).ok()
    }

    /// Parse channel_config as AgUiChannelConfig. Returns None if not an AG-UI
    /// channel or if the config is invalid.
    pub fn ag_ui_config(&self) -> Option<AgUiChannelConfig> {
        if self.channel_type != ChannelType::AgUi {
            return None;
        }
        serde_json::from_value(self.channel_config.clone()).ok()
    }

    /// Parse channel_config as ScheduleChannelConfig. Returns None if not a
    /// schedule channel or if the config is invalid.
    pub fn schedule_config(&self) -> Option<ScheduleChannelConfig> {
        if self.channel_type != ChannelType::Schedule {
            return None;
        }
        serde_json::from_value(self.channel_config.clone()).ok()
    }

    /// Parse channel_config as WebhookChannelConfig. Returns None if not a
    /// webhook channel or if the config is invalid.
    pub fn webhook_config(&self) -> Option<WebhookChannelConfig> {
        if self.channel_type != ChannelType::Webhook {
            return None;
        }
        serde_json::from_value(self.channel_config.clone()).ok()
    }

    /// Parse channel_config as FcpChannelConfig. Returns None if not an FCP
    /// channel or if the config is invalid.
    pub fn fcp_config(&self) -> Option<FcpChannelConfig> {
        if self.channel_type != ChannelType::Fcp {
            return None;
        }
        serde_json::from_value(self.channel_config.clone()).ok()
    }

    /// Parse channel_config as A2aChannelConfig. Returns None if not an A2A
    /// channel or if the config is invalid.
    pub fn a2a_config(&self) -> Option<A2aChannelConfig> {
        if self.channel_type != ChannelType::A2a {
            return None;
        }
        serde_json::from_value(self.channel_config.clone()).ok()
    }

    /// Parse channel_config as ApiEndpointChannelConfig. Returns None if not an
    /// api_endpoint channel or if the config is invalid.
    pub fn api_endpoint_config(&self) -> Option<ApiEndpointChannelConfig> {
        if self.channel_type != ChannelType::ApiEndpoint {
            return None;
        }
        serde_json::from_value(self.channel_config.clone()).ok()
    }

    /// Parse channel_config as PublicChatChannelConfig. Returns None if not a
    /// Public Chat channel or if the config is invalid.
    pub fn public_chat_config(&self) -> Option<PublicChatChannelConfig> {
        if self.channel_type != ChannelType::PublicChat {
            return None;
        }
        serde_json::from_value(self.channel_config.clone()).ok()
    }
}

impl App {
    /// Find the first Slack channel on this app.
    pub fn slack_channel(&self) -> Option<&AppChannel> {
        self.channels
            .iter()
            .find(|ch| ch.channel_type == ChannelType::Slack && ch.enabled)
    }

    /// Find the first enabled AG-UI channel on this app.
    pub fn ag_ui_channel(&self) -> Option<&AppChannel> {
        self.channels
            .iter()
            .find(|ch| ch.channel_type == ChannelType::AgUi && ch.enabled)
    }

    /// Find the first enabled schedule channel on this app.
    pub fn schedule_channel(&self) -> Option<&AppChannel> {
        self.channels
            .iter()
            .find(|ch| ch.channel_type == ChannelType::Schedule && ch.enabled)
    }

    /// Find the first enabled webhook channel on this app.
    pub fn webhook_channel(&self) -> Option<&AppChannel> {
        self.channels
            .iter()
            .find(|ch| ch.channel_type == ChannelType::Webhook && ch.enabled)
    }

    /// Find the first enabled FCP channel on this app.
    pub fn fcp_channel(&self) -> Option<&AppChannel> {
        self.channels
            .iter()
            .find(|ch| ch.channel_type == ChannelType::Fcp && ch.enabled)
    }

    /// Find the first enabled A2A channel on this app.
    pub fn a2a_channel(&self) -> Option<&AppChannel> {
        self.channels
            .iter()
            .find(|ch| ch.channel_type == ChannelType::A2a && ch.enabled)
    }

    /// Find the first enabled api_endpoint channel on this app.
    pub fn api_endpoint_channel(&self) -> Option<&AppChannel> {
        self.channels
            .iter()
            .find(|ch| ch.channel_type == ChannelType::ApiEndpoint && ch.enabled)
    }

    /// Find the first enabled Public Chat channel on this app.
    pub fn public_chat_channel(&self) -> Option<&AppChannel> {
        self.channels
            .iter()
            .find(|ch| ch.channel_type == ChannelType::PublicChat && ch.enabled)
    }

    /// Find a channel by its public ID.
    pub fn channel_by_id(&self, id: &AppChannelId) -> Option<&AppChannel> {
        self.channels.iter().find(|ch| ch.public_id == *id)
    }
}

/// Session strategy for incoming messages (how messages map to sessions).
///
/// This is the Slack-specific config type that serializes in `SlackChannelConfig`.
/// Converts to/from the generic `SessionRoutingStrategy` in `crate::channel`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum SessionStrategy {
    /// Each Slack thread gets its own session (default).
    #[default]
    PerThread,
    /// One session per channel.
    PerChannel,
    /// One session per user.
    PerUser,
}

impl From<SessionStrategy> for everruns_core::channel::SessionRoutingStrategy {
    fn from(s: SessionStrategy) -> Self {
        match s {
            SessionStrategy::PerThread => Self::PerThread,
            SessionStrategy::PerChannel => Self::PerChannel,
            SessionStrategy::PerUser => Self::PerUser,
        }
    }
}

impl From<everruns_core::channel::SessionRoutingStrategy> for SessionStrategy {
    fn from(s: everruns_core::channel::SessionRoutingStrategy) -> Self {
        match s {
            everruns_core::channel::SessionRoutingStrategy::PerThread => Self::PerThread,
            everruns_core::channel::SessionRoutingStrategy::PerChannel => Self::PerChannel,
            everruns_core::channel::SessionRoutingStrategy::PerUser => Self::PerUser,
        }
    }
}

/// How replies are delivered back to Slack.
///
/// This is the Slack-specific config type that serializes in `SlackChannelConfig`.
/// Converts to/from the generic `ChannelReplyMode` in `crate::channel`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum SlackReplyMode {
    /// Forward completed assistant messages directly to Slack.
    #[default]
    AllMessages,
    /// Only send deterministic updates emitted via `report_progress`.
    ReportProgressOnly,
}

impl From<SlackReplyMode> for everruns_core::channel::ChannelReplyMode {
    fn from(m: SlackReplyMode) -> Self {
        match m {
            SlackReplyMode::AllMessages => Self::AllMessages,
            SlackReplyMode::ReportProgressOnly => Self::ReportProgressOnly,
        }
    }
}

impl From<everruns_core::channel::ChannelReplyMode> for SlackReplyMode {
    fn from(m: everruns_core::channel::ChannelReplyMode) -> Self {
        match m {
            everruns_core::channel::ChannelReplyMode::AllMessages => Self::AllMessages,
            everruns_core::channel::ChannelReplyMode::ReportProgressOnly => {
                Self::ReportProgressOnly
            }
        }
    }
}

/// Typed Slack channel configuration.
/// Parsed from the `channel_config` JSON field on App.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct SlackChannelConfig {
    /// Slack signing secret for verifying webhook requests.
    pub signing_secret: String,
    /// Slack Bot OAuth token for sending responses.
    pub bot_token: String,
    /// Slack channel ID to listen on (e.g., "C0123456789").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    /// Slack team/workspace ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team_id: Option<String>,
    /// How incoming messages map to sessions.
    #[serde(default)]
    pub session_strategy: SessionStrategy,
    /// How replies are delivered back to Slack.
    #[serde(default)]
    pub reply_mode: SlackReplyMode,
    /// Set when Slack successfully verifies the webhook URL (url_verification challenge).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub webhook_verified_at: Option<DateTime<Utc>>,
    /// Set when the first real message is received from Slack.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_message_received_at: Option<DateTime<Utc>>,
}

/// Default session expiration for public channel threads (6 hours).
pub const DEFAULT_SESSION_EXPIRATION_SECONDS: u32 = 6 * 60 * 60;

/// Default public AG-UI text shown while a tool call is running.
pub const DEFAULT_AG_UI_GENERIC_TOOL_TEXT: &str = "Working...";

/// Public AG-UI tool activity visibility.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum AgUiToolVisibility {
    /// Do not expose tool activity in public AG-UI streams.
    None,
    /// Expose only editable generic text, without tool names, args, or output.
    #[default]
    Generic,
    /// Expose backend-authored narration, without raw tool names, args, or output.
    Narrated,
}

/// App-published endpoint authentication mode.
///
/// Stored inline on `app_channels.channel_config.auth` so users can protect a
/// single App/channel without first creating org-level identity-provider state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum AppEndpointAuthMode {
    Anonymous,
    SharedSecret,
    ApiKey,
    GoogleOidc,
    Oidc,
    OAuth2Introspection,
    HttpBasic,
    Mtls,
}

/// OIDC/OAuth/basic/mTLS provider details for one App endpoint.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AppEndpointAuthProviderConfig {
    GoogleOidc {
        client_id: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        allowed_domains: Vec<String>,
    },
    Oidc {
        issuer: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        jwks_url: Option<String>,
    },
    OAuth2Introspection {
        introspection_url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        client_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        client_secret: Option<String>,
    },
    HttpBasic {
        username: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        password: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        password_hash: Option<String>,
    },
    Mtls {
        header_name: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        allowed_values: Vec<String>,
        /// Header the trusted reverse proxy uses to prove its identity.
        /// Required. Configs without this field fail closed at verification time.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        proxy_secret_header: Option<String>,
        /// Shared secret the trusted proxy includes in `proxy_secret_header`.
        /// Write-only: redacted in GET responses. See TM-AUTH-021.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        proxy_secret: Option<String>,
    },
}

/// Claim and credential requirements common to App endpoint auth providers.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct AppEndpointAuthRequirements {
    /// JWT `aud` values to require on inbound tokens. Empty list disables audience checking.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub audiences: Vec<String>,
    /// OAuth scope strings to require (space-delimited per scope entry). Empty list disables scope checking.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scopes: Vec<String>,
    /// Arbitrary claim equality predicates. Empty map disables claim filtering.
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub claims: serde_json::Map<String, serde_json::Value>,
    /// Allowlist of `sub` claim values. Empty list disables subject filtering.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subjects: Vec<String>,
    /// Allowlist of group memberships (from `groups` claim). Empty list disables group filtering.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<String>,
    /// Allowlist of email/identifier domains. Empty list disables domain filtering.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub domains: Vec<String>,
}

/// Inline auth config for one App endpoint/channel.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[cfg_attr(
    feature = "openapi",
    schema(example = json!({"mode": "api_key", "requirements": {"audiences": ["everruns-api"], "scopes": ["app:invoke"]}}))
)]
pub struct AppEndpointAuthConfig {
    pub mode: AppEndpointAuthMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<AppEndpointAuthProviderConfig>,
    #[serde(default)]
    pub requirements: AppEndpointAuthRequirements,
}

/// Typed AG-UI channel configuration.
///
/// Parsed from the `channel_config` JSON field on App.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct AgUiChannelConfig {
    /// Whether anonymous access is allowed for this endpoint.
    /// Enabled by default for the initial AG-UI rollout.
    #[serde(default = "default_true")]
    pub anonymous: bool,
    /// Optional shared bearer token for the public AG-UI endpoint.
    /// When set, requests must include the token in a supported header.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    /// How long (in seconds) a thread can be resumed after its session was
    /// created. Once this elapses, the same `thread_id` cannot reuse the
    /// existing session and must start a new one. `0` disables expiration.
    /// Defaults to 6 hours.
    #[serde(default = "default_session_expiration_seconds")]
    pub session_expiration_seconds: u32,
    /// Optional per-IP rate limit applied to this app's AG-UI endpoint, in
    /// requests per minute. `None` or `Some(0)` disables the per-app limit
    /// (the global API limit still applies). Set a positive value to enforce
    /// a stricter cap on anonymous traffic for this app.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_limit_per_minute: Option<u32>,
    /// Public tool activity visibility for anonymous AG-UI streams.
    #[serde(default)]
    pub tool_visibility: AgUiToolVisibility,
    /// Generic public text shown when `tool_visibility` is `generic`.
    #[serde(
        default = "default_ag_ui_generic_tool_text",
        skip_serializing_if = "is_default_ag_ui_generic_tool_text"
    )]
    pub generic_tool_text: String,
    /// Whether provider-curated reasoning summaries may be emitted on public
    /// AG-UI streams. Defaults off because published apps can be anonymous and
    /// summaries may derive from private prompts, tools, or retrieved data.
    #[serde(default, skip_serializing_if = "is_false")]
    pub reasoning_summary_visible: bool,
    /// Optional inline auth config for this public endpoint. When omitted,
    /// legacy `anonymous` + `token` behavior applies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<AppEndpointAuthConfig>,
}

fn default_session_expiration_seconds() -> u32 {
    DEFAULT_SESSION_EXPIRATION_SECONDS
}

fn default_ag_ui_generic_tool_text() -> String {
    DEFAULT_AG_UI_GENERIC_TOOL_TEXT.to_string()
}

fn is_default_ag_ui_generic_tool_text(value: &str) -> bool {
    value == DEFAULT_AG_UI_GENERIC_TOOL_TEXT
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// Default FCP handshake body. Returned by `GET` when the channel config does
/// not override `handshake`. Kept generic so an unconfigured FCP endpoint
/// still satisfies the FCP `SHOULD` for handshake responses.
pub const DEFAULT_FCP_HANDSHAKE: &str = "FCP endpoint.\n\nPOST plain text or `application/json` (`{\"message\": \"...\"}`) to\nthis URL to talk to the agent. Replies are returned as `text/markdown`.\n\nSession state, when supported, is carried by the `fcp_session` cookie.";

/// Default response timeout for a blocking FCP POST, in seconds.
pub const DEFAULT_FCP_RESPONSE_TIMEOUT_SECONDS: u32 = 120;

/// Typed FCP channel configuration.
///
/// FCP is intentionally schema-free at the wire layer (see
/// `knowledge/integrations/fcp-channel.md`). The fields here only control server-side
/// behavior — handshake content, a single shared bearer token, session
/// reuse, rate limiting — and never constrain the body the actor sends in.
///
/// FCP deliberately exposes a **smaller** auth surface than AG-UI/A2A: it
/// supports anonymous access and a single shared bearer token, and nothing
/// else. Inline OIDC/HTTP-Basic/mTLS verifier modes are intentionally
/// **not** wired here so the FCP ingress path does not share authentication
/// machinery with other channels or with the main API's user auth stack.
/// Operators that need IdP-backed auth in front of an FCP endpoint should
/// terminate that at the edge (reverse proxy, IAP, mTLS) rather than asking
/// the FCP handler to grow another auth mode.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct FcpChannelConfig {
    /// Whether anonymous access is allowed for this endpoint. When `false`
    /// a non-empty `token` must authenticate every `POST`.
    #[serde(default = "default_true")]
    pub anonymous: bool,
    /// Optional shared bearer token. When set, callers must send
    /// `Authorization: Bearer <token>` (or the `X-Everruns-FCP-Token`
    /// header). Validated by constant-time comparison inside the FCP
    /// handler — never via the shared App endpoint auth verifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    /// Markdown body returned for `GET` requests (the FCP handshake). When
    /// omitted, a generic handshake derived from the app's name and
    /// description is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handshake: Option<String>,
    /// How long (in seconds) the FCP session cookie keeps a session
    /// resumable. `0` disables expiration. Defaults to 6 hours so a
    /// long-running conversation expires on a sensible cadence.
    #[serde(default = "default_session_expiration_seconds")]
    pub session_expiration_seconds: u32,
    /// Optional per-IP rate limit (requests per minute) for the FCP endpoint.
    /// `None` or `Some(0)` disables the per-app limit; the global API limit
    /// still applies. Counted in an FCP-specific limiter namespace so it
    /// cannot be shared or exhausted by other channels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_limit_per_minute: Option<u32>,
    /// Maximum number of seconds the FCP endpoint waits for the agent to
    /// produce a reply before returning a `504`. Defaults to 120 s.
    #[serde(default = "default_fcp_response_timeout_seconds")]
    pub response_timeout_seconds: u32,
}

fn default_fcp_response_timeout_seconds() -> u32 {
    DEFAULT_FCP_RESPONSE_TIMEOUT_SECONDS
}

/// How app-triggered invocations route into sessions.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[cfg_attr(feature = "openapi", schema(example = "shared_session"))]
#[serde(rename_all = "snake_case")]
pub enum InvocationSessionMode {
    /// Reuse a single durable session for every invocation of the channel.
    #[default]
    SharedSession,
    /// Create a fresh session for every invocation.
    SessionPerInvocation,
}

/// Typed schedule channel configuration.
///
/// `message` is also the template body. `{{path.to.value}}` placeholders are
/// expanded at invocation time.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ScheduleChannelConfig {
    /// Cron expression that drives the durable schedule.
    pub cron_expression: String,
    /// IANA timezone identifier for cron evaluation.
    #[serde(default = "default_timezone")]
    pub timezone: String,
    /// Whether invocations reuse a stable session or create a new one.
    #[serde(default)]
    pub session_mode: InvocationSessionMode,
    /// Message content or template sent when the schedule fires.
    pub message: String,
}

/// Typed webhook channel configuration.
///
/// `message` is also the template body. `{{path.to.value}}` placeholders are
/// expanded against the incoming webhook payload and metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct WebhookChannelConfig {
    /// Shared secret required from the incoming webhook request.
    pub token: String,
    /// Whether invocations reuse a stable session or create a new one.
    #[serde(default)]
    pub session_mode: InvocationSessionMode,
    /// Message content or template sent when the webhook arrives.
    pub message: String,
    /// Optional per-IP rate limit applied to this app's webhook endpoint, in
    /// requests per minute. `None` or `Some(0)` disables the per-channel limit
    /// (the global API limit still applies). Mirrors
    /// `A2aChannelConfig::rate_limit_per_minute` (EVE-627).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_limit_per_minute: Option<u32>,
}

fn default_timezone() -> String {
    "UTC".to_string()
}

/// Typed A2A (Agent2Agent) channel configuration.
///
/// The plaintext API key is **never** stored. Only the SHA-256 hex hash and a
/// non-secret display prefix are persisted. The plaintext is returned exactly
/// once at create / regenerate time.
///
/// `message` is the template body. `{{path.to.value}}` placeholders expand
/// against the incoming A2A request payload and metadata (see
/// `knowledge/integrations/a2a-channel.md`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct A2aChannelConfig {
    /// SHA-256 hex digest of the API key.
    pub api_key_hash: String,
    /// Public, non-secret display prefix (e.g. `evra2a_abc1...`).
    pub api_key_prefix: String,
    /// Whether invocations reuse a stable session or create a new one.
    #[serde(default)]
    pub session_mode: InvocationSessionMode,
    /// Message template rendered into the session per invocation.
    pub message: String,
    /// Optional human-readable agent name surfaced in the Agent Card.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_card_name: Option<String>,
    /// Optional description surfaced in the Agent Card.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_card_description: Option<String>,
    /// Optional per-IP rate limit applied to this app's A2A endpoint, in
    /// requests per minute. `None` or `Some(0)` disables the per-channel
    /// limit (the global API limit still applies). Set a positive value to
    /// enforce a stricter cap on unattended agent-to-agent traffic for this
    /// app. Mirrors `AgUiChannelConfig::rate_limit_per_minute`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_limit_per_minute: Option<u32>,
    /// Optional inline auth config for this A2A endpoint. When omitted,
    /// legacy per-channel API-key behavior applies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<AppEndpointAuthConfig>,
    /// Optional shared HMAC signing secret. When set, requests must include
    /// `X-Everruns-A2A-Timestamp` + `X-Everruns-A2A-Signature` headers and
    /// the server verifies an HMAC-SHA256 signature over the exact
    /// basestring `v0:{timestamp}:{body}` (Slack-style, no whitespace
    /// between segments) plus a 5-minute timestamp window plus a
    /// signature-keyed dedup so a captured request cannot be replayed
    /// while the API key is still valid (TM-A2A-010). When `None`, the
    /// channel keeps the existing API-key-only behavior. Layered **on top
    /// of** `auth` — independent concerns: `auth` selects who can call,
    /// `signing_secret` adds replay protection on top.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signing_secret: Option<String>,
}

/// Typed api_endpoint channel configuration.
///
/// Exposes an app-scoped, execution-only API key over native session routes
/// mounted under the app (`/v1/apps/{app_id}/api/{channel_id}/...`). The key is
/// structurally execution-only: it reaches only these app-mounted routes and
/// has no path to any management API. The plaintext key is **never** stored —
/// only the SHA-256 hex hash and a non-secret display prefix are persisted, and
/// the plaintext is returned exactly once at create / regenerate time. See
/// `knowledge/integrations/app-api-keys.md`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ApiEndpointChannelConfig {
    /// SHA-256 hex digest of the API key.
    pub api_key_hash: String,
    /// Public, non-secret display prefix (e.g. `evr_app_abc1...`).
    pub api_key_prefix: String,
    /// Whether invocations reuse a stable session or create a new one.
    #[serde(default)]
    pub session_mode: InvocationSessionMode,
    /// Optional per-IP rate limit applied to this app's api_endpoint, in
    /// requests per minute. `None` or `Some(0)` disables the per-channel limit
    /// (the global API limit still applies). Mirrors
    /// `A2aChannelConfig::rate_limit_per_minute`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_limit_per_minute: Option<u32>,
    /// Optional inline auth config for this endpoint. When omitted, the
    /// generated per-channel API-key bearer scheme applies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<AppEndpointAuthConfig>,
}

/// Branding shown on a Public Chat surface. All fields optional; the public app
/// falls back to the App's name and the default design-system theme when unset.
/// Branding is non-secret and is returned as-is in API responses.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct PublicChatBranding {
    /// Display name shown in the chat header. Falls back to the App name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Absolute URL of a logo image shown in the chat header.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logo_url: Option<String>,
    /// Primary accent color as a CSS hex string (e.g. `#0A1636`). Applied by
    /// overriding the design-system primary variable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_color: Option<String>,
    /// Welcome message shown before the visitor sends their first message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub welcome_message: Option<String>,
}

/// Bot-mitigation challenge provider for anonymous Public Chat access.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum CaptchaProvider {
    /// Cloudflare Turnstile. The only provider supported in the first release.
    #[default]
    Turnstile,
}

/// CAPTCHA / bot-mitigation configuration for a Public Chat channel.
///
/// When present and enabled, anonymous visitors must pass a challenge before a
/// session is created or any turn runs; signed-in visitors bypass it. The
/// `secret_key` is write-only: it is stored to call the provider's verify
/// endpoint server-side and is redacted in API responses (only
/// `secret_key_configured: bool` is surfaced). The `site_key` is public and
/// returned so the web app can render the widget.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct PublicChatCaptchaConfig {
    /// Challenge provider. Defaults to Cloudflare Turnstile.
    #[serde(default)]
    pub provider: CaptchaProvider,
    /// Whether the challenge is enforced. Defaults to true so adding a captcha
    /// config turns it on; set to false to keep keys configured but inactive.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Public site key rendered by the client widget.
    pub site_key: String,
    /// Secret key used for server-side verification. Write-only; redacted on read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_key: Option<String>,
}

/// Typed Public Chat channel configuration.
///
/// Parsed from the `channel_config` JSON field on App. Public Chat reuses
/// AG-UI's streaming semantics and the shared App endpoint auth verifier, and
/// adds branding and bot-mitigation tailored to a public, link-shareable chat
/// website. See `knowledge/integrations/public-chat.md`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct PublicChatChannelConfig {
    /// Whether anonymous access is allowed. Anonymous-by-default mirrors AG-UI.
    /// When `false`, visitors must authenticate (e.g. via `auth` Google OIDC).
    #[serde(default = "default_true")]
    pub anonymous: bool,
    /// Optional shared bearer token for simple gated access. When set, requests
    /// must include the token in a supported header. Redacted on read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    /// How long (in seconds) a visitor session can be resumed before a fresh
    /// one must be started. `0` disables expiration. Defaults to 6 hours.
    #[serde(default = "default_session_expiration_seconds")]
    pub session_expiration_seconds: u32,
    /// Optional per-IP, per-app rate limit in requests per minute. `None` or
    /// `Some(0)` disables the per-app cap (the global API limit still applies).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_limit_per_minute: Option<u32>,
    /// Public tool activity visibility. Same rules as AG-UI: raw tool names,
    /// args, results, and internal IDs are never exposed on public streams.
    #[serde(default)]
    pub tool_visibility: AgUiToolVisibility,
    /// Generic public text shown when `tool_visibility` is `generic`/`narrated`.
    #[serde(
        default = "default_ag_ui_generic_tool_text",
        skip_serializing_if = "is_default_ag_ui_generic_tool_text"
    )]
    pub generic_tool_text: String,
    /// Optional inline auth config for this public endpoint (e.g. Google OIDC).
    /// When omitted, anonymous + optional `token` behavior applies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<AppEndpointAuthConfig>,
    /// Branding shown on the public chat surface.
    #[serde(default, skip_serializing_if = "PublicChatBranding::is_empty")]
    pub branding: PublicChatBranding,
    /// Optional bot-mitigation / CAPTCHA configuration (Cloudflare Turnstile).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub captcha: Option<PublicChatCaptchaConfig>,
}

impl PublicChatBranding {
    /// True when no branding fields are set, so the whole object can be omitted
    /// from serialized output.
    pub fn is_empty(&self) -> bool {
        self.display_name.is_none()
            && self.logo_url.is_none()
            && self.primary_color.is_none()
            && self.welcome_message.is_none()
    }
}

impl PublicChatChannelConfig {
    /// Project the streaming-relevant fields onto an `AgUiChannelConfig` so the
    /// shared AG-UI ingress/streaming core can serve Public Chat without a
    /// parallel implementation. Branding and captcha are Public-Chat-only and
    /// are handled by the Public Chat handler, not the shared core.
    pub fn ag_ui_stream_config(&self) -> AgUiChannelConfig {
        AgUiChannelConfig {
            anonymous: self.anonymous,
            token: self.token.clone(),
            session_expiration_seconds: self.session_expiration_seconds,
            rate_limit_per_minute: self.rate_limit_per_minute,
            tool_visibility: self.tool_visibility,
            generic_tool_text: self.generic_tool_text.clone(),
            reasoning_summary_visible: false,
            auth: self.auth.clone(),
        }
    }

    /// Whether the Turnstile/CAPTCHA challenge should be enforced for an
    /// anonymous visitor. Returns false when no captcha is configured or it is
    /// explicitly disabled. Signed-in visitors (validated via `auth`) bypass the
    /// challenge and are handled by the caller.
    pub fn captcha_enforced(&self) -> bool {
        self.captcha.as_ref().is_some_and(|c| c.enabled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_app_status_display() {
        assert_eq!(AppStatus::Draft.to_string(), "draft");
        assert_eq!(AppStatus::Published.to_string(), "published");
        assert_eq!(AppStatus::Archived.to_string(), "archived");
        assert_eq!(AppStatus::Deleted.to_string(), "deleted");
    }

    #[test]
    fn test_app_status_from_str() {
        assert_eq!(AppStatus::from("draft"), AppStatus::Draft);
        assert_eq!(AppStatus::from("published"), AppStatus::Published);
        assert_eq!(AppStatus::from("archived"), AppStatus::Archived);
        assert_eq!(AppStatus::from("deleted"), AppStatus::Deleted);
        assert_eq!(AppStatus::from("unknown"), AppStatus::Draft);
        assert_eq!(AppStatus::from(""), AppStatus::Draft);
    }

    #[test]
    fn test_app_status_serde_roundtrip() {
        let json = serde_json::to_string(&AppStatus::Published).unwrap();
        assert_eq!(json, r#""published""#);
        let parsed: AppStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, AppStatus::Published);
    }

    #[test]
    fn test_channel_type_display() {
        assert_eq!(ChannelType::Slack.to_string(), "slack");
        assert_eq!(ChannelType::AgUi.to_string(), "ag_ui");
        assert_eq!(ChannelType::Schedule.to_string(), "schedule");
        assert_eq!(ChannelType::Webhook.to_string(), "webhook");
        assert_eq!(ChannelType::A2a.to_string(), "a2a");
        assert_eq!(ChannelType::Fcp.to_string(), "fcp");
        assert_eq!(ChannelType::PublicChat.to_string(), "public_chat");
    }

    #[test]
    fn test_channel_type_from_str_opt() {
        assert_eq!(ChannelType::from_str_opt("slack"), Some(ChannelType::Slack));
        assert_eq!(ChannelType::from_str_opt("ag_ui"), Some(ChannelType::AgUi));
        assert_eq!(
            ChannelType::from_str_opt("schedule"),
            Some(ChannelType::Schedule)
        );
        assert_eq!(
            ChannelType::from_str_opt("webhook"),
            Some(ChannelType::Webhook)
        );
        assert_eq!(ChannelType::from_str_opt("a2a"), Some(ChannelType::A2a));
        assert_eq!(ChannelType::from_str_opt("fcp"), Some(ChannelType::Fcp));
        assert_eq!(
            ChannelType::from_str_opt("public_chat"),
            Some(ChannelType::PublicChat)
        );
        assert_eq!(ChannelType::from_str_opt("unknown"), None);
        assert_eq!(ChannelType::from_str_opt(""), None);
    }

    #[test]
    fn test_channel_type_serde_roundtrip() {
        let json = serde_json::to_string(&ChannelType::Slack).unwrap();
        assert_eq!(json, r#""slack""#);
        let parsed: ChannelType = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, ChannelType::Slack);

        let json = serde_json::to_string(&ChannelType::AgUi).unwrap();
        assert_eq!(json, r#""ag_ui""#);
        let parsed: ChannelType = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, ChannelType::AgUi);

        let json = serde_json::to_string(&ChannelType::Schedule).unwrap();
        assert_eq!(json, r#""schedule""#);
        let parsed: ChannelType = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, ChannelType::Schedule);

        let json = serde_json::to_string(&ChannelType::Webhook).unwrap();
        assert_eq!(json, r#""webhook""#);
        let parsed: ChannelType = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, ChannelType::Webhook);
    }

    #[test]
    fn test_session_strategy_default() {
        assert_eq!(SessionStrategy::default(), SessionStrategy::PerThread);
    }

    #[test]
    fn test_session_strategy_serde() {
        let json = serde_json::to_string(&SessionStrategy::PerChannel).unwrap();
        assert_eq!(json, r#""per_channel""#);
        let parsed: SessionStrategy = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, SessionStrategy::PerChannel);

        let json = serde_json::to_string(&SessionStrategy::PerUser).unwrap();
        assert_eq!(json, r#""per_user""#);
    }

    #[test]
    fn test_slack_channel_config_full() {
        let json = r#"{
            "signing_secret": "sec123",
            "bot_token": "xoxb-tok",
            "channel_id": "C123",
            "team_id": "T123",
            "session_strategy": "per_channel",
            "reply_mode": "report_progress_only"
        }"#;
        let config: SlackChannelConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.signing_secret, "sec123");
        assert_eq!(config.bot_token, "xoxb-tok");
        assert_eq!(config.channel_id.as_deref(), Some("C123"));
        assert_eq!(config.team_id.as_deref(), Some("T123"));
        assert_eq!(config.session_strategy, SessionStrategy::PerChannel);
        assert_eq!(config.reply_mode, SlackReplyMode::ReportProgressOnly);
    }

    #[test]
    fn test_slack_channel_config_minimal() {
        let json = r#"{"signing_secret": "s", "bot_token": "t"}"#;
        let config: SlackChannelConfig = serde_json::from_str(json).unwrap();
        assert!(config.channel_id.is_none());
        assert!(config.team_id.is_none());
        assert_eq!(config.session_strategy, SessionStrategy::PerThread);
        assert_eq!(config.reply_mode, SlackReplyMode::AllMessages);
        assert!(config.webhook_verified_at.is_none());
        assert!(config.first_message_received_at.is_none());
    }

    #[test]
    fn test_slack_channel_config_with_verification_timestamps() {
        let json = r#"{
            "signing_secret": "s",
            "bot_token": "t",
            "webhook_verified_at": "2025-01-01T00:00:00Z",
            "first_message_received_at": "2025-01-01T01:00:00Z"
        }"#;
        let config: SlackChannelConfig = serde_json::from_str(json).unwrap();
        assert!(config.webhook_verified_at.is_some());
        assert!(config.first_message_received_at.is_some());

        // Round-trip: timestamps should be preserved
        let serialized = serde_json::to_value(&config).unwrap();
        assert!(serialized.get("webhook_verified_at").is_some());
        assert!(serialized.get("first_message_received_at").is_some());
    }

    #[test]
    fn test_slack_channel_config_timestamps_skipped_when_none() {
        let config = SlackChannelConfig {
            signing_secret: "s".into(),
            bot_token: "t".into(),
            channel_id: None,
            team_id: None,
            session_strategy: SessionStrategy::PerThread,
            reply_mode: SlackReplyMode::AllMessages,
            webhook_verified_at: None,
            first_message_received_at: None,
        };
        let json = serde_json::to_value(&config).unwrap();
        assert!(json.get("webhook_verified_at").is_none());
        assert!(json.get("first_message_received_at").is_none());
    }

    #[test]
    fn test_slack_reply_mode_serde_roundtrip() {
        let json = serde_json::to_string(&SlackReplyMode::ReportProgressOnly).unwrap();
        assert_eq!(json, r#""report_progress_only""#);
        let parsed: SlackReplyMode = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, SlackReplyMode::ReportProgressOnly);
    }

    #[test]
    fn test_slack_channel_config_missing_required_field() {
        let json = r#"{"signing_secret": "s"}"#;
        assert!(serde_json::from_str::<SlackChannelConfig>(json).is_err());
    }

    #[test]
    fn test_ag_ui_channel_config_defaults_to_anonymous() {
        let config: AgUiChannelConfig = serde_json::from_str("{}").unwrap();
        assert!(config.anonymous);
        assert_eq!(
            config.session_expiration_seconds,
            DEFAULT_SESSION_EXPIRATION_SECONDS
        );
        assert!(config.rate_limit_per_minute.is_none());
        assert!(config.token.is_none());
        assert!(config.auth.is_none());
        assert_eq!(config.tool_visibility, AgUiToolVisibility::Generic);
        assert_eq!(config.generic_tool_text, DEFAULT_AG_UI_GENERIC_TOOL_TEXT);
        assert!(!config.reasoning_summary_visible);
    }

    #[test]
    fn test_ag_ui_channel_config_roundtrip() {
        let config = AgUiChannelConfig {
            anonymous: true,
            token: Some("agui-token".to_string()),
            session_expiration_seconds: 3600,
            rate_limit_per_minute: Some(120),
            tool_visibility: AgUiToolVisibility::None,
            generic_tool_text: "Please wait".to_string(),
            reasoning_summary_visible: true,
            auth: None,
        };
        let json = serde_json::to_string(&config).unwrap();
        let parsed: AgUiChannelConfig = serde_json::from_str(&json).unwrap();
        assert!(parsed.anonymous);
        assert_eq!(parsed.token.as_deref(), Some("agui-token"));
        assert_eq!(parsed.session_expiration_seconds, 3600);
        assert_eq!(parsed.rate_limit_per_minute, Some(120));
        assert_eq!(parsed.tool_visibility, AgUiToolVisibility::None);
        assert_eq!(parsed.generic_tool_text, "Please wait");
        assert!(parsed.reasoning_summary_visible);
    }

    #[test]
    fn test_ag_ui_channel_config_zero_disables_expiration() {
        let config: AgUiChannelConfig =
            serde_json::from_str(r#"{"session_expiration_seconds": 0}"#).unwrap();
        assert_eq!(config.session_expiration_seconds, 0);
    }

    #[test]
    fn test_ag_ui_channel_config_omits_rate_limit_when_unset() {
        let config = AgUiChannelConfig {
            anonymous: true,
            token: None,
            session_expiration_seconds: DEFAULT_SESSION_EXPIRATION_SECONDS,
            rate_limit_per_minute: None,
            tool_visibility: AgUiToolVisibility::Generic,
            generic_tool_text: DEFAULT_AG_UI_GENERIC_TOOL_TEXT.to_string(),
            reasoning_summary_visible: false,
            auth: None,
        };
        let json = serde_json::to_value(&config).unwrap();
        assert!(json.get("rate_limit_per_minute").is_none());
        assert!(json.get("generic_tool_text").is_none());
    }

    #[test]
    fn test_invocation_session_mode_defaults_to_shared_session() {
        assert_eq!(
            InvocationSessionMode::default(),
            InvocationSessionMode::SharedSession
        );
    }

    #[test]
    fn test_schedule_channel_config_defaults() {
        let config: ScheduleChannelConfig =
            serde_json::from_str(r#"{"cron_expression":"0 * * * * * *","message":"Run checks"}"#)
                .unwrap();
        assert_eq!(config.timezone, "UTC");
        assert_eq!(config.session_mode, InvocationSessionMode::SharedSession);
    }

    #[test]
    fn test_webhook_channel_config_defaults() {
        let config: WebhookChannelConfig =
            serde_json::from_str(r#"{"token":"top-secret","message":"{{payload.action}}"}"#)
                .unwrap();
        assert_eq!(config.session_mode, InvocationSessionMode::SharedSession);
        // EVE-627: rate limit is optional and absent by default.
        assert!(config.rate_limit_per_minute.is_none());
    }

    #[test]
    fn test_webhook_channel_config_rate_limit_roundtrip() {
        // EVE-627: an explicit per-channel rate limit parses and re-serializes.
        let config: WebhookChannelConfig =
            serde_json::from_str(r#"{"token":"t","message":"m","rate_limit_per_minute":120}"#)
                .unwrap();
        assert_eq!(config.rate_limit_per_minute, Some(120));
        let json = serde_json::to_value(&config).unwrap();
        assert_eq!(
            json.get("rate_limit_per_minute").and_then(|v| v.as_u64()),
            Some(120)
        );

        // Absent when None (skip_serializing_if).
        let none_cfg: WebhookChannelConfig =
            serde_json::from_str(r#"{"token":"t","message":"m"}"#).unwrap();
        let none_json = serde_json::to_value(&none_cfg).unwrap();
        assert!(none_json.get("rate_limit_per_minute").is_none());
    }

    fn test_app(channels: Vec<AppChannel>) -> App {
        App {
            public_id: AppId::from_uuid(Uuid::nil()),
            internal_id: Uuid::nil(),
            org_id: 1,
            name: "test".into(),
            description: None,
            harness_id: HarnessId::from_uuid(Uuid::nil()),
            agent_id: Some(AgentId::from_uuid(Uuid::nil())),
            agent_version_policy: AgentVersionPolicy::Default,
            agent_version_id: None,
            agent_identity_id: None,
            owner_principal_id: PrincipalId::from_seed(1),
            resolved_owner_user_id: None,
            owner: None,
            effective_owner: None,
            channels,
            status: AppStatus::Draft,
            published_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            archived_at: None,
            deleted_at: None,
        }
    }

    fn test_channel(channel_type: ChannelType, config: serde_json::Value) -> AppChannel {
        AppChannel {
            public_id: AppChannelId::from_uuid(Uuid::nil()),
            internal_id: Uuid::nil(),
            channel_type,
            channel_config: config,
            enabled: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn test_app_channel_slack_config_valid() {
        let ch = test_channel(
            ChannelType::Slack,
            serde_json::json!({"signing_secret": "sec", "bot_token": "tok"}),
        );
        let config = ch.slack_config().unwrap();
        assert_eq!(config.signing_secret, "sec");
    }

    #[test]
    fn test_app_channel_slack_config_invalid_json() {
        let ch = test_channel(ChannelType::Slack, serde_json::json!({"bad": "data"}));
        assert!(ch.slack_config().is_none());
    }

    #[test]
    fn test_app_slack_channel_lookup() {
        let ch = test_channel(
            ChannelType::Slack,
            serde_json::json!({"signing_secret": "s", "bot_token": "t"}),
        );
        let app = test_app(vec![ch]);
        assert!(app.slack_channel().is_some());
    }

    #[test]
    fn test_app_slack_channel_none_when_empty() {
        let app = test_app(vec![]);
        assert!(app.slack_channel().is_none());
    }

    #[test]
    fn test_app_channel_ag_ui_config_valid() {
        let ch = test_channel(ChannelType::AgUi, serde_json::json!({"anonymous": true}));
        let config = ch.ag_ui_config().unwrap();
        assert!(config.anonymous);
    }

    #[test]
    fn test_app_ag_ui_channel_lookup() {
        let ch = test_channel(ChannelType::AgUi, serde_json::json!({"anonymous": true}));
        let app = test_app(vec![ch]);
        assert!(app.ag_ui_channel().is_some());
    }

    #[test]
    fn test_app_channel_fcp_config_defaults() {
        let ch = test_channel(ChannelType::Fcp, serde_json::json!({}));
        let config = ch.fcp_config().unwrap();
        assert!(config.anonymous);
        assert!(config.token.is_none());
        assert!(config.handshake.is_none());
        assert_eq!(
            config.session_expiration_seconds,
            DEFAULT_SESSION_EXPIRATION_SECONDS
        );
        assert_eq!(
            config.response_timeout_seconds,
            DEFAULT_FCP_RESPONSE_TIMEOUT_SECONDS
        );
    }

    #[test]
    fn test_app_fcp_channel_lookup() {
        let ch = test_channel(ChannelType::Fcp, serde_json::json!({}));
        let app = test_app(vec![ch]);
        assert!(app.fcp_channel().is_some());
    }

    #[test]
    fn test_app_channel_schedule_config_valid() {
        let ch = test_channel(
            ChannelType::Schedule,
            serde_json::json!({
                "cron_expression": "0 * * * * * *",
                "message": "Run checks"
            }),
        );
        let config = ch.schedule_config().unwrap();
        assert_eq!(config.message, "Run checks");
    }

    #[test]
    fn test_app_schedule_channel_lookup() {
        let ch = test_channel(
            ChannelType::Schedule,
            serde_json::json!({
                "cron_expression": "0 * * * * * *",
                "message": "Run checks"
            }),
        );
        let app = test_app(vec![ch]);
        assert!(app.schedule_channel().is_some());
    }

    #[test]
    fn test_app_channel_webhook_config_valid() {
        let ch = test_channel(
            ChannelType::Webhook,
            serde_json::json!({
                "token": "secret",
                "message": "{{payload.ref}}"
            }),
        );
        let config = ch.webhook_config().unwrap();
        assert_eq!(config.token, "secret");
    }

    #[test]
    fn test_app_webhook_channel_lookup() {
        let ch = test_channel(
            ChannelType::Webhook,
            serde_json::json!({
                "token": "secret",
                "message": "{{payload.ref}}"
            }),
        );
        let app = test_app(vec![ch]);
        assert!(app.webhook_channel().is_some());
    }

    #[test]
    fn test_a2a_channel_config_defaults() {
        let config: A2aChannelConfig = serde_json::from_str(
            r#"{"api_key_hash":"abc","api_key_prefix":"evra2a_abc1...","message":"{{a2a.text}}"}"#,
        )
        .unwrap();
        assert_eq!(config.session_mode, InvocationSessionMode::SharedSession);
        assert!(config.agent_card_name.is_none());
        assert!(config.agent_card_description.is_none());
        assert!(config.rate_limit_per_minute.is_none());
        assert!(config.auth.is_none());
        assert!(config.signing_secret.is_none());
    }

    #[test]
    fn test_a2a_channel_config_roundtrip() {
        let config = A2aChannelConfig {
            api_key_hash: "deadbeef".into(),
            api_key_prefix: "evra2a_dead...".into(),
            session_mode: InvocationSessionMode::SessionPerInvocation,
            message: "{{a2a.text}}".into(),
            agent_card_name: Some("Inbox triage".into()),
            agent_card_description: Some("Triages github events".into()),
            rate_limit_per_minute: Some(120),
            auth: None,
            signing_secret: None,
        };
        let json = serde_json::to_string(&config).unwrap();
        let parsed: A2aChannelConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.api_key_hash, "deadbeef");
        assert_eq!(
            parsed.session_mode,
            InvocationSessionMode::SessionPerInvocation
        );
        assert_eq!(parsed.agent_card_name.as_deref(), Some("Inbox triage"));
        assert_eq!(parsed.rate_limit_per_minute, Some(120));
    }

    #[test]
    fn test_a2a_channel_config_omits_optional_fields() {
        let config = A2aChannelConfig {
            api_key_hash: "h".into(),
            api_key_prefix: "evra2a_h...".into(),
            session_mode: InvocationSessionMode::SharedSession,
            message: "m".into(),
            agent_card_name: None,
            agent_card_description: None,
            rate_limit_per_minute: None,
            auth: None,
            signing_secret: None,
        };
        let json = serde_json::to_value(&config).unwrap();
        assert!(json.get("agent_card_name").is_none());
        assert!(json.get("agent_card_description").is_none());
        assert!(json.get("rate_limit_per_minute").is_none());
        assert!(json.get("signing_secret").is_none());
    }

    #[test]
    fn test_app_channel_a2a_config_valid() {
        let ch = test_channel(
            ChannelType::A2a,
            serde_json::json!({
                "api_key_hash": "h",
                "api_key_prefix": "evra2a_h...",
                "message": "{{a2a.text}}"
            }),
        );
        let config = ch.a2a_config().unwrap();
        assert_eq!(config.api_key_prefix, "evra2a_h...");
    }

    #[test]
    fn test_app_a2a_channel_lookup() {
        let ch = test_channel(
            ChannelType::A2a,
            serde_json::json!({
                "api_key_hash": "h",
                "api_key_prefix": "evra2a_h...",
                "message": "{{a2a.text}}"
            }),
        );
        let app = test_app(vec![ch]);
        assert!(app.a2a_channel().is_some());
    }

    #[test]
    fn test_channel_type_public_chat_serde_roundtrip() {
        let json = serde_json::to_string(&ChannelType::PublicChat).unwrap();
        assert_eq!(json, r#""public_chat""#);
        let parsed: ChannelType = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, ChannelType::PublicChat);
    }

    #[test]
    fn test_public_chat_channel_config_defaults() {
        let config: PublicChatChannelConfig = serde_json::from_str("{}").unwrap();
        assert!(config.anonymous);
        assert!(config.token.is_none());
        assert_eq!(
            config.session_expiration_seconds,
            DEFAULT_SESSION_EXPIRATION_SECONDS
        );
        assert!(config.rate_limit_per_minute.is_none());
        assert_eq!(config.tool_visibility, AgUiToolVisibility::Generic);
        assert_eq!(config.generic_tool_text, DEFAULT_AG_UI_GENERIC_TOOL_TEXT);
        assert!(config.auth.is_none());
        assert!(config.branding.is_empty());
        assert!(config.captcha.is_none());
    }

    #[test]
    fn test_public_chat_channel_config_omits_empty_branding_and_defaults() {
        let config: PublicChatChannelConfig = serde_json::from_str("{}").unwrap();
        let json = serde_json::to_value(&config).unwrap();
        assert!(json.get("branding").is_none());
        assert!(json.get("captcha").is_none());
        assert!(json.get("generic_tool_text").is_none());
        assert!(json.get("rate_limit_per_minute").is_none());
    }

    #[test]
    fn test_public_chat_channel_config_full_roundtrip() {
        let json = r##"{
            "anonymous": false,
            "token": "shared-secret",
            "session_expiration_seconds": 3600,
            "rate_limit_per_minute": 30,
            "tool_visibility": "narrated",
            "generic_tool_text": "Thinking...",
            "branding": {
                "display_name": "Support",
                "logo_url": "https://example.com/logo.png",
                "primary_color": "#0A1636",
                "welcome_message": "How can I help?"
            },
            "captcha": {
                "provider": "turnstile",
                "enabled": true,
                "site_key": "1x00000000000000000000AA",
                "secret_key": "1x0000000000000000000000000000000AA"
            }
        }"##;
        let config: PublicChatChannelConfig = serde_json::from_str(json).unwrap();
        assert!(!config.anonymous);
        assert_eq!(config.token.as_deref(), Some("shared-secret"));
        assert_eq!(config.session_expiration_seconds, 3600);
        assert_eq!(config.rate_limit_per_minute, Some(30));
        assert_eq!(config.tool_visibility, AgUiToolVisibility::Narrated);
        let branding = &config.branding;
        assert_eq!(branding.display_name.as_deref(), Some("Support"));
        assert_eq!(branding.primary_color.as_deref(), Some("#0A1636"));
        let captcha = config.captcha.unwrap();
        assert_eq!(captcha.provider, CaptchaProvider::Turnstile);
        assert!(captcha.enabled);
        assert_eq!(captcha.site_key, "1x00000000000000000000AA");
        assert!(captcha.secret_key.is_some());
    }

    #[test]
    fn test_app_channel_public_chat_config_valid() {
        let ch = test_channel(
            ChannelType::PublicChat,
            serde_json::json!({"anonymous": true}),
        );
        let config = ch.public_chat_config().unwrap();
        assert!(config.anonymous);
    }

    #[test]
    fn test_app_public_chat_channel_lookup() {
        let ch = test_channel(
            ChannelType::PublicChat,
            serde_json::json!({"branding": {"display_name": "Helpdesk"}}),
        );
        let app = test_app(vec![ch]);
        assert!(app.public_chat_channel().is_some());
    }

    #[test]
    fn test_app_serde_skips_internal_fields() {
        let app = test_app(vec![]);
        let json = serde_json::to_value(&app).unwrap();
        assert!(json.get("id").is_some()); // public_id serialized as "id"
        assert!(json.get("internal_id").is_none()); // skipped
        assert!(json.get("org_id").is_none()); // skipped
        assert!(json.get("published_at").is_none()); // None skipped
    }
}

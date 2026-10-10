// Agent-owned channels. Serialized IDs and transport values remain stable.

pub mod api;
pub mod exposure;
pub mod pact_delegation;
pub mod poppy;
pub mod slack_channel;
pub mod slack_provisioning;

#[cfg(test)]
mod wire_names_tests;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[cfg(test)]
use crate::domains::apps::record::{App, AppStatus};
use everruns_contracts::typed_id::AgentChannelId;
#[cfg(test)]
use everruns_contracts::typed_id::{AgentId, AppId, HarnessId, PrincipalId};
pub use everruns_core::channel::SessionBinding;
use exposure::{DEFAULT_PUBLIC_TOOL_ACTIVITY_TEXT, PublicToolVisibility};

use utoipa::ToSchema;

/// Per-channel lifecycle (EVE-1007).
///
/// This is the authority for whether an exposure accepts traffic. It replaced
/// the two-dimensional `App.status × AppChannel.enabled` matrix, which could
/// express "published App, disabled channel" and forced publishing a whole App —
/// and therefore every sibling channel on it — to make one endpoint reachable.
///
/// Liveness also depends on agent-level conditions, which are folded in at
/// resolution time rather than stored here.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[schema(example = "live")]
#[serde(rename_all = "lowercase")]
pub enum ChannelStatus {
    /// Configured but never published; refuses traffic.
    #[default]
    Draft,
    /// Published and accepting traffic, subject to the agent-level terms.
    Live,
    /// Explicitly turned off; refuses traffic.
    Disabled,
}

impl ChannelStatus {
    /// Whether the channel's own state permits traffic. Callers must still
    /// apply the agent-level terms.
    pub fn is_live(self) -> bool {
        matches!(self, ChannelStatus::Live)
    }
}

impl std::fmt::Display for ChannelStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChannelStatus::Draft => write!(f, "draft"),
            ChannelStatus::Live => write!(f, "live"),
            ChannelStatus::Disabled => write!(f, "disabled"),
        }
    }
}

impl From<&str> for ChannelStatus {
    fn from(s: &str) -> Self {
        match s {
            "live" => ChannelStatus::Live,
            "disabled" => ChannelStatus::Disabled,
            _ => ChannelStatus::Draft,
        }
    }
}

/// Supported channel types for app distribution.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ToSchema)]
#[schema(example = "webhook")]
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
    /// optional handshake.
    Fcp,
    /// App-scoped, execution-only API key over native session routes.
    #[serde(rename = "api_endpoint")]
    ApiEndpoint,
    /// Public Chat channel — an isolated, public-facing chat web app bound to a
    /// single App's agent. Anonymous by default, with optional Google sign-in
    /// and Cloudflare Turnstile bot mitigation. Reuses AG-UI streaming and the
    /// shared channel auth verifier.
    #[serde(rename = "public_chat")]
    PublicChat,
    /// Voice channel: callers talk to the agent; a speech model listens and
    /// speaks while the agent writes every answer (`VoiceChannelConfig`).
    Voice,
    /// Agent Execution API: the agent's base URL for code, called with agent
    /// keys (`api::AgentApiChannelConfig`).
    Api,
    /// Personal Agent Protocol front door: personal agents find the company,
    /// start sessions and talk to the agent (`poppy::PoppyChannelConfig`).
    Poppy,
}

impl ChannelType {
    /// The bindings this transport can actually offer.
    ///
    /// A transport constrains the binding because of what it *is*, not because
    /// of a second enum: a schedule or webhook has no thread and no requester to
    /// key on, so only `Shared` and `Ephemeral` mean anything there, while a
    /// messaging channel has no single "the channel's session" to share. Before
    /// EVE-1005 this distinction was carried by having two enums, which is why
    /// every new surface had to pick a side.
    ///
    /// `AgUi`, `Fcp`, `ApiEndpoint` and `PublicChat` carry no binding field
    /// today — their session handling is decided by the caller per request — so
    /// they offer none and reject every value.
    pub fn allowed_bindings(&self) -> &'static [SessionBinding] {
        match self {
            // Messaging: keyed off the inbound message.
            ChannelType::Slack => &SessionBinding::MESSAGE_KEYED,
            // Nothing is listening on a thread; the exposure owns the session.
            ChannelType::Schedule
            | ChannelType::Webhook
            | ChannelType::A2a
            | ChannelType::ApiEndpoint => &SessionBinding::INVOCATION_KEYED,
            ChannelType::Api => &api::API_BINDINGS,
            // Poppy: one session per conversation, chosen by the caller.
            ChannelType::AgUi
            | ChannelType::Fcp
            | ChannelType::PublicChat
            | ChannelType::Voice
            | ChannelType::Poppy => &[],
        }
    }

    /// Whether this transport can offer `binding`.
    pub fn allows_binding(&self, binding: SessionBinding) -> bool {
        self.allowed_bindings().contains(&binding)
    }
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
            ChannelType::Voice => write!(f, "voice"),
            ChannelType::Api => write!(f, "api"),
            ChannelType::Poppy => write!(f, "poppy"),
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
            "voice" => Some(ChannelType::Voice),
            "api" => Some(ChannelType::Api),
            "poppy" => Some(ChannelType::Poppy),
            _ => None,
        }
    }
}

/// An independently published communication channel owned by an Agent.
/// Each channel has its own type, config, and lifecycle status.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AgentChannel {
    /// External identifier (appchan_<32-hex>). Shown as "id" in API.
    #[serde(rename = "id")]
    #[schema(value_type = String, example = "appchan_01933b5a000070008000000000000001")]
    pub public_id: AgentChannelId,
    /// Internal UUID primary key. Never exposed in API.
    #[serde(skip, default = "Uuid::nil")]
    pub internal_id: Uuid,
    /// Channel type (e.g. slack).
    pub channel_type: ChannelType,
    /// Channel-specific configuration (validated per channel type).
    #[serde(default)]
    pub channel_config: serde_json::Value,
    /// Authentication policy for this channel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<Box<ChannelAuthConfig>>,
    /// Whether this channel is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Per-channel lifecycle. Authoritative for ingress (EVE-1007); `enabled`
    /// is retained for the App API's existing shape.
    #[serde(default)]
    pub status: ChannelStatus,
    /// Timestamp when this channel was created.
    pub created_at: DateTime<Utc>,
    /// Timestamp when this channel was last updated.
    pub updated_at: DateTime<Utc>,
}

fn default_true() -> bool {
    true
}

impl AgentChannel {
    /// Parse channel_config as SlackChannelConfig. Returns None if not a Slack channel
    /// or if the config is invalid.
    pub fn slack_config(&self) -> Option<slack_channel::SlackChannelConfig> {
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

    /// Parse channel_config as ApiChannelConfig. Returns None if not an
    /// api_endpoint channel or if the config is invalid.
    pub fn api_channel_config(&self) -> Option<ApiChannelConfig> {
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

    /// Parse channel_config as a Poppy channel's config.
    pub fn poppy_config(&self) -> Option<poppy::PoppyChannelConfig> {
        (self.channel_type == ChannelType::Poppy)
            .then(|| serde_json::from_value(self.channel_config.clone()).ok())
            .flatten()
    }

    /// Parse channel_config as a voice channel's config.
    pub fn voice_config(&self) -> Option<everruns_contracts::voice::VoiceChannelConfig> {
        (self.channel_type == ChannelType::Voice)
            .then(|| serde_json::from_value(self.channel_config.clone()).ok())
            .flatten()
    }
}

// These accessors answer "which channel of this type does this app have",
// and deliberately filter on `enabled` rather than liveness. Liveness is
// applied separately by `channel_liveness` at the ingress gates, because
// non-ingress callers — Slack delivery recovery for sessions that are
// already running, for one — must keep working when an endpoint is
// unpublished or its agent is suspended. Folding liveness in here would
// orphan in-flight deliveries.

/// The binding a schedule, webhook, A2A or api_endpoint exposure declares.
///
/// Those transports have no thread to key on, so they default to one durable
/// session shared by every invocation — the former `shared_session`. Spelled as
/// a function because `SessionBinding::default()` is `Thread`, which is the
/// right default for messaging and the wrong one here (EVE-1005).
pub(crate) fn default_invocation_binding() -> SessionBinding {
    SessionBinding::Shared
}

/// How replies are delivered back to Slack.
///
/// This is the Slack-specific config type that serializes in `SlackChannelConfig`.
/// Converts to/from the generic `ChannelReplyMode` in `everruns_core::channel`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SlackReplyMode {
    /// Forward completed assistant messages directly to Slack.
    #[default]
    AllMessages,
    /// Only publish messages explicitly posted through `channel_post_message`.
    #[serde(alias = "report_progress_only")]
    ToolOnly,
}

impl From<SlackReplyMode> for everruns_core::channel::ChannelReplyMode {
    fn from(m: SlackReplyMode) -> Self {
        match m {
            SlackReplyMode::AllMessages => Self::AllMessages,
            SlackReplyMode::ToolOnly => Self::ToolOnly,
        }
    }
}

impl From<everruns_core::channel::ChannelReplyMode> for SlackReplyMode {
    fn from(m: everruns_core::channel::ChannelReplyMode) -> Self {
        match m {
            everruns_core::channel::ChannelReplyMode::AllMessages => Self::AllMessages,
            everruns_core::channel::ChannelReplyMode::ToolOnly => Self::ToolOnly,
        }
    }
}

/// Default session expiration for public channel threads (6 hours).
pub const DEFAULT_SESSION_EXPIRATION_SECONDS: u32 = 6 * 60 * 60;

/// Default public AG-UI text shown while a tool call is running.
pub const DEFAULT_AG_UI_GENERIC_TOOL_TEXT: &str = DEFAULT_PUBLIC_TOOL_ACTIVITY_TEXT;

/// Agent channel authentication mode.
///
/// Stored on `AgentChannel.auth` so users can protect one endpoint without first
/// creating org-level identity-provider state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChannelAuthMode {
    Anonymous,
    SharedSecret,
    ApiKey,
    GoogleOidc,
    Oidc,
    #[serde(rename = "oauth2_introspection", alias = "o_auth2_introspection")]
    OAuth2Introspection,
    HttpBasic,
    Mtls,
}

/// OIDC/OAuth/basic/mTLS provider details for one channel.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChannelAuthProviderConfig {
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
    #[serde(rename = "oauth2_introspection", alias = "o_auth2_introspection")]
    OAuth2Introspection {
        introspection_url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        client_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        client_secret: Option<String>,
        #[serde(default, skip_serializing_if = "is_false")]
        client_secret_configured: bool,
    },
    HttpBasic {
        username: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        password: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        password_hash: Option<String>,
        #[serde(default, skip_serializing_if = "is_false")]
        password_configured: bool,
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
        #[serde(default, skip_serializing_if = "is_false")]
        proxy_secret_configured: bool,
    },
}

/// Claim and credential requirements common to channel auth providers.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
pub struct ChannelAuthRequirements {
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

/// Authentication config for one channel/channel.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[schema(example = json!({"mode": "api_key", "requirements": {"audiences": ["everruns-api"], "scopes": ["app:invoke"]}}))]
pub struct ChannelAuthConfig {
    pub mode: ChannelAuthMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<ChannelAuthProviderConfig>,
    #[serde(default)]
    pub requirements: ChannelAuthRequirements,
}

/// Issuer of AgentID, AgentMail's OpenID Connect provider for AI agents.
pub const AGENTID_ISSUER: &str = "https://auth.agentid.com";

/// Binding provider for subjects proven by AgentID's own discovered keys.
pub const AGENTID_PROVIDER: &str = "agentid";

impl ChannelAuthConfig {
    /// The AgentID channel preset.
    ///
    /// Decision: AgentID is ordinary `oidc` channel auth, not a verifier mode of
    /// its own. The preset pins the issuer, leaves the keys to AgentID discovery,
    /// requires the operator's registered client id as audience, and requires
    /// `actor_type = "agent"` (every AgentID subject is an agent inbox).
    pub fn agentid_preset(client_id: &str) -> Self {
        let mut claims = serde_json::Map::new();
        claims.insert(
            "actor_type".to_string(),
            serde_json::Value::String("agent".to_string()),
        );
        Self {
            mode: ChannelAuthMode::Oidc,
            provider: Some(ChannelAuthProviderConfig::Oidc {
                issuer: AGENTID_ISSUER.to_string(),
                jwks_url: None,
            }),
            requirements: ChannelAuthRequirements {
                audiences: vec![client_id.trim().to_string()],
                claims,
                ..Default::default()
            },
        }
    }

    /// Whether this config verifies AgentID tokens against AgentID's own
    /// discovered keys. A config that names its own JWKS URL is not AgentID,
    /// whatever issuer it claims.
    pub fn is_agentid(&self) -> bool {
        self.mode == ChannelAuthMode::Oidc
            && matches!(
                self.provider.as_ref(),
                Some(ChannelAuthProviderConfig::Oidc { issuer, jwks_url: None })
                    if issuer.trim().trim_end_matches('/') == AGENTID_ISSUER
            )
    }
}

/// Typed AG-UI channel configuration.
///
/// Parsed from the `channel_config` JSON field on App.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AgUiChannelConfig {
    /// Whether anonymous access is allowed for this channel (default on).
    #[serde(default = "default_true")]
    pub anonymous: bool,
    /// Optional shared bearer token for the public AG-UI endpoint.
    /// When set, requests must include the token in a supported header.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    /// Seconds a thread stays resumable after its session was created; after
    /// that the `thread_id` starts a new session. `0` disables. Default 6 hours.
    #[serde(default = "default_session_expiration_seconds")]
    pub session_expiration_seconds: u32,
    /// Optional per-IP rate limit in requests per minute; `None` or `Some(0)`
    /// disables the per-app limit (the global API limit still applies).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_limit_per_minute: Option<u32>,
    /// Public tool activity visibility for anonymous AG-UI streams.
    #[serde(default)]
    pub tool_visibility: PublicToolVisibility,
    /// Generic public text shown when `tool_visibility` is `generic`.
    #[serde(
        default = "default_ag_ui_generic_tool_text",
        skip_serializing_if = "is_default_ag_ui_generic_tool_text"
    )]
    pub generic_tool_text: String,
    /// Emit provider reasoning summaries; off because endpoints can be anonymous
    /// and summaries may derive from private prompts, tools, or retrieved data.
    #[serde(default, skip_serializing_if = "is_false")]
    pub reasoning_summary_visible: bool,
    /// Let the client answer tool-approval interrupts; off so anonymous visitors cannot (TM-TOOL-052).
    #[serde(default, skip_serializing_if = "is_false")]
    pub tool_approval_interrupts: bool,
    /// Report token usage on terminal run events; off because it reveals model and cost.
    #[serde(default, skip_serializing_if = "is_false")]
    pub usage_visible: bool,
    /// Stream subagent work as `SUBAGENT_*`; off because child output and errors reach the client.
    #[serde(default, skip_serializing_if = "is_false")]
    pub subagents_visible: bool,
    /// Stream the agent's todo list as AG-UI shared state; off because todos
    /// are `write_todos` arguments, which public tool visibility never shows.
    #[serde(default, skip_serializing_if = "is_false")]
    pub state_visible: bool,
    /// Optional inline auth config for this public endpoint. When omitted,
    /// legacy `anonymous` + `token` behavior applies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<ChannelAuthConfig>,
}

fn default_session_expiration_seconds() -> u32 {
    DEFAULT_SESSION_EXPIRATION_SECONDS
}

pub(crate) fn default_ag_ui_generic_tool_text() -> String {
    DEFAULT_AG_UI_GENERIC_TOOL_TEXT.to_string()
}

pub(crate) fn is_default_ag_ui_generic_tool_text(value: &str) -> bool {
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
/// FCP is intentionally schema-free at the wire layer. The fields here only control server-side
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
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct FcpChannelConfig {
    /// Whether anonymous access is allowed for this channel. When `false`
    /// a non-empty `token` must authenticate every `POST`.
    #[serde(default = "default_true")]
    pub anonymous: bool,
    /// Optional shared bearer token. When set, callers must send
    /// `Authorization: Bearer <token>` (or the `X-Everruns-FCP-Token`
    /// header). Validated by constant-time comparison inside the FCP
    /// handler — never via the shared channel auth verifier.
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

/// Typed schedule channel configuration.
///
/// `message` is also the template body. `{{path.to.value}}` placeholders are
/// expanded at invocation time.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ScheduleChannelConfig {
    /// Cron expression that drives the durable schedule.
    pub cron_expression: String,
    /// IANA timezone identifier for cron evaluation.
    #[serde(default = "default_timezone")]
    pub timezone: String,
    /// Whether invocations reuse a stable session or create a new one.
    #[serde(default = "default_invocation_binding")]
    pub session_mode: SessionBinding,
    /// Message content or template sent when the schedule fires.
    pub message: String,
}

/// Typed webhook channel configuration.
///
/// `message` is also the template body. `{{path.to.value}}` placeholders are
/// expanded against the incoming webhook payload and metadata.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct WebhookChannelConfig {
    /// Shared secret required from the incoming webhook request.
    pub token: String,
    /// Whether invocations reuse a stable session or create a new one.
    #[serde(default = "default_invocation_binding")]
    pub session_mode: SessionBinding,
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
/// against the incoming A2A request payload and metadata.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct A2aChannelConfig {
    /// SHA-256 hex digest of the API key.
    pub api_key_hash: String,
    /// Public, non-secret display prefix (e.g. `evra2a_abc1...`).
    pub api_key_prefix: String,
    /// Whether invocations reuse a stable session or create a new one.
    #[serde(default = "default_invocation_binding")]
    pub session_mode: SessionBinding,
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
    pub auth: Option<ChannelAuthConfig>,
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
    /// Optional PACT Identity profile. When set, the endpoint is also served
    /// at `/v1/channels/{channel_id}/a2a/pact` to the personal agents listed here, which
    /// authenticate with JWTs they sign instead of the channel API key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pact: Option<PactProfileConfig>,
}

/// PACT Identity profile for an A2A endpoint (Personal Agent Consent and
/// Trust, <https://github.com/openpactprotocol/openpactprotocol>). Holds only
/// public data: issuers, audiences, and public keys.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq)]
pub struct PactProfileConfig {
    /// The `aud` every personal agent puts in its tokens, verbatim. One value
    /// per provider, never derived from the card URL.
    pub audience: String,
    /// Personal agents allowed to call. Any other issuer gets `401`.
    pub personal_agents: Vec<PactPersonalAgent>,
    /// Optional PACT Delegated profile: personal agents may act on the user's
    /// account with the company after the user signs in and consents.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delegation: Option<pact_delegation::PactDelegationConfig>,
}

/// A personal agent registered with a PACT endpoint.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq)]
pub struct PactPersonalAgent {
    /// Exact `iss` the personal agent signs with.
    pub issuer: String,
    /// HTTPS URL of the personal agent's JWKS. Set this or `jwks`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jwks_uri: Option<String>,
    /// The personal agent's public keys inline, as a JWKS document. Set this
    /// or `jwks_uri`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Object>)]
    pub jwks: Option<serde_json::Value>,
    /// A disabled personal agent is refused with `401`.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

/// Typed api_endpoint channel configuration.
///
/// Exposes an app-scoped, execution-only API key over native session routes
/// mounted under the app (`/v1/apps/{app_id}/api/{channel_id}/...`). The key is
/// structurally execution-only: it reaches only these app-mounted routes and
/// has no path to any management API. The plaintext key is **never** stored —
/// only the SHA-256 hex hash and a non-secret display prefix are persisted, and
/// the plaintext is returned exactly once at create / regenerate time.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ApiChannelConfig {
    /// SHA-256 hex digest of the API key.
    pub api_key_hash: String,
    /// Public, non-secret display prefix (e.g. `evr_app_abc1...`).
    pub api_key_prefix: String,
    /// Whether invocations reuse a stable session or create a new one.
    #[serde(default = "default_invocation_binding")]
    pub session_mode: SessionBinding,
    /// Optional per-IP rate limit applied to this app's api_endpoint, in
    /// requests per minute. `None` or `Some(0)` disables the per-channel limit
    /// (the global API limit still applies). Mirrors
    /// `A2aChannelConfig::rate_limit_per_minute`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_limit_per_minute: Option<u32>,
    /// Optional inline auth config for this channel. When omitted, the
    /// generated per-channel API-key bearer scheme applies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<ChannelAuthConfig>,
}

/// Branding shown on a Public Chat surface. All fields optional; the public app
/// falls back to the App's name and the default design-system theme when unset.
/// Branding is non-secret and is returned as-is in API responses.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
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
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default, ToSchema)]
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
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
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
/// AG-UI's streaming semantics and the shared channel auth verifier, and
/// adds branding and bot-mitigation tailored to a public, link-shareable chat
/// website.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
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
    pub tool_visibility: PublicToolVisibility,
    /// Generic public text shown when `tool_visibility` is `generic`/`narrated`.
    #[serde(
        default = "default_ag_ui_generic_tool_text",
        skip_serializing_if = "is_default_ag_ui_generic_tool_text"
    )]
    pub generic_tool_text: String,
    /// Optional inline auth config for this public endpoint (e.g. Google OIDC).
    /// When omitted, anonymous + optional `token` behavior applies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<ChannelAuthConfig>,
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
    /// Project the streaming fields onto an `AgUiChannelConfig` for the shared AG-UI core.
    /// Branding and captcha stay with the Public Chat handler; approvals and usage stay off.
    pub fn ag_ui_stream_config(&self) -> AgUiChannelConfig {
        AgUiChannelConfig {
            anonymous: self.anonymous,
            token: self.token.clone(),
            session_expiration_seconds: self.session_expiration_seconds,
            rate_limit_per_minute: self.rate_limit_per_minute,
            tool_visibility: self.tool_visibility,
            generic_tool_text: self.generic_tool_text.clone(),
            reasoning_summary_visible: false,
            tool_approval_interrupts: false,
            usage_visible: false,
            subagents_visible: false,
            state_visible: false,
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
mod tests;

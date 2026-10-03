// Agent trigger domain types
//
// Design Decision:
// - AgentTrigger is an org-scoped, agent-owned entity that describes how an
//   agent gets invoked autonomously (e.g. on a schedule). It mirrors the
//   VirtualUser CRUD shape (see `virtual_user.rs`).
// - The concrete per-type configuration lives in `config` (JSONB). Typed
//   accessors parse it on demand, mirroring `AgentEndpoint::schedule_config()`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// Reuse the app-side invocation/schedule config so schedule triggers and
// schedule channels share one shape. Do not duplicate these.
use crate::records::app::default_invocation_binding;
use everruns_contracts::typed_id::{AgentEndpointId, AgentId, TriggerId};
use everruns_core::channel::SessionBinding;

use utoipa::ToSchema;

/// The kind of event that fires an agent trigger.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum AgentTriggerType {
    /// Cron-driven schedule trigger.
    #[default]
    Schedule,
    /// Token-authenticated HTTP webhook trigger.
    Webhook,
    /// GitHub events delivered to the agent identity's GitHub App.
    GitHub,
    /// MCP Events from one of the agent's MCP servers, delivered to a signed
    /// webhook subscription Everruns holds on the agent's behalf.
    #[serde(rename = "mcp_event")]
    McpEvent,
}

impl std::fmt::Display for AgentTriggerType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentTriggerType::Schedule => write!(f, "schedule"),
            AgentTriggerType::Webhook => write!(f, "webhook"),
            AgentTriggerType::GitHub => write!(f, "github"),
            AgentTriggerType::McpEvent => write!(f, "mcp_event"),
        }
    }
}

impl From<&str> for AgentTriggerType {
    fn from(value: &str) -> Self {
        match value {
            "webhook" => Self::Webhook,
            "github" => Self::GitHub,
            "mcp_event" => Self::McpEvent,
            _ => Self::Schedule,
        }
    }
}

/// Typed configuration for a `Schedule` trigger.
///
/// `message` is also the template body. `{{path.to.value}}` placeholders are
/// expanded at invocation time. Mirrors `app::ScheduleChannelConfig`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ScheduleTriggerConfig {
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

/// Typed configuration for a `Webhook` trigger.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct WebhookTriggerConfig {
    /// Shared secret accepted through bearer or webhook-token authentication.
    pub token: String,
    /// Whether invocations reuse a stable session or create a new one.
    #[serde(default = "default_invocation_binding")]
    pub session_mode: SessionBinding,
    /// Message template rendered with the webhook request context.
    pub message: String,
    /// Optional per-ingress, per-IP request limit. `0` disables this limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_limit_per_minute: Option<u32>,
    /// Optional template for the delivery's idempotency key, e.g.
    /// `{{webhook.headers.x-github-delivery}}`. A repeated key is recorded as a
    /// duplicate and never starts a second run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_id_template: Option<String>,
    /// Optional template for the event's subject, e.g.
    /// `{{payload.repository.full_name}}#{{payload.number}}`. With
    /// `session_mode: per_thread`, every event with the same subject continues
    /// one session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject_template: Option<String>,
    /// Optional conditions an event must meet to start a run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<TriggerEventFilter>,
}

/// Typed configuration for a `GitHub` trigger.
///
/// Events arrive through the webhook of the GitHub App the agent's identity
/// created ("Connect GitHub"), signed with that App's secret, so the trigger
/// holds no token of its own. The event context carries `github.*` fields
/// (`event`, `action`, `repository`, `number`, `title`, `url`, `sender`) and
/// the raw `payload`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct GitHubTriggerConfig {
    /// Subscribed events: an event name (`pull_request`) or an event and
    /// action (`pull_request.opened`).
    pub events: Vec<String>,
    /// Repositories (`owner/name`) to accept. Empty accepts every repository
    /// the installation can see.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub repositories: Vec<String>,
    /// Session strategy. `per_thread` keeps one session per pull request or
    /// issue.
    #[serde(default = "default_github_binding")]
    pub session_mode: SessionBinding,
    /// Message template rendered with the event context.
    pub message: String,
    /// Optional extra conditions an event must meet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<TriggerEventFilter>,
}

fn default_github_binding() -> SessionBinding {
    SessionBinding::Thread
}

/// Typed configuration for an `McpEvent` trigger.
///
/// Everruns subscribes, as the agent, to `event` on the agent's MCP server
/// `server` (an attachment name from the agent's or its harness's MCP
/// configuration) through `events/subscribe`, and the server POSTs signed
/// events to the trigger's callback. The event context carries `mcp.*` fields
/// (`server`, `event`, `event_id`, `timestamp`, `subscription_id`) and the
/// event's `data` as `payload`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ToSchema)]
pub struct McpEventTriggerConfig {
    /// Name of the agent's MCP server attachment that publishes the event.
    pub server: String,
    /// Event name from the server's `events/list`.
    pub event: String,
    /// Subscription arguments (the event's `inputSchema`), sent with
    /// `events/subscribe`.
    #[serde(default = "empty_object")]
    #[schema(value_type = Object)]
    pub arguments: serde_json::Value,
    /// Session strategy. `per_thread` needs a `subject_template`.
    #[serde(default = "default_invocation_binding")]
    pub session_mode: SessionBinding,
    /// Message template rendered with the event context.
    pub message: String,
    /// Optional template for the event's subject, e.g. `{{payload.issue_id}}`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject_template: Option<String>,
    /// Optional conditions an event must meet to start a run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<TriggerEventFilter>,
}

fn empty_object() -> serde_json::Value {
    serde_json::Value::Object(Default::default())
}

/// Conditions an incoming event must satisfy before a trigger runs.
///
/// Every condition must hold. A condition reads one dotted path of the event's
/// template context (`payload.action`, `event.type`, ...) and passes when the
/// value there equals any of `any_of`. Events that do not match are recorded as
/// `filtered` deliveries and start no session.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, ToSchema)]
pub struct TriggerEventFilter {
    /// Conditions that must all hold.
    #[serde(default)]
    pub conditions: Vec<TriggerFilterCondition>,
}

/// One filter condition: the value at `path` must equal one of `any_of`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ToSchema)]
pub struct TriggerFilterCondition {
    /// Dotted path into the event context, e.g. `payload.action`.
    pub path: String,
    /// Accepted values. Strings compare exactly; other JSON values compare by
    /// equality.
    #[schema(value_type = Vec<Object>)]
    pub any_of: Vec<serde_json::Value>,
}

/// What happened to one event delivered to a trigger.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum TriggerDeliveryStatus {
    /// Accepted and handed to a session.
    Dispatched,
    /// Did not match the trigger's filter; no session started.
    Filtered,
    /// Same event id as an earlier delivery; no session started.
    Duplicate,
    /// Accepted but the run could not be started.
    Failed,
}

impl TriggerDeliveryStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Dispatched => "dispatched",
            Self::Filtered => "filtered",
            Self::Duplicate => "duplicate",
            Self::Failed => "failed",
        }
    }
}

impl std::str::FromStr for TriggerDeliveryStatus {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "dispatched" => Ok(Self::Dispatched),
            "filtered" => Ok(Self::Filtered),
            "duplicate" => Ok(Self::Duplicate),
            "failed" => Ok(Self::Failed),
            other => anyhow::bail!("unknown trigger delivery status {other}"),
        }
    }
}

/// One recorded event delivery for an agent trigger.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AgentTriggerDelivery {
    /// Delivery identifier.
    #[schema(value_type = String)]
    pub id: uuid::Uuid,
    /// Where the event came from (`schedule`, `webhook`, ...).
    pub source: String,
    /// Source event identifier used for deduplication, when the source has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_id: Option<String>,
    /// Source event type, when the source has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_type: Option<String>,
    /// Subject the event is about (for example `owner/repo#12`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    /// Outcome.
    pub status: TriggerDeliveryStatus,
    /// Why the event was filtered or failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Session that handled the event.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub session_id: Option<everruns_contracts::typed_id::SessionId>,
    /// When the event was received.
    pub created_at: DateTime<Utc>,
}

fn default_timezone() -> String {
    "UTC".to_string()
}

/// AgentTrigger is a durable, agent-owned invocation trigger.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AgentTrigger {
    /// External identifier (trg_<32-hex>). Shown as `id` in API.
    #[serde(rename = "id")]
    #[schema(value_type = String, example = "trg_01933b5a000070008000000000000001")]
    pub id: TriggerId,
    /// Agent that owns this trigger.
    #[schema(value_type = String, example = "agent_01933b5a000070008000000000000001")]
    pub agent_id: AgentId,
    /// The kind of event that fires this trigger.
    pub trigger_type: AgentTriggerType,
    /// Stable HTTP ingress identifier for trigger types that accept requests.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub ingress_id: Option<AgentEndpointId>,
    /// Type-specific configuration (parsed via typed accessors).
    pub config: serde_json::Value,
    /// Whether the trigger is currently active.
    pub enabled: bool,
    /// Which Agent version sessions started by this trigger run.
    #[serde(default)]
    pub agent_version_policy: crate::records::app::AgentVersionPolicy,
    /// Pinned Agent version. Set only when `agent_version_policy` is `pinned`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, example = "agentver_01933b5a00007000800000000000001")]
    pub agent_version_id: Option<everruns_contracts::typed_id::AgentVersionId>,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Last update timestamp.
    pub updated_at: DateTime<Utc>,
    /// Archive timestamp.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<DateTime<Utc>>,
    /// Delete timestamp.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<DateTime<Utc>>,
}

impl AgentTrigger {
    /// Parse `config` as [`ScheduleTriggerConfig`]. Errors if this is not a
    /// schedule trigger or the config is malformed. Mirrors
    /// `AgentEndpoint::schedule_config()`.
    pub fn schedule_config(&self) -> anyhow::Result<ScheduleTriggerConfig> {
        if self.trigger_type != AgentTriggerType::Schedule {
            anyhow::bail!("agent trigger {} is not a schedule trigger", self.id);
        }
        Ok(serde_json::from_value(self.config.clone())?)
    }

    /// Parse `config` as [`GitHubTriggerConfig`].
    pub fn github_config(&self) -> anyhow::Result<GitHubTriggerConfig> {
        if self.trigger_type != AgentTriggerType::GitHub {
            anyhow::bail!("agent trigger {} is not a GitHub trigger", self.id);
        }
        Ok(serde_json::from_value(self.config.clone())?)
    }

    /// Parse `config` as [`McpEventTriggerConfig`].
    pub fn mcp_event_config(&self) -> anyhow::Result<McpEventTriggerConfig> {
        if self.trigger_type != AgentTriggerType::McpEvent {
            anyhow::bail!("agent trigger {} is not an MCP event trigger", self.id);
        }
        Ok(serde_json::from_value(self.config.clone())?)
    }

    /// Parse `config` as [`WebhookTriggerConfig`].
    pub fn webhook_config(&self) -> anyhow::Result<WebhookTriggerConfig> {
        if self.trigger_type != AgentTriggerType::Webhook {
            anyhow::bail!("agent trigger {} is not a webhook trigger", self.id);
        }
        Ok(serde_json::from_value(self.config.clone())?)
    }
}

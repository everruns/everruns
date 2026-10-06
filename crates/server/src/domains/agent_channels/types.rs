use crate::records::{AgentVersionPolicy, ChannelType};
use everruns_contracts::typed_id::AgentVersionId;
use serde::Deserialize;
use serde_json::Value;
use utoipa::ToSchema;

/// Request to create an ingress channel owned by an Agent.
#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct CreateAgentChannelRequest {
    /// Transport used by the channel.
    pub channel_type: ChannelType,
    /// Transport-specific channel configuration.
    #[serde(default)]
    pub channel_config: Value,
    /// Whether the channel can accept ingress traffic.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Which Agent version sessions started through this channel run.
    /// Omitted means `default` (the agent's default version).
    #[serde(default)]
    pub agent_version_policy: Option<AgentVersionPolicy>,
    /// Version to run when `agent_version_policy` is `pinned`. Must be a saved
    /// version of this agent.
    #[serde(default)]
    #[schema(value_type = Option<String>, example = "agentver_01933b5a00007000800000000000001")]
    pub agent_version_id: Option<AgentVersionId>,
}

/// Request to update an ingress channel owned by an Agent.
#[derive(Debug, Clone, Default, Deserialize, ToSchema, serde::Serialize)]
pub struct UpdateAgentChannelRequest {
    /// Replacement transport-specific channel configuration.
    pub channel_config: Option<Value>,
    /// Whether the channel can accept ingress traffic.
    pub enabled: Option<bool>,
    /// Replacement version policy. `pinned` keeps the current pin when
    /// `agent_version_id` is omitted; `default` or `latest` clears the pin.
    #[serde(default)]
    pub agent_version_policy: Option<AgentVersionPolicy>,
    /// Version to pin. Only valid with policy `pinned`.
    #[serde(default)]
    #[schema(value_type = Option<String>, example = "agentver_01933b5a00007000800000000000001")]
    pub agent_version_id: Option<AgentVersionId>,
}

fn default_true() -> bool {
    true
}

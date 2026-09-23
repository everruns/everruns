use everruns_platform::ChannelType;
use serde::Deserialize;
use serde_json::Value;
use utoipa::ToSchema;

/// Request to create an ingress channel owned by an Agent.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateAgentChannelRequest {
    /// Transport used by the channel.
    pub channel_type: ChannelType,
    /// Transport-specific channel configuration.
    #[serde(default)]
    pub channel_config: Value,
    /// Whether the channel can accept ingress traffic.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

/// Request to update an ingress channel owned by an Agent.
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateAgentChannelRequest {
    /// Replacement transport-specific channel configuration.
    pub channel_config: Option<Value>,
    /// Whether the channel can accept ingress traffic.
    pub enabled: Option<bool>,
}

fn default_true() -> bool {
    true
}

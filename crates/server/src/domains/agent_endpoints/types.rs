use everruns_platform::ChannelType;
use serde::Deserialize;
use serde_json::Value;
use utoipa::ToSchema;

/// Request to create an ingress endpoint owned by an Agent.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateAgentEndpointRequest {
    /// Transport used by the endpoint.
    pub channel_type: ChannelType,
    /// Transport-specific endpoint configuration.
    #[serde(default)]
    pub channel_config: Value,
    /// Whether the endpoint can accept ingress traffic.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

/// Request to update an ingress endpoint owned by an Agent.
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateAgentEndpointRequest {
    /// Replacement transport-specific endpoint configuration.
    pub channel_config: Option<Value>,
    /// Whether the endpoint can accept ingress traffic.
    pub enabled: Option<bool>,
}

fn default_true() -> bool {
    true
}

use everruns_platform::ChannelType;
use serde::Deserialize;
use serde_json::Value;
use utoipa::ToSchema;

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateAgentEndpointRequest {
    pub channel_type: ChannelType,
    #[serde(default)]
    pub channel_config: Value,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateAgentEndpointRequest {
    pub channel_config: Option<Value>,
    pub enabled: Option<bool>,
}

fn default_true() -> bool {
    true
}

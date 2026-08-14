//! OpenAPI adapter for the neutral capability reference wire shape.

// The focused capability contract deliberately carries no OpenAPI dependency;
// this platform-owned surrogate must preserve the historical public schema
// descriptions byte-for-byte when ownership moves.
/// Per-agent capability configuration
///
/// Associates a capability with an agent, including optional per-agent configuration.
/// The config field allows the same capability to behave differently per-agent.
#[derive(serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
#[schema(as = AgentCapabilityConfig)]
pub struct CapabilityRefSchema {
    /// Reference to the capability ID
    #[serde(rename = "ref")]
    #[schema(value_type = String)]
    pub capability_ref: String,
    /// Per-agent configuration for this capability (capability-specific)
    #[serde(default)]
    pub config: serde_json::Value,
}

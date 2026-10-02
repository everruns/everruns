// Agent version selection shared by endpoints, triggers and the frozen App row.
// The wire name stays `AgentVersionPolicy` (EVE-1139).

use serde::{Deserialize, Serialize};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// How an exposure (endpoint or trigger; formerly the App) resolves the
/// Agent version its sessions run.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[cfg_attr(feature = "openapi", schema(example = "pinned"))]
#[serde(rename_all = "lowercase")]
pub enum AgentVersionPolicy {
    /// Resolve the agent's default_version_id at session creation/invocation time.
    #[default]
    Default,
    /// Resolve the newest saved agent_versions row for the exposure's agent.
    Latest,
    /// Use the exposure's pinned agent_version_id.
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

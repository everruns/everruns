// Portable authored agent execution configuration (EVE-877).
//
// Decision: the stored `Agent`/`AgentVersion` persistence records — lifecycle
// status, versioning and publication metadata, fork lineage, timestamps,
// usage — live in `everruns-platform`. Core keeps only this portable,
// execution-facing projection: the authored configuration the runtime folds
// into the harness → agent → session overlay chain. The platform loading seam
// (server repositories, worker adapters, hosted stores) projects stored
// records into this value and enforces lifecycle validation (archived or
// deleted records fail) before host execution begins.

use serde::{Deserialize, Serialize};

use crate::capability_types::AgentCapabilityConfig;
use crate::mcp_server::{ScopedMcpServers, scoped_mcp_servers_is_empty};
use crate::network_access::NetworkAccessList;
use crate::session_file::InitialFile;
use crate::tool_types::ToolDefinition;
use crate::typed_id::{AgentId, ModelId};

/// Portable authored execution configuration for an agent.
///
/// Carries exactly what turn execution consumes: the agent's identity for
/// correlation plus the authored configuration layer merged between the
/// harness chain and the session overlay. It is not a persistence record —
/// stored lifecycle/versioning metadata stays in `everruns-platform`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDefinition {
    /// Public agent identifier (`agent_<32-hex>`), used for correlation and
    /// session/agent mismatch validation during snapshot projection.
    pub id: AgentId,
    /// Addressable name, unique per org (e.g. "customer-support").
    pub name: String,
    /// Human-readable display name; falls back to `name` when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Human-readable description of what the agent does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// System prompt contributed by the agent layer.
    pub system_prompt: String,
    /// Default LLM model; overridable at the session layer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_model_id: Option<ModelId>,
    /// Capabilities enabled for this agent with per-agent configuration.
    #[serde(default)]
    pub capabilities: Vec<AgentCapabilityConfig>,
    /// Starter files copied into each new session for this agent.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub initial_files: Vec<InitialFile>,
    /// Network access list merged with harness and session layers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_access: Option<NetworkAccessList>,
    /// Maximum number of LLM iterations per turn for this agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_iterations: Option<usize>,
    /// Request-level parallel tool calling preference (EVE-598).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallel_tool_calls: Option<bool>,
    /// Client-side tools registered for this agent.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolDefinition>,
    /// Remote MCP servers scoped to this agent and inherited by its sessions.
    #[serde(
        default,
        rename = "mcpServers",
        alias = "mcp_servers",
        skip_serializing_if = "scoped_mcp_servers_is_empty"
    )]
    pub mcp_servers: ScopedMcpServers,
}

impl AgentDefinition {
    /// Create a definition with the given identity and prompt; all other
    /// configuration starts empty.
    pub fn new(id: AgentId, name: impl Into<String>, system_prompt: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            display_name: None,
            description: None,
            system_prompt: system_prompt.into(),
            default_model_id: None,
            capabilities: vec![],
            initial_files: vec![],
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
            tools: vec![],
            mcp_servers: ScopedMcpServers::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn definition_preserves_portable_wire_contract() {
        let mut definition = AgentDefinition::new(
            "agent_01933b5a000070008000000000000001".parse().unwrap(),
            "test",
            "You are helpful.",
        );
        let mut expected = serde_json::json!({
            "id": "agent_01933b5a000070008000000000000001", "name": "test",
            "system_prompt": "You are helpful.", "capabilities": []
        });
        // Exact shape also excludes product persistence metadata.
        assert_eq!(serde_json::to_value(&definition).unwrap(), expected);
        definition.capabilities = vec![AgentCapabilityConfig::with_config(
            "web_fetch",
            serde_json::json!({"timeout_ms": 30000}),
        )];
        definition.max_iterations = Some(7);
        definition.parallel_tool_calls = Some(false);
        definition.mcp_servers.insert(
            "docs".into(),
            crate::mcp_server::ScopedMcpServer {
                url: "https://docs.example.test/mcp".into(),
                ..Default::default()
            },
        );
        expected["capabilities"] =
            serde_json::json!([{"ref": "web_fetch", "config": {"timeout_ms": 30000}}]);
        expected["max_iterations"] = serde_json::json!(7);
        expected["parallel_tool_calls"] = serde_json::json!(false);
        expected["mcpServers"] =
            serde_json::json!({"docs": {"type": "http", "url": "https://docs.example.test/mcp"}});
        assert_eq!(serde_json::to_value(&definition).unwrap(), expected);
        let mut legacy = expected.clone();
        let servers = legacy
            .as_object_mut()
            .unwrap()
            .remove("mcpServers")
            .unwrap();
        legacy["mcp_servers"] = servers;
        for input in [expected.clone(), legacy] {
            let parsed: AgentDefinition = serde_json::from_value(input).unwrap();
            assert_eq!(serde_json::to_value(parsed).unwrap(), expected);
        }
    }
}

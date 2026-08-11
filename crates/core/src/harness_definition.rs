// Portable harness execution configuration (EVE-881).
//
// Decision: the stored `Harness` persistence record — lifecycle status,
// hierarchy identifiers, built-in flags, organization ownership, timestamps —
// lives in `everruns-platform`. Core keeps only this portable, execution-facing
// projection: the neutral environment configuration the runtime folds as the
// base layer of the harness → agent → session overlay chain. The platform
// loading seam (server repositories, worker adapters, hosted stores) resolves
// parent-chain inheritance, enforces archived/deleted validation, and projects
// stored records into this value before host execution begins — the host never
// requests or receives a stored Harness.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::capability_types::AgentCapabilityConfig;
use crate::mcp_server::{ScopedMcpServers, scoped_mcp_servers_is_empty};
use crate::network_access::NetworkAccessList;
use crate::session_file::InitialFile;
use crate::typed_id::ModelId;

/// Portable harness execution configuration.
///
/// Carries exactly what turn execution consumes: the effective
/// (inheritance-resolved) base environment configuration below the agent and
/// session overlay layers. It is not a persistence record — identity,
/// hierarchy, lifecycle, and display bookkeeping stay in `everruns-platform`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HarnessDefinition {
    /// Addressable name (e.g. "generic"). Correlation/bookkeeping only; the
    /// stored record's identity and uniqueness live at the platform layer.
    pub name: String,
    /// System prompt contributed by the harness layer. `None` contributes no
    /// base prompt; empty/whitespace values normalize to `None` during merge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    /// Default LLM model; lowest priority in the chain
    /// (controls > session > agent > harness).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_model_id: Option<ModelId>,
    /// Capabilities enabled for this harness with per-harness configuration.
    #[serde(default)]
    pub capabilities: Vec<AgentCapabilityConfig>,
    /// Starter files copied into each new session for this harness.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub initial_files: Vec<InitialFile>,
    /// Network access list merged with agent and session layers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_access: Option<NetworkAccessList>,
    /// Request-level parallel tool calling preference (EVE-598).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallel_tool_calls: Option<bool>,
    /// Remote MCP servers scoped to this harness and inherited by descendant layers.
    #[serde(
        default,
        rename = "mcpServers",
        alias = "mcp_servers",
        skip_serializing_if = "scoped_mcp_servers_is_empty"
    )]
    pub mcp_servers: ScopedMcpServers,
    /// Arbitrary key-value metadata injected into LLM requests for
    /// observability. Folded root-to-leaf across the harness chain (leaf wins)
    /// by the platform projection before this value is built.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub embedder_metadata: HashMap<String, String>,
}

impl HarnessDefinition {
    /// Create a definition with the given name and base prompt; all other
    /// configuration starts empty. Empty prompts normalize to `None`.
    pub fn new(name: impl Into<String>, system_prompt: impl Into<String>) -> Self {
        let system_prompt = system_prompt.into();
        Self {
            name: name.into(),
            system_prompt: (!system_prompt.trim().is_empty()).then_some(system_prompt),
            ..Self::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn definition_serde_round_trips() {
        let definition = HarnessDefinition::new("generic", "You are helpful.");
        let json = serde_json::to_value(&definition).unwrap();
        assert_eq!(json["name"], "generic");
        let round_tripped: HarnessDefinition = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(serde_json::to_value(&round_tripped).unwrap(), json);
    }

    #[test]
    fn new_normalizes_empty_prompt_to_none() {
        assert_eq!(HarnessDefinition::new("h", "  ").system_prompt, None);
        assert_eq!(
            HarnessDefinition::new("h", "prompt")
                .system_prompt
                .as_deref(),
            Some("prompt")
        );
    }

    #[test]
    fn definition_carries_no_persistence_metadata() {
        let definition = HarnessDefinition::new("generic", "prompt");
        let json = serde_json::to_value(&definition).unwrap();
        for persistence_field in [
            "id",
            "parent_harness_id",
            "is_built_in",
            "status",
            "created_at",
            "updated_at",
            "archived_at",
            "deleted_at",
            "display_name",
            "description",
            "tags",
        ] {
            assert!(
                json.get(persistence_field).is_none(),
                "portable definition must not expose {persistence_field}"
            );
        }
    }
}

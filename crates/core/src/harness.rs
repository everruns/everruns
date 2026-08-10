// Harness domain types
//
// Harness defines base rules and capabilities for sessions.
// Agent (optional) provides domain-specific customizations on top.
// Hierarchy: Harness → Agent → Session

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::capability_types::AgentCapabilityConfig;
use crate::mcp_server::{ScopedMcpServers, scoped_mcp_servers_is_empty};
use crate::network_access::NetworkAccessList;
use crate::session_file::InitialFile;
use crate::typed_id::{HarnessId, ModelId};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// Harness lifecycle status.
/// - `active`: Harness is available for use
/// - `archived`: Harness is hidden from listings and cannot be modified or assigned
/// - `deleted`: Harness is a tombstone kept only for historical references
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[cfg_attr(feature = "openapi", schema(example = "active"))]
#[serde(rename_all = "lowercase")]
pub enum HarnessStatus {
    /// Harness is available for use.
    Active,
    /// Harness is hidden from listings and cannot be modified or assigned.
    Archived,
    /// Harness is deleted and should only survive as a tombstone for references.
    Deleted,
}

impl std::fmt::Display for HarnessStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HarnessStatus::Active => write!(f, "active"),
            HarnessStatus::Archived => write!(f, "archived"),
            HarnessStatus::Deleted => write!(f, "deleted"),
        }
    }
}

impl From<&str> for HarnessStatus {
    fn from(s: &str) -> Self {
        match s {
            "archived" => HarnessStatus::Archived,
            "deleted" => HarnessStatus::Deleted,
            _ => HarnessStatus::Active,
        }
    }
}

/// Harness configuration for sessions.
/// A harness defines the base behavior and capabilities that apply to all sessions.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct Harness {
    /// Unique identifier for the harness (format: harness_{32-hex}).
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "harness_01933b5a00007000800000000000001"))]
    pub id: HarnessId,
    /// Name, unique per org (e.g. "generic").
    #[cfg_attr(feature = "openapi", schema(example = "generic"))]
    pub name: String,
    /// Human-readable display name shown in UI.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = "Generic Harness"))]
    pub display_name: Option<String>,
    /// Human-readable description of what the harness does.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(
        feature = "openapi",
        schema(
            example = "Default harness with file-system + secrets capabilities; safe baseline for new agents."
        )
    )]
    pub description: Option<String>,
    /// System prompt that defines the harness's base behavior.
    ///
    /// Forms the foundation of the prompt stack. Optional: when absent the
    /// harness contributes no base prompt, so the effective prompt comes
    /// entirely from the parent harness (if any), the agent, the session, and
    /// capability contributions. Empty/whitespace-only values normalize to
    /// `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(
        feature = "openapi",
        schema(
            example = "You are an Everruns agent. Be concise, cite sources when possible, and decline tasks outside your assigned scope."
        )
    )]
    pub system_prompt: Option<String>,
    /// Optional parent harness that this harness inherits from.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, example = "harness_01933b5a000070008000000000000602"))]
    pub parent_harness_id: Option<HarnessId>,
    /// Default LLM model ID for this harness.
    /// Lowest priority in chain: controls > session > agent > harness.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, example = "model_01933b5a00007000800000000000001"))]
    pub default_model_id: Option<ModelId>,
    /// Tags for organizing and filtering harnesses.
    #[serde(default)]
    #[cfg_attr(feature = "openapi", schema(example = json!(["baseline", "production"])))]
    pub tags: Vec<String>,
    /// Capabilities enabled for this harness with per-harness configuration.
    #[serde(default)]
    #[cfg_attr(
        feature = "openapi",
        schema(value_type = Vec<crate::capability_types::AgentCapabilityConfigSchema>)
    )]
    pub capabilities: Vec<AgentCapabilityConfig>,
    /// Starter files copied into each new session for this harness.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub initial_files: Vec<InitialFile>,
    /// Network access list controlling which hosts/URLs sessions can reach.
    /// Merged with agent and session layers (allowed: intersect, blocked: union).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_access: Option<NetworkAccessList>,
    /// Request-level parallel tool calling preference (EVE-598).
    ///
    /// `None` (default) preserves provider defaults. `Some(true)` signals the
    /// provider that parallel tool calls are wanted; `Some(false)` requests at
    /// most one tool call per turn and forces serial execution. Merged across
    /// harness/agent/session layers (overlay wins).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = true))]
    pub parallel_tool_calls: Option<bool>,
    /// Remote MCP servers scoped to this harness and inherited by descendant layers.
    #[serde(
        default,
        rename = "mcpServers",
        alias = "mcp_servers",
        skip_serializing_if = "scoped_mcp_servers_is_empty"
    )]
    pub mcp_servers: ScopedMcpServers,
    /// Arbitrary key-value metadata injected into LLM requests for observability.
    /// Keys from system context (session_id, org_id, etc.) always take precedence.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    #[cfg_attr(feature = "openapi", schema(example = json!({"env": "production", "team": "platform"})))]
    pub embedder_metadata: HashMap<String, String>,
    /// Whether this harness is built-in (system-managed, readonly).
    /// Built-in harnesses are provisioned during org initialization and
    /// cannot be modified or deleted via the API. Users can copy them.
    #[serde(default)]
    #[cfg_attr(feature = "openapi", schema(example = false))]
    pub is_built_in: bool,
    /// Current lifecycle status of the harness.
    pub status: HarnessStatus,
    /// Timestamp when the harness was created.
    #[cfg_attr(feature = "openapi", schema(example = "2026-04-01T10:00:00Z"))]
    pub created_at: DateTime<Utc>,
    /// Timestamp when the harness was last updated.
    #[cfg_attr(feature = "openapi", schema(example = "2026-05-20T14:00:00Z"))]
    pub updated_at: DateTime<Utc>,
    /// Timestamp when the harness was archived.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = "2026-05-26T00:00:00Z"))]
    pub archived_at: Option<DateTime<Utc>>,
    /// Timestamp when the harness was deleted.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = "2026-05-26T00:00:00Z"))]
    pub deleted_at: Option<DateTime<Utc>>,
}

/// Merge a parent harness into a child harness, producing the effective child harness.
///
/// Metadata remains child-owned (`id`, `name`, `tags`, status, timestamps). Runtime-affecting
/// fields compose via `AgentConfigOverlay::merge()` — same semantics used for the full
/// harness → agent → session fold.
pub fn merge_harness(parent: &Harness, child: &Harness) -> Harness {
    use crate::config_layer::AgentConfigOverlay;

    let effective = AgentConfigOverlay::from(parent).merge(AgentConfigOverlay::from(child));

    Harness {
        // Metadata: always child-owned
        id: child.id,
        name: child.name.clone(),
        display_name: child.display_name.clone(),
        description: child.description.clone(),
        parent_harness_id: child.parent_harness_id,
        tags: child.tags.clone(),
        is_built_in: child.is_built_in,
        status: child.status.clone(),
        created_at: child.created_at,
        updated_at: child.updated_at,
        archived_at: child.archived_at,
        deleted_at: child.deleted_at,
        // Config: from merged overlay. `None` means no base prompt at this
        // layer; `merge_system_prompts` already trims empties to `None`.
        system_prompt: effective.system_prompt,
        default_model_id: effective.default_model_id,
        capabilities: effective.capabilities,
        initial_files: effective.initial_files,
        network_access: effective.network_access,
        parallel_tool_calls: effective.parallel_tool_calls,
        mcp_servers: effective.mcp_servers,
        // embedder_metadata: parent base, child keys win
        embedder_metadata: {
            let mut m = parent.embedder_metadata.clone();
            m.extend(
                child
                    .embedder_metadata
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone())),
            );
            m
        },
    }
}

/// Merge a root-to-leaf chain of harnesses into one effective harness.
pub fn merge_harness_chain(chain: &[Harness]) -> Option<Harness> {
    let mut iter = chain.iter();
    let first = iter.next()?.clone();
    Some(iter.fold(first, |effective, layer| merge_harness(&effective, layer)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_harness(id_seed: u128, system_prompt: &str) -> Harness {
        Harness {
            id: HarnessId::from_uuid(uuid::Uuid::from_u128(id_seed)),
            name: format!("harness-{id_seed}"),
            display_name: Some(format!("Harness {id_seed}")),
            description: None,
            system_prompt: Some(system_prompt.to_string()),
            parent_harness_id: None,
            default_model_id: None,
            tags: vec![],
            capabilities: vec![],
            initial_files: vec![],
            network_access: None,
            parallel_tool_calls: None,
            mcp_servers: ScopedMcpServers::default(),
            embedder_metadata: HashMap::new(),
            is_built_in: false,
            status: HarnessStatus::Active,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            archived_at: None,
            deleted_at: None,
        }
    }

    #[test]
    fn merges_prompt_capabilities_and_initial_files() {
        let mut parent = test_harness(1, "Parent prompt.");
        parent.capabilities = vec![
            AgentCapabilityConfig::new("session_file_system"),
            AgentCapabilityConfig::with_config(
                "web_fetch",
                serde_json::json!({"enable_file_download": true}),
            ),
        ];
        parent.initial_files = vec![InitialFile {
            path: "/workspace/README.md".to_string(),
            content: "parent".to_string(),
            encoding: "text".to_string(),
            is_readonly: true,
        }];

        let mut child = test_harness(2, "Child prompt.");
        child.parent_harness_id = Some(parent.id);
        child.capabilities = vec![
            AgentCapabilityConfig::with_config(
                "web_fetch",
                serde_json::json!({"enable_file_download": false}),
            ),
            AgentCapabilityConfig::new("platform_management"),
        ];
        child.initial_files = vec![
            InitialFile {
                path: "README.md".to_string(),
                content: "child".to_string(),
                encoding: "text".to_string(),
                is_readonly: false,
            },
            InitialFile {
                path: "/notes.txt".to_string(),
                content: "notes".to_string(),
                encoding: "text".to_string(),
                is_readonly: false,
            },
        ];

        let merged = merge_harness(&parent, &child);

        assert_eq!(
            merged.system_prompt.as_deref(),
            Some("Parent prompt.\n\nChild prompt.")
        );
        assert_eq!(merged.capabilities.len(), 3);
        assert_eq!(
            merged.capabilities[1],
            AgentCapabilityConfig::with_config(
                "web_fetch",
                serde_json::json!({"enable_file_download": false}),
            )
        );
        assert_eq!(merged.initial_files.len(), 2);
        assert_eq!(merged.initial_files[0].content, "child");
        assert_eq!(merged.initial_files[1].path, "/notes.txt");
    }

    #[test]
    fn merges_embedder_metadata_parent_first_child_wins() {
        let mut parent = test_harness(1, "Parent.");
        parent.embedder_metadata = HashMap::from([
            ("env".to_string(), "production".to_string()),
            ("team".to_string(), "platform".to_string()),
        ]);

        let mut child = test_harness(2, "Child.");
        child.parent_harness_id = Some(parent.id);
        child.embedder_metadata = HashMap::from([
            ("team".to_string(), "ai".to_string()), // overrides parent
            ("experiment".to_string(), "v2".to_string()), // child-only key
        ]);

        let merged = merge_harness(&parent, &child);

        assert_eq!(
            merged.embedder_metadata.get("env").map(String::as_str),
            Some("production")
        );
        assert_eq!(
            merged.embedder_metadata.get("team").map(String::as_str),
            Some("ai"),
            "child wins on collision"
        );
        assert_eq!(
            merged
                .embedder_metadata
                .get("experiment")
                .map(String::as_str),
            Some("v2")
        );
    }

    #[test]
    fn merges_chain_root_to_leaf() {
        let root = test_harness(1, "Root");
        let mut middle = test_harness(2, "Middle");
        middle.parent_harness_id = Some(root.id);
        let mut leaf = test_harness(3, "Leaf");
        leaf.parent_harness_id = Some(middle.id);

        let merged = merge_harness_chain(&[root, middle, leaf]).expect("merged harness");
        assert_eq!(
            merged.system_prompt.as_deref(),
            Some("Root\n\nMiddle\n\nLeaf")
        );
    }

    #[test]
    fn merges_optional_prompts_skipping_empty_layers() {
        // A child with no base prompt inherits the parent's prompt verbatim.
        let parent = test_harness(1, "Parent prompt.");
        let mut child = test_harness(2, "");
        child.system_prompt = None;
        child.parent_harness_id = Some(parent.id);

        let merged = merge_harness(&parent, &child);
        assert_eq!(merged.system_prompt.as_deref(), Some("Parent prompt."));

        // Two promptless layers compose to no base prompt at all.
        let mut root = test_harness(1, "");
        root.system_prompt = None;
        let mut leaf = test_harness(2, "");
        leaf.system_prompt = None;
        leaf.parent_harness_id = Some(root.id);

        let merged = merge_harness(&root, &leaf);
        assert_eq!(merged.system_prompt, None);
    }
}

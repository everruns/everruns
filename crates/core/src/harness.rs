// Harness domain types
//
// Harness defines base rules and capabilities for sessions.
// Agent (optional) provides domain-specific customizations on top.
// Hierarchy: Harness → Agent → Session

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::capability_types::AgentCapabilityConfig;
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
    pub name: String,
    /// Human-readable display name shown in UI.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Human-readable description of what the harness does.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// System prompt that defines the harness's base behavior.
    /// Forms the foundation of the prompt stack.
    pub system_prompt: String,
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
    pub tags: Vec<String>,
    /// Capabilities enabled for this harness with per-harness configuration.
    #[serde(default)]
    pub capabilities: Vec<AgentCapabilityConfig>,
    /// Starter files copied into each new session for this harness.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub initial_files: Vec<InitialFile>,
    /// Network access list controlling which hosts/URLs sessions can reach.
    /// Merged with agent and session layers (allowed: intersect, blocked: union).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_access: Option<NetworkAccessList>,
    /// Whether this harness is built-in (system-managed, readonly).
    /// Built-in harnesses are provisioned during org initialization and
    /// cannot be modified or deleted via the API. Users can copy them.
    #[serde(default)]
    pub is_built_in: bool,
    /// Current lifecycle status of the harness.
    pub status: HarnessStatus,
    /// Timestamp when the harness was created.
    pub created_at: DateTime<Utc>,
    /// Timestamp when the harness was last updated.
    pub updated_at: DateTime<Utc>,
    /// Timestamp when the harness was archived.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<DateTime<Utc>>,
    /// Timestamp when the harness was deleted.
    #[serde(skip_serializing_if = "Option::is_none")]
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
        // Config: from merged overlay
        system_prompt: effective.system_prompt.unwrap_or_default(),
        default_model_id: effective.default_model_id,
        capabilities: effective.capabilities,
        initial_files: effective.initial_files,
        network_access: effective.network_access,
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
            system_prompt: system_prompt.to_string(),
            parent_harness_id: None,
            default_model_id: None,
            tags: vec![],
            capabilities: vec![],
            initial_files: vec![],
            network_access: None,
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

        assert_eq!(merged.system_prompt, "Parent prompt.\n\nChild prompt.");
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
    fn merges_chain_root_to_leaf() {
        let root = test_harness(1, "Root");
        let mut middle = test_harness(2, "Middle");
        middle.parent_harness_id = Some(root.id);
        let mut leaf = test_harness(3, "Leaf");
        leaf.parent_harness_id = Some(middle.id);

        let merged = merge_harness_chain(&[root, middle, leaf]).expect("merged harness");
        assert_eq!(merged.system_prompt, "Root\n\nMiddle\n\nLeaf");
    }
}

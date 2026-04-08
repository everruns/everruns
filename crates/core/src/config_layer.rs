// Composable configuration overlay for Harness → Agent → Session merging.
//
// See specs/concepts.md#AgentConfigOverlay for design rationale and diagrams.
//
// Harness, Agent, and Session share additive fields (system_prompt, capabilities,
// initial_files, network_access, tools, max_iterations, default_model_id).
// AgentConfigOverlay extracts this shared shape so merge semantics are defined
// once and tested in one place.
//
// Each entity produces an AgentConfigOverlay via From<&T>. Harness inheritance
// chains produce one overlay per harness. All overlays fold bottom-up into a
// single effective config that RuntimeAgentBuilder::from_overlay() resolves
// into a RuntimeAgent.
//
// Merge semantics per field:
// - system_prompt: base first, overlay appended (concatenate non-empty parts)
// - capabilities: overlay overrides base by capability ID (last wins)
// - initial_files: overlay overrides base by normalized path (last wins)
// - network_access: allowed intersects, blocked unions (can only narrow)
// - default_model_id: overlay wins if set, else inherit base
// - tools: additive (overlay appended after base, deduplicated at build time)
// - max_iterations: overlay wins if set, else inherit base

use crate::agent::Agent;
use crate::capability_types::AgentCapabilityConfig;
use crate::harness::Harness;
use crate::network_access::{self, NetworkAccessList};
use crate::session::Session;
use crate::session_file::InitialFile;
use crate::tool_types::ToolDefinition;
use crate::typed_id::ModelId;

/// A composable configuration layer.
///
/// Produced by Harness, Agent, or Session via `From<&T>`. Layers merge
/// bottom-up into a single effective config for RuntimeAgent building.
#[derive(Debug, Clone, Default)]
pub struct AgentConfigOverlay {
    /// System prompt fragment for this layer.
    pub system_prompt: Option<String>,
    /// Capabilities with per-layer configuration.
    pub capabilities: Vec<AgentCapabilityConfig>,
    /// Starter files for the session filesystem.
    pub initial_files: Vec<InitialFile>,
    /// Network access restrictions.
    pub network_access: Option<NetworkAccessList>,
    /// Default model ID for this layer.
    pub default_model_id: Option<ModelId>,
    /// Tool definitions (client-side or capability-provided).
    pub tools: Vec<ToolDefinition>,
    /// Max iterations per turn.
    pub max_iterations: Option<usize>,
}

impl AgentConfigOverlay {
    /// Merge an overlay on top of this base layer, producing a new effective layer.
    ///
    /// This is the core composition operation. Each field follows its own merge
    /// semantic (see module-level docs). The result represents the combined
    /// configuration of both layers.
    pub fn merge(self, overlay: AgentConfigOverlay) -> AgentConfigOverlay {
        let system_prompt = merge_system_prompts(self.system_prompt, overlay.system_prompt);
        let capabilities = merge_capabilities(&self.capabilities, &overlay.capabilities);
        let initial_files = merge_initial_files(&self.initial_files, &overlay.initial_files);
        let network_access = network_access::merge_network_access(
            self.network_access.as_ref(),
            overlay.network_access.as_ref(),
        );
        let default_model_id = overlay.default_model_id.or(self.default_model_id);
        let max_iterations = overlay.max_iterations.or(self.max_iterations);

        let mut tools = self.tools;
        tools.extend(overlay.tools);

        AgentConfigOverlay {
            system_prompt,
            capabilities,
            initial_files,
            network_access,
            default_model_id,
            tools,
            max_iterations,
        }
    }

    /// Fold multiple layers bottom-up into a single effective layer.
    ///
    /// Layers are applied in order: the first layer is the base, each subsequent
    /// layer is merged on top.
    pub fn fold(layers: impl IntoIterator<Item = AgentConfigOverlay>) -> AgentConfigOverlay {
        layers
            .into_iter()
            .fold(AgentConfigOverlay::default(), |acc, layer| acc.merge(layer))
    }
}

// ---------------------------------------------------------------------------
// Merge helpers (shared by harness inheritance and config layer merging)
// ---------------------------------------------------------------------------

/// Merge two optional system prompt fragments. Base first, overlay appended.
fn merge_system_prompts(base: Option<String>, overlay: Option<String>) -> Option<String> {
    let base = base.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let overlay = overlay
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    match (base, overlay) {
        (None, None) => None,
        (Some(b), None) => Some(b),
        (None, Some(o)) => Some(o),
        (Some(b), Some(o)) => Some(format!("{b}\n\n{o}")),
    }
}

/// Merge capabilities: overlay overrides base by capability ID (last wins).
pub fn merge_capabilities(
    base: &[AgentCapabilityConfig],
    overlay: &[AgentCapabilityConfig],
) -> Vec<AgentCapabilityConfig> {
    let mut merged = base.to_vec();

    for overlay_cap in overlay {
        if let Some(existing) = merged
            .iter_mut()
            .find(|existing| existing.capability_id() == overlay_cap.capability_id())
        {
            *existing = overlay_cap.clone();
        } else {
            merged.push(overlay_cap.clone());
        }
    }

    merged
}

/// Merge initial files: overlay overrides base by normalized path (last wins).
pub fn merge_initial_files(base: &[InitialFile], overlay: &[InitialFile]) -> Vec<InitialFile> {
    let mut merged = base.to_vec();

    for overlay_file in overlay {
        let normalized_path = normalize_initial_file_path(&overlay_file.path);
        if let Some(existing) = merged
            .iter_mut()
            .find(|existing| normalize_initial_file_path(&existing.path) == normalized_path)
        {
            *existing = overlay_file.clone();
        } else {
            merged.push(overlay_file.clone());
        }
    }

    merged
}

/// Normalize an initial file path to a canonical form for comparison.
///
/// Strips `/workspace/` prefix, ensures leading `/`.
pub fn normalize_initial_file_path(path: &str) -> String {
    if path == "/workspace" {
        "/".to_string()
    } else if let Some(stripped) = path.strip_prefix("/workspace/") {
        format!("/{}", stripped.trim_start_matches('/'))
    } else if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{}", path)
    }
}

// ---------------------------------------------------------------------------
// From conversions
// ---------------------------------------------------------------------------

impl From<&Harness> for AgentConfigOverlay {
    fn from(h: &Harness) -> Self {
        AgentConfigOverlay {
            system_prompt: Some(h.system_prompt.clone()),
            capabilities: h.capabilities.clone(),
            initial_files: h.initial_files.clone(),
            network_access: h.network_access.clone(),
            default_model_id: h.default_model_id,
            tools: vec![],
            max_iterations: None,
        }
    }
}

impl From<&Agent> for AgentConfigOverlay {
    fn from(a: &Agent) -> Self {
        AgentConfigOverlay {
            system_prompt: Some(a.system_prompt.clone()),
            capabilities: a.capabilities.clone(),
            initial_files: a.initial_files.clone(),
            network_access: a.network_access.clone(),
            default_model_id: a.default_model_id,
            tools: a.tools.clone(),
            max_iterations: a.max_iterations,
        }
    }
}

impl From<&Session> for AgentConfigOverlay {
    fn from(s: &Session) -> Self {
        AgentConfigOverlay {
            system_prompt: s.system_prompt.clone(),
            capabilities: s.capabilities.clone(),
            initial_files: s.initial_files.clone(),
            network_access: s.network_access.clone(),
            default_model_id: s.model_id,
            tools: s.tools.clone(),
            max_iterations: s.max_iterations,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability_types::AgentCapabilityConfig;
    use crate::network_access::NetworkAccessList;
    use crate::session_file::InitialFile;

    fn make_file(path: &str, content: &str) -> InitialFile {
        InitialFile {
            path: path.to_string(),
            content: content.to_string(),
            encoding: "text".to_string(),
            is_readonly: false,
        }
    }

    #[test]
    fn merge_system_prompts_concatenates() {
        let base = AgentConfigOverlay {
            system_prompt: Some("Base prompt.".into()),
            ..Default::default()
        };
        let overlay = AgentConfigOverlay {
            system_prompt: Some("Overlay prompt.".into()),
            ..Default::default()
        };
        let merged = base.merge(overlay);
        assert_eq!(
            merged.system_prompt.as_deref(),
            Some("Base prompt.\n\nOverlay prompt.")
        );
    }

    #[test]
    fn merge_system_prompts_base_only() {
        let base = AgentConfigOverlay {
            system_prompt: Some("Base.".into()),
            ..Default::default()
        };
        let overlay = AgentConfigOverlay::default();
        let merged = base.merge(overlay);
        assert_eq!(merged.system_prompt.as_deref(), Some("Base."));
    }

    #[test]
    fn merge_system_prompts_overlay_only() {
        let base = AgentConfigOverlay::default();
        let overlay = AgentConfigOverlay {
            system_prompt: Some("Overlay.".into()),
            ..Default::default()
        };
        let merged = base.merge(overlay);
        assert_eq!(merged.system_prompt.as_deref(), Some("Overlay."));
    }

    #[test]
    fn merge_system_prompts_both_empty() {
        let merged = AgentConfigOverlay::default().merge(AgentConfigOverlay::default());
        assert!(merged.system_prompt.is_none());
    }

    #[test]
    fn merge_capabilities_override_by_id() {
        let base = AgentConfigOverlay {
            capabilities: vec![
                AgentCapabilityConfig::new("session_file_system"),
                AgentCapabilityConfig::with_config(
                    "web_fetch",
                    serde_json::json!({"enable_file_download": true}),
                ),
            ],
            ..Default::default()
        };
        let overlay = AgentConfigOverlay {
            capabilities: vec![
                AgentCapabilityConfig::with_config(
                    "web_fetch",
                    serde_json::json!({"enable_file_download": false}),
                ),
                AgentCapabilityConfig::new("current_time"),
            ],
            ..Default::default()
        };
        let merged = base.merge(overlay);

        assert_eq!(merged.capabilities.len(), 3);
        assert_eq!(
            merged.capabilities[0].capability_id(),
            "session_file_system"
        );
        assert_eq!(merged.capabilities[1].capability_id(), "web_fetch");
        assert_eq!(
            merged.capabilities[1],
            AgentCapabilityConfig::with_config(
                "web_fetch",
                serde_json::json!({"enable_file_download": false})
            )
        );
        assert_eq!(merged.capabilities[2].capability_id(), "current_time");
    }

    #[test]
    fn merge_initial_files_override_by_path() {
        let base = AgentConfigOverlay {
            initial_files: vec![
                make_file("/workspace/README.md", "parent"),
                make_file("/workspace/config.txt", "parent-config"),
            ],
            ..Default::default()
        };
        let overlay = AgentConfigOverlay {
            initial_files: vec![
                make_file("README.md", "child"),
                make_file("/notes.txt", "notes"),
            ],
            ..Default::default()
        };
        let merged = base.merge(overlay);

        assert_eq!(merged.initial_files.len(), 3);
        assert_eq!(merged.initial_files[0].content, "child"); // overridden
        assert_eq!(merged.initial_files[1].content, "parent-config"); // kept
        assert_eq!(merged.initial_files[2].content, "notes"); // added
    }

    #[test]
    fn merge_network_access_narrows() {
        let base = AgentConfigOverlay {
            network_access: Some(NetworkAccessList::allow_only([
                "*.example.com",
                "*.github.com",
            ])),
            ..Default::default()
        };
        let overlay = AgentConfigOverlay {
            network_access: Some(NetworkAccessList::allow_only(["api.example.com"])),
            ..Default::default()
        };
        let merged = base.merge(overlay);

        let na = merged.network_access.unwrap();
        assert_eq!(na.allowed, vec!["api.example.com".to_string()]);
    }

    #[test]
    fn merge_model_overlay_wins() {
        let base = AgentConfigOverlay {
            default_model_id: Some(ModelId::from_uuid(uuid::Uuid::from_u128(1))),
            ..Default::default()
        };
        let overlay = AgentConfigOverlay {
            default_model_id: Some(ModelId::from_uuid(uuid::Uuid::from_u128(2))),
            ..Default::default()
        };
        let merged = base.merge(overlay);
        assert_eq!(
            merged.default_model_id,
            Some(ModelId::from_uuid(uuid::Uuid::from_u128(2)))
        );
    }

    #[test]
    fn merge_model_inherits_base() {
        let base = AgentConfigOverlay {
            default_model_id: Some(ModelId::from_uuid(uuid::Uuid::from_u128(1))),
            ..Default::default()
        };
        let overlay = AgentConfigOverlay::default();
        let merged = base.merge(overlay);
        assert_eq!(
            merged.default_model_id,
            Some(ModelId::from_uuid(uuid::Uuid::from_u128(1)))
        );
    }

    #[test]
    fn merge_max_iterations_overlay_wins() {
        let base = AgentConfigOverlay {
            max_iterations: Some(100),
            ..Default::default()
        };
        let overlay = AgentConfigOverlay {
            max_iterations: Some(50),
            ..Default::default()
        };
        let merged = base.merge(overlay);
        assert_eq!(merged.max_iterations, Some(50));
    }

    #[test]
    fn merge_max_iterations_inherits_base() {
        let base = AgentConfigOverlay {
            max_iterations: Some(100),
            ..Default::default()
        };
        let overlay = AgentConfigOverlay::default();
        let merged = base.merge(overlay);
        assert_eq!(merged.max_iterations, Some(100));
    }

    #[test]
    fn merge_tools_additive() {
        use crate::tool_types::{BuiltinTool, ToolDefinition, ToolPolicy};

        let make_tool = |name: &str| {
            ToolDefinition::Builtin(BuiltinTool {
                name: name.to_string(),
                display_name: None,
                description: format!("{name} tool"),
                parameters: serde_json::json!({}),
                policy: ToolPolicy::Auto,
                category: None,
                deferrable: Default::default(),
                hints: crate::tool_types::ToolHints::default(),
            })
        };

        let base = AgentConfigOverlay {
            tools: vec![make_tool("tool_a")],
            ..Default::default()
        };
        let overlay = AgentConfigOverlay {
            tools: vec![make_tool("tool_b")],
            ..Default::default()
        };
        let merged = base.merge(overlay);
        assert_eq!(merged.tools.len(), 2);
        assert_eq!(merged.tools[0].name(), "tool_a");
        assert_eq!(merged.tools[1].name(), "tool_b");
    }

    #[test]
    fn fold_three_layers() {
        let harness = AgentConfigOverlay {
            system_prompt: Some("Harness prompt.".into()),
            capabilities: vec![AgentCapabilityConfig::new("session_file_system")],
            initial_files: vec![make_file("/config.txt", "harness")],
            max_iterations: None,
            ..Default::default()
        };
        let agent = AgentConfigOverlay {
            system_prompt: Some("Agent prompt.".into()),
            capabilities: vec![AgentCapabilityConfig::new("current_time")],
            initial_files: vec![make_file("/config.txt", "agent")],
            max_iterations: Some(200),
            ..Default::default()
        };
        let session = AgentConfigOverlay {
            system_prompt: Some("Session prompt.".into()),
            max_iterations: Some(50),
            ..Default::default()
        };

        let effective = AgentConfigOverlay::fold([harness, agent, session]);

        assert_eq!(
            effective.system_prompt.as_deref(),
            Some("Harness prompt.\n\nAgent prompt.\n\nSession prompt.")
        );
        assert_eq!(effective.capabilities.len(), 2);
        assert_eq!(effective.initial_files.len(), 1);
        assert_eq!(effective.initial_files[0].content, "agent");
        assert_eq!(effective.max_iterations, Some(50));
    }

    #[test]
    fn normalize_workspace_prefix() {
        assert_eq!(
            normalize_initial_file_path("/workspace/README.md"),
            "/README.md"
        );
        assert_eq!(normalize_initial_file_path("/workspace"), "/");
        assert_eq!(normalize_initial_file_path("README.md"), "/README.md");
        assert_eq!(normalize_initial_file_path("/README.md"), "/README.md");
    }
}

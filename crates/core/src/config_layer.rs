// Composable configuration overlay for Harness → Agent → Session merging.
//
// See knowledge/foundations/concepts.md#AgentConfigOverlay for design rationale and diagrams.
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
// - parallel_tool_calls: overlay wins if set, else inherit base
// - mcp_servers: overlay overrides base by logical server name (last wins)

use crate::agent_definition::AgentDefinition;
use crate::capability_types::AgentCapabilityConfig;
use crate::harness_definition::HarnessDefinition;
use crate::mcp_server::{ScopedMcpServers, merge_scoped_mcp_servers};
use crate::network_access::{self, NetworkAccessList};
use crate::session::ExecutionSession;
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
    /// Request-level parallel tool calling preference (EVE-598).
    pub parallel_tool_calls: Option<bool>,
    /// Remote MCP servers scoped to this layer.
    pub mcp_servers: ScopedMcpServers,
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
        let parallel_tool_calls = overlay.parallel_tool_calls.or(self.parallel_tool_calls);
        let mcp_servers = merge_scoped_mcp_servers(&self.mcp_servers, &overlay.mcp_servers);

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
            parallel_tool_calls,
            mcp_servers,
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

impl From<&HarnessDefinition> for AgentConfigOverlay {
    fn from(h: &HarnessDefinition) -> Self {
        AgentConfigOverlay {
            system_prompt: h.system_prompt.clone(),
            capabilities: h.capabilities.clone(),
            initial_files: h.initial_files.clone(),
            network_access: h.network_access.clone(),
            default_model_id: h.default_model_id,
            tools: vec![],
            max_iterations: None,
            parallel_tool_calls: h.parallel_tool_calls,
            mcp_servers: h.mcp_servers.clone(),
        }
    }
}

impl From<&AgentDefinition> for AgentConfigOverlay {
    fn from(a: &AgentDefinition) -> Self {
        AgentConfigOverlay {
            system_prompt: Some(a.system_prompt.clone()),
            capabilities: a.capabilities.clone(),
            initial_files: a.initial_files.clone(),
            network_access: a.network_access.clone(),
            default_model_id: a.default_model_id,
            tools: a.tools.clone(),
            max_iterations: a.max_iterations,
            parallel_tool_calls: a.parallel_tool_calls,
            mcp_servers: a.mcp_servers.clone(),
        }
    }
}

impl From<&ExecutionSession> for AgentConfigOverlay {
    fn from(s: &ExecutionSession) -> Self {
        let goal_prompt = s
            .goal
            .as_ref()
            .map(|goal| goal.trim())
            .filter(|goal| !goal.is_empty())
            .map(|goal| format!("<session-goal>\n{goal}\n</session-goal>"));
        AgentConfigOverlay {
            system_prompt: merge_system_prompts(s.system_prompt.clone(), goal_prompt),
            capabilities: s.capabilities.clone(),
            initial_files: s.initial_files.clone(),
            network_access: s.network_access.clone(),
            default_model_id: s.model_id,
            tools: s.tools.clone(),
            max_iterations: s.max_iterations,
            parallel_tool_calls: s.parallel_tool_calls,
            mcp_servers: s.mcp_servers.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp_server::{McpServerAuthMode, ScopedMcpServer};
    use crate::tool_types::{BuiltinTool, ToolHints, ToolPolicy};
    use serde_json::json;

    fn file(path: &str, content: &str, readonly: bool) -> InitialFile {
        InitialFile {
            path: path.into(),
            content: content.into(),
            encoding: "text".into(),
            is_readonly: readonly,
        }
    }

    fn tool(name: &str, description: &str) -> ToolDefinition {
        ToolDefinition::Builtin(BuiltinTool {
            name: name.into(),
            description: description.into(),
            display_name: Some("Display".into()),
            parameters: json!({"type":"object","properties":{"path":{"type":"string"}}}),
            policy: ToolPolicy::Auto,
            category: None,
            deferrable: Default::default(),
            hints: ToolHints::default(),
            full_parameters: None,
        })
    }

    fn server(url: &str, marker: &str) -> ScopedMcpServer {
        ScopedMcpServer {
            url: url.into(),
            headers: [("X-Test".into(), marker.into())].into(),
            auth_mode: McpServerAuthMode::OAuth,
            oauth_provider_id: Some(marker.into()),
            tool_discovery: false,
            ..Default::default()
        }
    }

    fn sample_overlay() -> AgentConfigOverlay {
        AgentConfigOverlay {
            system_prompt: Some("Base prompt.".into()),
            capabilities: vec![AgentCapabilityConfig::with_config(
                "web_fetch",
                json!({"download":false}),
            )],
            initial_files: vec![file("/config.txt", "config", true)],
            network_access: Some(NetworkAccessList::allow_only(["api.example.com"])),
            default_model_id: Some(ModelId::from_uuid(uuid::Uuid::from_u128(7))),
            tools: vec![tool("inspect", "Inspect the workspace")],
            max_iterations: Some(17),
            parallel_tool_calls: Some(false),
            mcp_servers: [(
                "docs".into(),
                server("https://docs.example.com/mcp", "docs-provider"),
            )]
            .into(),
        }
    }

    fn assert_overlay(actual: AgentConfigOverlay, expected: AgentConfigOverlay) {
        assert_eq!(actual.system_prompt, expected.system_prompt);
        assert_eq!(actual.capabilities, expected.capabilities);
        assert_eq!(actual.initial_files, expected.initial_files);
        assert_eq!(actual.network_access, expected.network_access);
        assert_eq!(actual.default_model_id, expected.default_model_id);
        assert_eq!(
            serde_json::to_value(actual.tools).unwrap(),
            serde_json::to_value(expected.tools).unwrap()
        );
        assert_eq!(actual.max_iterations, expected.max_iterations);
        assert_eq!(actual.parallel_tool_calls, expected.parallel_tool_calls);
        assert_eq!(actual.mcp_servers, expected.mcp_servers);
    }

    #[test]
    fn merge_prompts_trims_omits_empty_and_preserves_layer_order() {
        for (base, overlay, expected) in [
            (None, None, None),
            (Some(" \n"), Some("\t"), None),
            (Some(" Base. "), None, Some("Base.")),
            (None, Some(" Overlay.\n"), Some("Overlay.")),
            (Some("Base."), Some(" \t"), Some("Base.")),
            (Some(" \t"), Some("Overlay."), Some("Overlay.")),
            (
                Some(" Base. "),
                Some(" Overlay. "),
                Some("Base.\n\nOverlay."),
            ),
        ] {
            let result = AgentConfigOverlay {
                system_prompt: base.map(str::to_owned),
                ..Default::default()
            }
            .merge(AgentConfigOverlay {
                system_prompt: overlay.map(str::to_owned),
                ..Default::default()
            });
            assert_eq!(
                result.system_prompt.as_deref(),
                expected,
                "{base:?} + {overlay:?}"
            );
        }
    }

    #[test]
    fn merge_capabilities_replaces_full_config_and_preserves_unrelated_order() {
        let retained =
            AgentCapabilityConfig::with_config("session_file_system", json!({"readonly":true}));
        let old = AgentCapabilityConfig::with_config("web_fetch", json!({"old":true}));
        let replacement =
            AgentCapabilityConfig::with_config("web_fetch", json!({"download":false}));
        let added = AgentCapabilityConfig::with_config("current_time", json!({"zone":"UTC"}));
        let result = AgentConfigOverlay {
            capabilities: vec![retained.clone(), old],
            ..Default::default()
        }
        .merge(AgentConfigOverlay {
            capabilities: vec![
                AgentCapabilityConfig::new("web_fetch"),
                added.clone(),
                replacement.clone(),
            ],
            ..Default::default()
        });
        assert_eq!(result.capabilities, vec![retained, replacement, added]);
    }

    #[test]
    fn merge_initial_files_replaces_full_file_by_normalized_path() {
        let retained = file("/workspace/config.txt", "parent-config", true);
        let replacement = InitialFile {
            path: "README.md".into(),
            content: "Y2hpbGQ=".into(),
            encoding: "base64".into(),
            is_readonly: true,
        };
        let added = file("/notes.txt", "notes", false);
        let result = AgentConfigOverlay {
            initial_files: vec![
                file("/workspace/README.md", "parent", false),
                retained.clone(),
            ],
            ..Default::default()
        }
        .merge(AgentConfigOverlay {
            initial_files: vec![replacement.clone(), added.clone()],
            ..Default::default()
        });
        assert_eq!(result.initial_files, vec![replacement, retained, added]);
    }

    #[test]
    fn merge_network_access_keeps_intersection_and_both_block_lists() {
        let result = AgentConfigOverlay {
            network_access: Some(NetworkAccessList {
                allowed: vec!["*.example.com".into(), "*.github.com".into()],
                blocked: vec!["private.example.com".into()],
            }),
            ..Default::default()
        }
        .merge(AgentConfigOverlay {
            network_access: Some(NetworkAccessList {
                allowed: vec![
                    "api.example.com".into(),
                    "private.example.com".into(),
                    "child.example.com".into(),
                    "outside.net".into(),
                ],
                blocked: vec!["child.example.com".into(), "private.example.com".into()],
            }),
            ..Default::default()
        });
        let policy = result.network_access.unwrap();
        assert_eq!(
            policy,
            NetworkAccessList {
                allowed: vec![
                    "api.example.com".into(),
                    "private.example.com".into(),
                    "child.example.com".into()
                ],
                blocked: vec!["private.example.com".into(), "child.example.com".into()],
            }
        );
        assert!(policy.is_url_allowed("https://api.example.com/data"));
        for url in [
            "https://private.example.com",
            "https://child.example.com",
            "https://outside.net",
            "https://github.com",
        ] {
            assert!(!policy.is_url_allowed(url), "{url}");
        }
    }

    #[test]
    fn merge_scalar_options_inherits_missing_and_keeps_explicit_zero_or_false() {
        // Each tuple is base, overlay, expected. Distinct values expose reversed precedence.
        for (base, overlay, expected, base_parallel, overlay_parallel, expected_parallel) in [
            (None, None, None, None, None, None),
            (Some(7), None, Some(7), Some(true), None, Some(true)),
            (None, Some(9), Some(9), None, Some(false), Some(false)),
            (
                Some(7),
                Some(0),
                Some(0),
                Some(true),
                Some(false),
                Some(false),
            ),
            (Some(7), None, Some(7), Some(false), None, Some(false)),
        ] {
            let model = |n: u128| ModelId::from_uuid(uuid::Uuid::from_u128(n));
            let result = AgentConfigOverlay {
                default_model_id: base.map(model),
                max_iterations: base.map(|n| n as usize),
                parallel_tool_calls: base_parallel,
                ..Default::default()
            }
            .merge(AgentConfigOverlay {
                default_model_id: overlay.map(model),
                max_iterations: overlay.map(|n| n as usize),
                parallel_tool_calls: overlay_parallel,
                ..Default::default()
            });
            assert_eq!(result.default_model_id, expected.map(model));
            assert_eq!(result.max_iterations, expected.map(|n| n as usize));
            assert_eq!(result.parallel_tool_calls, expected_parallel);
        }
    }

    #[test]
    fn merge_tools_preserves_full_definitions_and_defers_deduplication() {
        let base = tool("inspect", "base schema owner");
        let overlay = tool("inspect", "overlay schema owner");
        let added = tool("search", "new tool");
        let expected =
            serde_json::to_value([base.clone(), overlay.clone(), added.clone()]).unwrap();
        let result = AgentConfigOverlay {
            tools: vec![base],
            ..Default::default()
        }
        .merge(AgentConfigOverlay {
            tools: vec![overlay, added],
            ..Default::default()
        });
        assert_eq!(serde_json::to_value(result.tools).unwrap(), expected);
    }

    #[test]
    fn merge_mcp_servers_replaces_credentials_and_keeps_unrelated_entries() {
        let retained = server("https://retained.example.com/mcp", "retained");
        let replacement = server("https://overlay.example.com/mcp", "new-provider");
        let added = server("https://search.example.com/mcp", "search-provider");
        let result = AgentConfigOverlay {
            mcp_servers: [
                (
                    "docs".into(),
                    server("https://base.example.com/mcp", "old-provider"),
                ),
                ("retained".into(), retained.clone()),
            ]
            .into(),
            ..Default::default()
        }
        .merge(AgentConfigOverlay {
            mcp_servers: [
                ("docs".into(), replacement.clone()),
                ("search".into(), added.clone()),
            ]
            .into(),
            ..Default::default()
        });
        assert_eq!(
            result.mcp_servers,
            [
                ("docs".into(), replacement),
                ("search".into(), added),
                ("retained".into(), retained)
            ]
            .into()
        );
    }

    #[test]
    fn fold_three_layers_preserves_every_overlay_field() {
        let harness = sample_overlay();
        let mut expected = sample_overlay();
        expected.system_prompt = Some("Base prompt.\n\nAgent prompt.\n\nSession prompt.".into());
        let extra_capability =
            AgentCapabilityConfig::with_config("current_time", json!({"zone":"UTC"}));
        expected.capabilities.push(extra_capability.clone());
        expected.initial_files = vec![file("config.txt", "agent", false)];
        expected.max_iterations = Some(50);
        let agent = AgentConfigOverlay {
            system_prompt: Some("Agent prompt.".into()),
            capabilities: vec![extra_capability],
            initial_files: vec![file("config.txt", "agent", false)],
            max_iterations: Some(200),
            ..Default::default()
        };
        let session = AgentConfigOverlay {
            system_prompt: Some("Session prompt.".into()),
            max_iterations: Some(50),
            ..Default::default()
        };
        assert_overlay(
            AgentConfigOverlay::fold([harness, agent, session]),
            expected,
        );
        assert_overlay(AgentConfigOverlay::fold([]), AgentConfigOverlay::default());
    }

    #[test]
    fn normalize_workspace_prefix_preserves_other_namespaces() {
        for (input, expected) in [
            ("/workspace/README.md", "/README.md"),
            ("/workspace", "/"),
            ("README.md", "/README.md"),
            ("/README.md", "/README.md"),
            ("/workspace//nested/file", "/nested/file"),
            ("/workspace/", "/"),
            ("/workspace-other/file", "/workspace-other/file"),
            ("", "/"),
        ] {
            assert_eq!(normalize_initial_file_path(input), expected, "{input:?}");
        }
    }

    #[test]
    fn harness_projection_preserves_all_supported_overlay_fields() {
        let mut expected = sample_overlay();
        expected.tools.clear();
        expected.max_iterations = None;
        let harness = HarnessDefinition {
            name: "harness".into(),
            system_prompt: expected.system_prompt.clone(),
            capabilities: expected.capabilities.clone(),
            initial_files: expected.initial_files.clone(),
            network_access: expected.network_access.clone(),
            default_model_id: expected.default_model_id,
            parallel_tool_calls: expected.parallel_tool_calls,
            mcp_servers: expected.mcp_servers.clone(),
            ..Default::default()
        };
        assert_overlay(AgentConfigOverlay::from(&harness), expected);
    }

    #[test]
    fn agent_projection_preserves_all_overlay_fields() {
        let expected = sample_overlay();
        let mut agent =
            AgentDefinition::new(crate::typed_id::AgentId::new(), "agent", "Base prompt.");
        agent.capabilities = expected.capabilities.clone();
        agent.initial_files = expected.initial_files.clone();
        agent.network_access = expected.network_access.clone();
        agent.default_model_id = expected.default_model_id;
        agent.tools = expected.tools.clone();
        agent.max_iterations = expected.max_iterations;
        agent.parallel_tool_calls = expected.parallel_tool_calls;
        agent.mcp_servers = expected.mcp_servers.clone();
        assert_overlay(AgentConfigOverlay::from(&agent), expected);
    }

    #[test]
    fn session_projection_preserves_fields_and_appends_only_nonempty_goal() {
        for (prompt, goal, expected_prompt) in [
            (
                Some(" Session prompt. "),
                Some(" goal text \n"),
                Some("Session prompt.\n\n<session-goal>\ngoal text\n</session-goal>"),
            ),
            (
                None,
                Some("goal"),
                Some("<session-goal>\ngoal\n</session-goal>"),
            ),
            (Some("prompt"), Some(" \t"), Some("prompt")),
            (None, None, None),
        ] {
            let mut expected = sample_overlay();
            expected.system_prompt = expected_prompt.map(str::to_owned);
            let mut session = ExecutionSession::new(
                crate::typed_id::SessionId::new(),
                crate::typed_id::WorkspaceId::new(),
                crate::typed_id::HarnessId::new(),
            );
            session.system_prompt = prompt.map(str::to_owned);
            session.goal = goal.map(str::to_owned);
            session.capabilities = expected.capabilities.clone();
            session.initial_files = expected.initial_files.clone();
            session.network_access = expected.network_access.clone();
            session.model_id = expected.default_model_id;
            session.tools = expected.tools.clone();
            session.max_iterations = expected.max_iterations;
            session.parallel_tool_calls = expected.parallel_tool_calls;
            session.mcp_servers = expected.mcp_servers.clone();
            assert_overlay(AgentConfigOverlay::from(&session), expected);
        }
    }
}

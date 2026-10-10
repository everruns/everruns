// MCP Virtual Capability
//
// Spec: knowledge/integrations/mcp.md (umbrella), knowledge/integrations/mcp-servers.md (capabilities integration)
//
// This module provides a capability wrapper for MCP servers.
// Each active MCP server becomes a virtual capability that contributes
// its tools to the agent's tool set.
//
// Design decisions:
// - Capability ID format: "mcp:{server_id}" using the MCP server's UUID
// - Tool names are prefixed: "mcp_{sanitized_server_name}_{tool_name}"
// - Tool execution is delegated to the MCP server via HTTP
// - Tools are cached and refreshed periodically

use crate::capabilities::Capability;
use crate::capability_types::CapabilityStatus;
use crate::mcp_server::{McpToolDefinition, McpToolLabel, mcp_tool_name};
use crate::tools::Tool;
use everruns_contracts::CapabilityId;
use everruns_contracts::tool_types::{
    BuiltinTool, DeferrablePolicy, ToolDefinition, ToolHints, ToolPolicy,
};
use std::collections::HashMap;
use uuid::Uuid;

/// A person's saved labels for one server's tools, keyed by the tool's own
/// (unprefixed) name.
pub type McpToolLabels = HashMap<String, McpToolLabel>;

/// MCP Virtual Capability ID prefix
pub const MCP_CAPABILITY_PREFIX: &str = "mcp:";

/// Generate capability ID for an MCP server
pub fn mcp_capability_id(server_id: Uuid) -> String {
    format!("{}{}", MCP_CAPABILITY_PREFIX, server_id)
}

/// Check if a capability ID is an MCP capability
pub fn is_mcp_capability(capability_id: &str) -> bool {
    capability_id.starts_with(MCP_CAPABILITY_PREFIX)
}

/// Parse MCP server ID from capability ID
pub fn parse_mcp_capability_id(capability_id: &str) -> Option<Uuid> {
    if !capability_id.starts_with(MCP_CAPABILITY_PREFIX) {
        return None;
    }
    let uuid_str = &capability_id[MCP_CAPABILITY_PREFIX.len()..];
    Uuid::parse_str(uuid_str).ok()
}

/// MCP Virtual Capability wrapping an MCP server.
///
/// This capability provides tools from a remote MCP server.
/// Tool names are prefixed with "mcp_{server_name}_" to avoid collisions.
#[derive(Debug, Clone)]
pub struct McpCapability {
    /// MCP server UUID
    pub server_id: Uuid,
    /// Server name (used for tool name prefix)
    pub server_name: String,
    /// Server description
    pub description: Option<String>,
    /// Cached tool definitions from the MCP server
    pub tools: Vec<McpToolDefinition>,
    /// A person's saved risk labels; each one wins over the tool's annotations.
    pub tool_labels: McpToolLabels,
}

impl McpCapability {
    /// Create a new MCP capability from server info and cached tools
    pub fn new(
        server_id: Uuid,
        server_name: String,
        description: Option<String>,
        tools: Vec<McpToolDefinition>,
    ) -> Self {
        Self {
            server_id,
            server_name,
            description,
            tools,
            tool_labels: McpToolLabels::new(),
        }
    }

    /// Apply a person's saved per-tool risk labels to the definitions.
    pub fn with_tool_labels(mut self, tool_labels: McpToolLabels) -> Self {
        self.tool_labels = tool_labels;
        self
    }

    /// Get the capability ID for this MCP server
    pub fn capability_id(&self) -> String {
        mcp_capability_id(self.server_id)
    }

    /// Convert MCP tool definition to our ToolDefinition with prefixed name.
    /// Maps MCP annotations to ToolHints when available.
    fn mcp_tool_to_definition(&self, mcp_tool: &McpToolDefinition) -> ToolDefinition {
        let prefixed_name = mcp_tool_name(&self.server_name, &mcp_tool.name);

        // Map MCP annotations to ToolHints
        let mut hints = match &mcp_tool.annotations {
            Some(ann) => ToolHints {
                readonly: ann.read_only_hint,
                destructive: ann.destructive_hint,
                idempotent: ann.idempotent_hint,
                // Default open_world to true for MCP tools unless explicitly set to false
                open_world: Some(ann.open_world_hint.unwrap_or(true)),
                // MCP doesn't define requires_secrets or long_running — leave as None
                ..ToolHints::default()
            },
            None => {
                // MCP tools are external by nature
                ToolHints::default().with_open_world(true)
            }
        };
        if let Some(label) = self.tool_labels.get(&mcp_tool.name) {
            label.apply(&mut hints);
        }

        ToolDefinition::Builtin(BuiltinTool {
            name: prefixed_name,
            display_name: Some(
                mcp_tool
                    .title
                    .as_deref()
                    .filter(|title| !title.trim().is_empty())
                    .map(crate::tool_narration::narration_detail)
                    .unwrap_or_else(|| {
                        format!(
                            "{}: {}",
                            self.server_name,
                            mcp_tool
                                .name
                                .split(['_', '-', '.'])
                                .filter(|part| !part.is_empty())
                                .collect::<Vec<_>>()
                                .join(" ")
                        )
                    }),
            ),
            description: mcp_tool
                .description
                .clone()
                .unwrap_or_else(|| format!("Tool from MCP server: {}", self.server_name)),
            parameters: mcp_tool.input_schema.clone(),
            policy: ToolPolicy::Auto,
            category: self.category().map(|s| s.to_string()),
            deferrable: DeferrablePolicy::default(),
            hints,
            full_parameters: None,
        })
        .with_capability_attribution(self.capability_id(), Some(self.server_name.clone()))
    }
}

impl Capability for McpCapability {
    fn id(&self) -> &str {
        // Return a static reference by leaking the capability ID
        // This is acceptable since capabilities are long-lived
        Box::leak(self.capability_id().into_boxed_str())
    }

    fn name(&self) -> &str {
        Box::leak(self.server_name.clone().into_boxed_str())
    }

    fn description(&self) -> &str {
        let desc = self
            .description
            .clone()
            .unwrap_or_else(|| format!("MCP Server providing {} tool(s)", self.tools.len()));
        Box::leak(desc.into_boxed_str())
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("mcp") // Official MCP logo
    }

    fn category(&self) -> Option<&str> {
        Some("MCP Servers")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        None // MCP tools are self-documenting
    }

    fn narrate(
        &self,
        _tool_def: Option<&everruns_contracts::tool_types::ToolDefinition>,
        tool_call: &everruns_contracts::tool_types::ToolCall,
        phase: crate::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: crate::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        // Generic search narration for provider/MCP search tools (`*__search`).
        if !self
            .tools
            .iter()
            .any(|tool| mcp_tool_name(&self.server_name, &tool.name) == tool_call.name)
            || !tool_call.name.ends_with("__search")
        {
            return None;
        }
        Some(crate::tool_narration::narrate_provider_search(
            &tool_call.arguments,
            phase,
            locale,
        ))
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        // MCP tools are executed via HTTP, not directly
        // Return empty vec - tool execution is handled specially
        vec![]
    }

    fn tool_definitions(&self) -> Vec<ToolDefinition> {
        // Existing stored configs also pass here: never publish a name that
        // parses as a different server/tool pair.
        if !crate::mcp_server::is_valid_mcp_server_name(&self.server_name) {
            return vec![];
        }
        self.tools
            .iter()
            .map(|t| self.mcp_tool_to_definition(t))
            .collect()
    }
}

/// MCP-namespace helpers for [`CapabilityId`].
///
/// An extension trait because the ID type lives in the neutral
/// `everruns-contracts` contract crate while the `mcp:` namespace is owned
/// by this capability implementation.
pub trait McpCapabilityIdExt: Sized {
    /// Check if this capability ID is for an MCP server
    fn is_mcp(&self) -> bool;
    /// Create a capability ID for an MCP server
    fn mcp(server_id: Uuid) -> Self;
    /// Parse MCP server UUID from this capability ID
    fn mcp_server_id(&self) -> Option<Uuid>;
}

impl McpCapabilityIdExt for CapabilityId {
    fn is_mcp(&self) -> bool {
        is_mcp_capability(self.as_str())
    }

    fn mcp(server_id: Uuid) -> Self {
        Self::new(mcp_capability_id(server_id))
    }

    fn mcp_server_id(&self) -> Option<Uuid> {
        parse_mcp_capability_id(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn capability_id_helpers_preserve_wire_identity_and_reject_invalid_namespaces() {
        let id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let wire = "mcp:550e8400-e29b-41d4-a716-446655440000";
        assert_eq!(mcp_capability_id(id), wire);
        assert!(is_mcp_capability(wire));
        assert_eq!(parse_mcp_capability_id(wire), Some(id));
        let capability_id = CapabilityId::mcp(id);
        assert_eq!(capability_id.as_str(), wire);
        assert!(capability_id.is_mcp());
        assert_eq!(capability_id.mcp_server_id(), Some(id));
        for invalid in ["current_time", "mcp_something", "mcp:invalid", "mcp:"] {
            assert_eq!(parse_mcp_capability_id(invalid), None);
            assert_eq!(CapabilityId::new(invalid).mcp_server_id(), None);
        }
        assert!(!is_mcp_capability("current_time"));
        assert!(!is_mcp_capability("mcp_something"));
        assert!(!CapabilityId::new("current_time").is_mcp());
        assert!(is_mcp_capability("mcp:invalid"));
    }

    #[test]
    fn capability_definitions_preserve_schema_attribution_and_annotation_overrides() {
        let id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let schema = json!({"type":"object","properties":{"query":{"type":"string","minLength":3}},"required":["query"]});
        for (
            annotations,
            expected_readonly,
            expected_destructive,
            expected_idempotent,
            expected_open_world,
        ) in [
            (None, None, None, None, true),
            (
                Some(crate::McpToolAnnotations {
                    read_only_hint: Some(true),
                    destructive_hint: Some(false),
                    idempotent_hint: Some(true),
                    open_world_hint: Some(false),
                }),
                Some(true),
                Some(false),
                Some(true),
                false,
            ),
        ] {
            let capability = McpCapability::new(
                id,
                "microsoft-learn".into(),
                Some("Microsoft Learn MCP".into()),
                vec![McpToolDefinition {
                    name: "search".into(),
                    title: None,
                    description: Some("Search documentation".into()),
                    input_schema: schema.clone(),
                    annotations,
                }],
            );
            let defs = capability.tool_definitions();
            assert_eq!(defs.len(), 1);
            let ToolDefinition::Builtin(builtin) = &defs[0] else {
                panic!("expected builtin")
            };
            assert_eq!(builtin.name, "mcp_microsoft_learn__search");
            assert_eq!(builtin.description, "Search documentation");
            assert_eq!(builtin.parameters, schema);
            assert_eq!(builtin.hints.readonly, expected_readonly);
            assert_eq!(builtin.hints.destructive, expected_destructive);
            assert_eq!(builtin.hints.idempotent, expected_idempotent);
            assert_eq!(builtin.hints.open_world, Some(expected_open_world));
            assert_eq!(
                defs[0].capability_attribution(),
                Some((
                    "mcp:550e8400-e29b-41d4-a716-446655440000",
                    Some("microsoft-learn")
                ))
            );
        }
    }

    #[test]
    fn person_labels_override_tool_annotations() {
        let tool = |name: &str, read_only: Option<bool>| McpToolDefinition {
            name: name.into(),
            title: None,
            description: None,
            input_schema: json!({"type":"object"}),
            annotations: read_only.map(|read_only| crate::McpToolAnnotations {
                read_only_hint: Some(read_only),
                destructive_hint: Some(!read_only),
                ..Default::default()
            }),
        };
        let capability = McpCapability::new(
            Uuid::nil(),
            "docs".into(),
            None,
            vec![
                // Declares nothing: a read_only label clears the open_world default.
                tool("search", None),
                // Declares itself read-only: a changes label wins.
                tool("fetch", Some(true)),
                // No label: keeps the default, which asks in normal mode.
                tool("other", None),
            ],
        )
        .with_tool_labels(McpToolLabels::from([
            ("search".to_string(), McpToolLabel::ReadOnly),
            ("fetch".to_string(), McpToolLabel::Changes),
        ]));
        let hints = |name: &str| {
            capability
                .tool_definitions()
                .into_iter()
                .find(|def| def.name() == mcp_tool_name("docs", name))
                .unwrap()
                .hints()
                .clone()
        };
        let search = hints("search");
        assert_eq!(
            (search.readonly, search.destructive, search.open_world),
            (Some(true), Some(false), Some(false))
        );
        let fetch = hints("fetch");
        assert_eq!(
            (fetch.readonly, fetch.destructive, fetch.open_world),
            (Some(false), Some(true), Some(true))
        );
        let other = hints("other");
        assert_eq!(
            (other.readonly, other.destructive, other.open_world),
            (None, None, Some(true))
        );
    }

    #[test]
    fn ambiguous_server_names_do_not_publish_misrouted_tool_definitions() {
        for name in ["docs_", "docs-", "docs__private", "docs..private", "_", ""] {
            let capability = McpCapability::new(
                Uuid::nil(),
                name.into(),
                None,
                vec![McpToolDefinition {
                    name: "search".into(),
                    title: None,
                    description: None,
                    input_schema: json!({"type":"object"}),
                    annotations: None,
                }],
            );
            assert!(
                capability.tool_definitions().is_empty(),
                "ambiguous server {name:?} published tools"
            );
        }
        let capability = McpCapability::new(
            Uuid::nil(),
            "docs_api".into(),
            None,
            vec![McpToolDefinition {
                name: "read__file".into(),
                title: None,
                description: None,
                input_schema: json!({"type":"object"}),
                annotations: None,
            }],
        );
        let definitions = capability.tool_definitions();
        assert_eq!(definitions.len(), 1);
        assert_eq!(
            crate::parse_mcp_tool_name(definitions[0].name()),
            Some(("docs_api".into(), "read__file".into()))
        );
    }
}

#[cfg(test)]
mod narration_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn remote_titles_survive_discovery_and_generic_narration() {
        for (title, expected) in [
            (Some("Read project notes"), "Read project notes"),
            (None, "notes: read file"),
            (Some("  "), "notes: read file"),
        ] {
            let tool: McpToolDefinition = serde_json::from_value(
                json!({"name":"read_file", "title":title, "inputSchema":{"type":"object"}}),
            )
            .unwrap();
            let capability = McpCapability::new(Uuid::nil(), "notes".into(), None, vec![tool]);
            let definitions = capability.tool_definitions();
            assert_eq!(definitions[0].display_name(), Some(expected));
            let call = everruns_contracts::tool_types::ToolCall {
                id: "call".into(),
                name: definitions[0].name().into(),
                arguments: json!({}),
            };
            assert_eq!(
                crate::tool_narration::render_tool_narration(
                    Some(&definitions[0]),
                    &call,
                    crate::tool_narration::ToolNarrationPhase::Completed
                ),
                format!("Ran {expected}")
            );
        }
    }

    #[test]
    fn capability_only_narrates_search_tools_it_owns() {
        let capability = McpCapability::new(
            Uuid::nil(),
            "notes".into(),
            None,
            vec![
                serde_json::from_value(json!({"name":"search", "inputSchema":{"type":"object"}}))
                    .unwrap(),
            ],
        );
        for (name, owned) in [("mcp_notes__search", true), ("mcp_other__search", false)] {
            let call = everruns_contracts::tool_types::ToolCall {
                id: "call".into(),
                name: name.into(),
                arguments: json!({"query":"release notes"}),
            };
            assert_eq!(
                capability
                    .narrate(
                        None,
                        &call,
                        crate::tool_narration::ToolNarrationPhase::Completed,
                        None,
                        crate::tool_narration::ToolNarrationContext::default()
                    )
                    .is_some(),
                owned
            );
        }
    }
}

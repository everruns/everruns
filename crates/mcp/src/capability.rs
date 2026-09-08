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

use everruns_capability::CapabilityId;
use everruns_core::capabilities::Capability;
use everruns_core::capability_types::CapabilityStatus;
use everruns_core::mcp_server::{McpToolDefinition, mcp_tool_name};
use everruns_core::tools::Tool;
use everruns_provider::tool_types::{
    BuiltinTool, DeferrablePolicy, ToolDefinition, ToolHints, ToolPolicy,
};
use uuid::Uuid;

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
        }
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
        let hints = match &mcp_tool.annotations {
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

        ToolDefinition::Builtin(BuiltinTool {
            name: prefixed_name,
            display_name: None,
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
        _tool_def: Option<&everruns_provider::tool_types::ToolDefinition>,
        tool_call: &everruns_provider::tool_types::ToolCall,
        phase: everruns_core::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: everruns_core::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        // Generic search narration for provider/MCP search tools (`*__search`).
        if !tool_call.name.ends_with("__search") {
            return None;
        }
        Some(everruns_core::tool_narration::narrate_provider_search(
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
        if !everruns_core::mcp_server::is_valid_mcp_server_name(&self.server_name) {
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
/// `everruns-capability` contract crate while the `mcp:` namespace is owned
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
                Some(everruns_core::McpToolAnnotations {
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
    fn ambiguous_server_names_do_not_publish_misrouted_tool_definitions() {
        for name in ["docs_", "docs-", "docs__private", "docs..private", "_", ""] {
            let capability = McpCapability::new(
                Uuid::nil(),
                name.into(),
                None,
                vec![McpToolDefinition {
                    name: "search".into(),
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
                description: None,
                input_schema: json!({"type":"object"}),
                annotations: None,
            }],
        );
        let definitions = capability.tool_definitions();
        assert_eq!(definitions.len(), 1);
        assert_eq!(
            everruns_core::parse_mcp_tool_name(definitions[0].name()),
            Some(("docs_api".into(), "read__file".into()))
        );
    }
}

// MCP Virtual Capability
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

use crate::capability_types::{CapabilityId, CapabilityStatus};
use crate::mcp_server::{McpToolDefinition, mcp_tool_name};
use crate::tool_types::{BuiltinTool, DeferrablePolicy, ToolDefinition, ToolPolicy};
use crate::tools::Tool;

use super::Capability;
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

    /// Convert MCP tool definition to our ToolDefinition with prefixed name
    fn mcp_tool_to_definition(&self, mcp_tool: &McpToolDefinition) -> ToolDefinition {
        let prefixed_name = mcp_tool_name(&self.server_name, &mcp_tool.name);

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
        })
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

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        // MCP tools are executed via HTTP, not directly
        // Return empty vec - tool execution is handled specially
        vec![]
    }

    fn tool_definitions(&self) -> Vec<ToolDefinition> {
        self.tools
            .iter()
            .map(|t| self.mcp_tool_to_definition(t))
            .collect()
    }
}

/// Capability ID constant for MCP capabilities
impl CapabilityId {
    /// Check if this capability ID is for an MCP server
    pub fn is_mcp(&self) -> bool {
        is_mcp_capability(self.as_str())
    }

    /// Create a capability ID for an MCP server
    pub fn mcp(server_id: Uuid) -> Self {
        Self::new(mcp_capability_id(server_id))
    }

    /// Parse MCP server UUID from this capability ID
    pub fn mcp_server_id(&self) -> Option<Uuid> {
        parse_mcp_capability_id(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_mcp_capability_id() {
        let server_id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let cap_id = mcp_capability_id(server_id);
        assert_eq!(cap_id, "mcp:550e8400-e29b-41d4-a716-446655440000");
    }

    #[test]
    fn test_is_mcp_capability() {
        assert!(is_mcp_capability(
            "mcp:550e8400-e29b-41d4-a716-446655440000"
        ));
        assert!(!is_mcp_capability("current_time"));
        assert!(!is_mcp_capability("mcp_something")); // Wrong format
    }

    #[test]
    fn test_parse_mcp_capability_id() {
        let server_id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let cap_id = mcp_capability_id(server_id);
        let parsed = parse_mcp_capability_id(&cap_id);
        assert_eq!(parsed, Some(server_id));

        assert_eq!(parse_mcp_capability_id("current_time"), None);
        assert_eq!(parse_mcp_capability_id("mcp:invalid"), None);
    }

    #[test]
    fn test_mcp_capability_tool_definitions() {
        let server_id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let tools = vec![McpToolDefinition {
            name: "search".to_string(),
            description: Some("Search documentation".to_string()),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string" }
                }
            }),
        }];

        let capability = McpCapability::new(
            server_id,
            "microsoft-learn".to_string(),
            Some("Microsoft Learn MCP".to_string()),
            tools,
        );

        let defs = capability.tool_definitions();
        assert_eq!(defs.len(), 1);

        let ToolDefinition::Builtin(builtin) = &defs[0] else {
            panic!("expected Builtin variant");
        };
        assert_eq!(builtin.name, "mcp_microsoft_learn__search");
        assert_eq!(builtin.description, "Search documentation");
    }

    #[test]
    fn test_capability_id_mcp_methods() {
        let server_id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let cap_id = CapabilityId::mcp(server_id);

        assert!(cap_id.is_mcp());
        assert_eq!(cap_id.mcp_server_id(), Some(server_id));

        let regular_cap = CapabilityId::new("current_time");
        assert!(!regular_cap.is_mcp());
        assert_eq!(regular_cap.mcp_server_id(), None);
    }
}

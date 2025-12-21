// Tool definitions and policies for agent execution
//
// Runtime types are defined in everruns-core and re-exported here
// for backward compatibility.

// Re-export all tool runtime types from core
pub use everruns_core::tool_types::{
    BuiltinTool, BuiltinToolKind, ToolCall, ToolDefinition, ToolPolicy, ToolResult,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builtin_tool_http_get() {
        let json = r#"{
            "type": "builtin",
            "name": "fetch_data",
            "description": "Fetch data from URL",
            "parameters": {"type": "object"},
            "kind": "http_get"
        }"#;

        let tool: ToolDefinition = serde_json::from_str(json).unwrap();
        match tool {
            ToolDefinition::Builtin(builtin) => {
                assert_eq!(builtin.kind, BuiltinToolKind::HttpGet);
                assert_eq!(builtin.policy, ToolPolicy::Auto);
            }
        }
    }

    #[test]
    fn test_builtin_tool_requires_approval() {
        let json = r#"{
            "type": "builtin",
            "name": "delete_file",
            "description": "Delete a file",
            "parameters": {"type": "object"},
            "kind": "write_file",
            "policy": "requires_approval"
        }"#;

        let tool: ToolDefinition = serde_json::from_str(json).unwrap();
        match tool {
            ToolDefinition::Builtin(builtin) => {
                assert_eq!(builtin.policy, ToolPolicy::RequiresApproval);
            }
        }
    }
}

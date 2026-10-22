//! Preserve authorized MCP source metadata at the worker boundary.

use super::{proto, proto_struct_to_json};

/// Convert proto McpToolDef to core ToolDefinition. Hints carry annotations,
/// saved risk labels and source search metadata. Older servers omit them;
/// preserve the open_world default without inventing a source description.
pub(super) fn proto_mcp_tool_def_to_tool_definition(
    t: proto::McpToolDef,
) -> everruns_contracts::tool_types::ToolDefinition {
    use everruns_contracts::tool_types::{BuiltinTool, ToolDefinition, ToolHints};
    let parameters = t.parameters.map(|s| proto_struct_to_json(&s));
    let mut hints = ToolHints {
        readonly: t.readonly,
        destructive: t.destructive,
        idempotent: t.idempotent,
        open_world: Some(t.open_world.unwrap_or(true)),
        metadata: t.search_metadata.as_ref().map(proto_struct_to_json),
        ..Default::default()
    };
    if !t.capability_id.is_empty() {
        hints = hints.with_capability_attribution(
            t.capability_id.clone(),
            (!t.capability_name.is_empty()).then_some(t.capability_name.clone()),
        );
    }
    ToolDefinition::Builtin(BuiltinTool {
        name: t.name,
        display_name: t.display_name,
        description: t.description,
        parameters: parameters.unwrap_or_else(|| serde_json::json!({"type": "object"})),
        policy: everruns_contracts::tool_types::ToolPolicy::Auto, // MCP tools are auto-executed
        category: None,
        deferrable: Default::default(),
        hints,
        full_parameters: None,
    })
}

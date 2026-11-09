//! Model-facing source context for scoped MCP discovery.

use crate::kernel_imports::{ScopedMcpServers, contracts::tool_types::ToolDefinition};
use everruns_core::mcp_server::sanitize_mcp_server_name;
use std::collections::HashMap;

/// Attach the authorized server identity and description to its MCP tools.
/// The search index uses this metadata to rank source descriptions separately
/// from operation names and schemas.
pub(crate) fn attach_server_search_metadata(
    definitions: &mut [ToolDefinition],
    servers: &ScopedMcpServers,
) {
    let servers_by_prefix: HashMap<_, _> = servers
        .iter()
        .map(|(name, server)| (sanitize_mcp_server_name(name), (name, server)))
        .collect();
    for definition in definitions {
        let prefix = everruns_core::deferred_mcp_server_prefix(definition.name())
            .map(str::to_owned)
            .or_else(|| {
                everruns_core::mcp_server::parse_mcp_tool_name(definition.name())
                    .map(|(prefix, _)| prefix)
            });
        let Some((server_name, server)) = prefix
            .as_deref()
            .and_then(|prefix| servers_by_prefix.get(prefix).copied())
        else {
            continue;
        };
        let ToolDefinition::Builtin(tool) = definition else {
            continue;
        };
        if let Some(context) = &server.capability_context {
            tool.hints.capability_id = Some(context.id.clone());
            tool.hints.capability_name = Some(context.name.clone());
            tool.hints.capability_description = Some(everruns_core::mcp::visible_mcp_description(
                &context.description,
            ));
        }
        let metadata = tool
            .hints
            .metadata
            .get_or_insert_with(|| serde_json::json!({}));
        let Some(metadata) = metadata.as_object_mut() else {
            // Keep non-object extension metadata intact instead of replacing it.
            continue;
        };
        let server_metadata = metadata
            .entry("mcp_server")
            .or_insert_with(|| serde_json::json!({}));
        let Some(server_metadata) = server_metadata.as_object_mut() else {
            continue;
        };
        server_metadata.insert("name".to_string(), serde_json::json!(server_name));
        if let Some(description) = server
            .description
            .as_deref()
            .filter(|description| !description.trim().is_empty())
        {
            server_metadata.insert(
                "description".to_string(),
                serde_json::json!(everruns_core::mcp::visible_mcp_description(description)),
            );
        }
    }
}

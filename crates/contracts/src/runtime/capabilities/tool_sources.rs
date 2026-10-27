//! Attach public contributing-capability context to discovered MCP operations.

use super::{
    AgentCapabilityConfig, CapabilityRegistry, DeclarativeCapabilityDefinition,
    is_declarative_capability, superseded_capability_ids,
};
use crate::runtime::mcp_deferred::deferred_mcp_server_prefix;
use crate::runtime::mcp_server::{parse_mcp_tool_name, sanitize_mcp_server_name};
use crate::tool_types::ToolDefinition;
use std::collections::HashMap;

/// Source context follows the current capability composition. It is metadata,
/// never a grant to load a server or invoke an operation.
pub fn attribute_capability_mcp_tools(
    tools: &mut [ToolDefinition],
    configs: &[AgentCapabilityConfig],
    registry: &CapabilityRegistry,
) {
    let superseded = superseded_capability_ids(configs, registry);
    let mut sources = HashMap::new();
    for config in configs {
        let id = config.capability_id();
        if superseded.contains(id) {
            continue;
        }
        let (name, description, servers) = if is_declarative_capability(id)
            || crate::is_plugin_capability(id)
        {
            let Ok(definition) = serde_json::from_value::<DeclarativeCapabilityDefinition>(
                config.config_value().clone(),
            ) else {
                continue;
            };
            if !definition.status.is_active() {
                continue;
            }
            (
                definition.display_name.unwrap_or(definition.name),
                definition.description,
                definition.mcp_servers.unwrap_or_default(),
            )
        } else {
            let Some(capability) = registry.get(id).filter(|cap| cap.status().is_active()) else {
                continue;
            };
            (
                capability.name().to_owned(),
                capability.description().to_owned(),
                capability.mcp_servers_with_config(config.config_value()),
            )
        };
        for server in servers.keys() {
            sources.insert(
                sanitize_mcp_server_name(server),
                (id.to_owned(), name.clone(), description.clone()),
            );
        }
    }
    for tool in tools {
        let source = deferred_mcp_server_prefix(tool.name())
            .map(str::to_owned)
            .or_else(|| parse_mcp_tool_name(tool.name()).map(|(source, _)| source));
        if let Some((id, name, description)) = source.and_then(|source| sources.get(&source)) {
            *tool = tool
                .clone()
                .with_capability_attribution(id.clone(), Some(name.clone()))
                .with_capability_description(description.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::mcp_deferred::deferred_mcp_server_definition;
    use crate::runtime::tool_catalog::ToolSearchIndex;
    use serde_json::json;

    #[test]
    fn deferred_plugin_sources_include_capability_text_without_changing_their_schema() {
        let definition = deferred_mcp_server_definition("records", Some("Read records"));
        let mut tools = vec![definition.clone()];
        let config = AgentCapabilityConfig::with_config(
            "plugin:records",
            json!({
                "name":"records", "display_name":"Records research", "description":"Turquoise observatory",
                "mcp_servers":{"records":{"url":"https://example.test/mcp", "actsAs":"user"}}
            }),
        );
        attribute_capability_mcp_tools(&mut tools, &[config], &CapabilityRegistry::new());
        assert_eq!(tools[0].parameters(), definition.parameters());
        assert_eq!(
            tools[0].hints().capability_id.as_deref(),
            Some("plugin:records")
        );
        let hits = ToolSearchIndex::new(&tools).search("turquoise", 8, |_| true);
        assert_eq!(hits.len(), 1);
        assert!(hits[0].capability_match);
        assert!(
            ToolSearchIndex::new(&tools)
                .search("turquoise", 8, |_| false)
                .is_empty()
        );
    }
}

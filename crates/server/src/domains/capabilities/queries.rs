// Capability query helpers.
//
// Capabilities are a read-only registry backed by CapabilityService.
// No direct DB access — all reads delegate to the service which combines
// built-in capabilities, MCP servers, and skills.
//
// Filtering and pagination are done in-memory since the registry is small
// (~30-50 items).

use super::types::CapabilityInfo;
use super::types::{DeclarativeCapability, DeclarativeCapabilityRow};
use crate::kernel_imports::{
    AgentCapabilityConfig, DeclarativeCapabilityDefinition, declarative_capability_id,
    everruns_provider::typed_id::DeclarativeCapabilityId, hydrate_declarative_capability_config,
    hydrate_plugin_capability_config, is_declarative_capability, is_plugin_capability,
    parse_declarative_capability_id, parse_plugin_capability_id,
};
use crate::storage::StorageBackend;

fn normalize_legacy_mcp_identity(value: &mut serde_json::Value) {
    if let Some(servers) = value
        .get_mut("mcp_servers")
        .and_then(serde_json::Value::as_object_mut)
    {
        for server in servers.values_mut() {
            let Some(server) = server.as_object_mut() else {
                continue;
            };
            let has_identity = server.contains_key("actsAs") || server.contains_key("acts_as");
            let has_oauth = server
                .get("auth_mode")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|mode| mode != "none")
                || server.contains_key("oauth_provider_id");
            // Only a legacy inline transport with no credential source has an
            // unambiguous identity. OAuth and presets need an explicit choice.
            if !has_identity && !has_oauth && !server.contains_key("use") {
                server.insert(
                    "actsAs".to_string(),
                    serde_json::Value::String("none".to_string()),
                );
            }
        }
    }
}

fn deserialize_persisted_definition(
    mut value: serde_json::Value,
) -> DeclarativeCapabilityDefinition {
    normalize_legacy_mcp_identity(&mut value);
    serde_json::from_value(value).unwrap_or_default()
}

/// Filter capabilities by search query (name/description match).
pub fn filter_by_search(capabilities: &mut Vec<CapabilityInfo>, search: &str) {
    capabilities.retain(|c| c.matches_search(search));
}

pub fn row_to_declarative_capability(row: &DeclarativeCapabilityRow) -> DeclarativeCapability {
    let mut definition = deserialize_persisted_definition(row.definition.clone());
    definition.name = row.name.clone();
    definition.display_name = row.display_name.clone();
    definition.description = row.description.clone();
    definition.status = match row.status.as_str() {
        "disabled" | "archived" => everruns_core::CapabilityStatus::Retired,
        _ => everruns_core::CapabilityStatus::Available,
    };
    DeclarativeCapability {
        public_id: row
            .public_id
            .parse()
            .unwrap_or_else(|_| DeclarativeCapabilityId::from_uuid(row.id)),
        internal_id: row.id,
        capability_id: declarative_capability_id(&row.name),
        name: row.name.clone(),
        display_name: row.display_name.clone(),
        description: row.description.clone(),
        status: row.status.clone(),
        definition,
        created_at: row.created_at,
        updated_at: row.updated_at,
        archived_at: row.archived_at,
        deleted_at: row.deleted_at,
    }
}

pub async fn hydrate_declarative_capability_configs(
    db: &StorageBackend,
    org_id: i64,
    capabilities: Vec<AgentCapabilityConfig>,
) -> anyhow::Result<Vec<AgentCapabilityConfig>> {
    let mut hydrated = Vec::with_capacity(capabilities.len());
    for cap in capabilities {
        let cap_id = cap.capability_id().to_string();
        if is_declarative_capability(&cap_id)
            && let Some(name) = parse_declarative_capability_id(&cap_id)
            && let Some(row) = db.get_declarative_capability_by_name(org_id, name).await?
            && matches!(row.status.as_str(), "active" | "disabled")
        {
            let mut definition = deserialize_persisted_definition(row.definition);
            definition.name = row.name;
            definition.display_name = row.display_name;
            definition.description = row.description;
            definition.status = if row.status == "active" {
                everruns_core::CapabilityStatus::Available
            } else {
                everruns_core::CapabilityStatus::Retired
            };
            hydrated.push(AgentCapabilityConfig::with_config(
                cap_id,
                hydrate_declarative_capability_config(cap.config_value().clone(), &definition),
            ));
        } else if is_plugin_capability(&cap_id)
            && let Some(plugin_public_id) = parse_plugin_capability_id(&cap_id)
            && let Some(row) = db
                .get_plugin_install_by_public_id(org_id, plugin_public_id)
                .await?
            && row.status == "active"
        {
            let definition = deserialize_persisted_definition(row.definition);
            hydrated.push(AgentCapabilityConfig::with_config(
                cap_id,
                hydrate_plugin_capability_config(cap.config_value().clone(), &definition),
            ));
        } else {
            hydrated.push(cap);
        }
    }
    Ok(hydrated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persisted_unauthenticated_inline_server_without_acts_as_loads_as_none() {
        let definition = deserialize_persisted_definition(serde_json::json!({
            "name": "legacy",
            "description": "Legacy capability",
            "mcp_servers": {
                "docs": {
                    "url": "https://learn.microsoft.com/api/mcp"
                }
            }
        }));

        let server = definition
            .mcp_servers
            .expect("legacy MCP servers should load")
            .remove("docs")
            .expect("legacy MCP server should remain present");
        assert_eq!(server.acts_as, everruns_core::McpServerActsAs::None);
    }

    #[test]
    fn persisted_oauth_server_without_acts_as_is_not_assigned_an_identity() {
        let mut definition = serde_json::json!({
            "name": "legacy",
            "description": "Legacy capability",
            "mcp_servers": {
                "oauth": {
                    "url": "https://mcp.example.com/mcp",
                    "auth_mode": "oauth",
                    "oauth_provider_id": "mcp_oauth_example"
                }
            }
        });

        normalize_legacy_mcp_identity(&mut definition);
        assert!(
            definition["mcp_servers"]["oauth"].get("actsAs").is_none(),
            "ambiguous OAuth identity must not be inferred"
        );
    }
}

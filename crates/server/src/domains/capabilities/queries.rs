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

/// Filter capabilities by search query (name/description match).
pub fn filter_by_search(capabilities: &mut Vec<CapabilityInfo>, search: &str) {
    capabilities.retain(|c| c.matches_search(search));
}

pub fn row_to_declarative_capability(row: &DeclarativeCapabilityRow) -> DeclarativeCapability {
    let mut definition: DeclarativeCapabilityDefinition =
        serde_json::from_value(row.definition.clone()).unwrap_or_default();
    definition.name = row.name.clone();
    definition.display_name = row.display_name.clone();
    definition.description = row.description.clone();
    definition.status = match row.status.as_str() {
        "disabled" | "archived" => everruns_core::CapabilityStatus::Deprecated,
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
            let mut definition: DeclarativeCapabilityDefinition =
                serde_json::from_value(row.definition.clone()).unwrap_or_default();
            definition.name = row.name;
            definition.display_name = row.display_name;
            definition.description = row.description;
            definition.status = if row.status == "active" {
                everruns_core::CapabilityStatus::Available
            } else {
                everruns_core::CapabilityStatus::Deprecated
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
            let definition: DeclarativeCapabilityDefinition =
                serde_json::from_value(row.definition.clone()).unwrap_or_default();
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

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
use std::collections::HashSet;
use std::sync::{LazyLock, Mutex};
use uuid::Uuid;

/// Rows already reported as unparseable, so the log records the corruption
/// once rather than once per read. Every capability listing walks this path.
static REPORTED_UNPARSEABLE_ROWS: LazyLock<Mutex<HashSet<Uuid>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

/// Parse a stored declarative definition, or report why it cannot be used.
///
/// EVE-1079: this used to be `unwrap_or_default()` at three call sites. The
/// default is `status: Available` with no system prompt, skills, files or MCP
/// servers, and two of the call sites then wrote the row's own `name`,
/// `display_name` and `description` back over it. A corrupt row therefore
/// presented as an active, correctly named capability that silently
/// contributed nothing — the inverse of the fail-closed contract EVE-1029 set
/// for this subsystem. Returning the error makes the failure representable;
/// what each caller does with it is stated at the call site.
///
/// Reading a row never rewrites it. A definition that cannot be parsed is
/// preserved exactly as stored so it stays diagnosable and repairable.
pub(crate) fn deserialize_persisted_definition(
    row_id: Uuid,
    name: &str,
    definition: &serde_json::Value,
) -> Result<DeclarativeCapabilityDefinition, serde_json::Error> {
    match serde_json::from_value(definition.clone()) {
        Ok(parsed) => Ok(parsed),
        Err(error) => {
            let first_sighting = REPORTED_UNPARSEABLE_ROWS
                .lock()
                .map(|mut seen| seen.insert(row_id))
                .unwrap_or(true);
            if first_sighting {
                tracing::error!(
                    capability = %name,
                    row_id = %row_id,
                    %error,
                    "Stored declarative capability definition could not be parsed; the capability contributes nothing until the row is repaired"
                );
            }
            Err(error)
        }
    }
}

/// Filter capabilities by search query (name/description match).
pub fn filter_by_search(capabilities: &mut Vec<CapabilityInfo>, search: &str) {
    capabilities.retain(|c| c.matches_search(search));
}

pub fn row_to_declarative_capability(row: &DeclarativeCapabilityRow) -> DeclarativeCapability {
    // A read surface, so a corrupt row is shown rather than hidden: the caller
    // still gets the row's identity and the parse error that explains it. The
    // definition is the inert default and the status below is forced to
    // `Retired`, so nothing downstream mistakes it for a working capability.
    let (mut definition, definition_error) =
        match deserialize_persisted_definition(row.id, &row.name, &row.definition) {
            Ok(definition) => (definition, None),
            Err(error) => (
                DeclarativeCapabilityDefinition::default(),
                Some(error.to_string()),
            ),
        };
    definition.name = row.name.clone();
    definition.display_name = row.display_name.clone();
    definition.description = row.description.clone();
    definition.status = if definition_error.is_some() {
        everruns_core::CapabilityStatus::Retired
    } else {
        match row.status.as_str() {
            "disabled" | "archived" => everruns_core::CapabilityStatus::Retired,
            _ => everruns_core::CapabilityStatus::Available,
        }
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
        definition_error,
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
            // Fail closed. This is the runtime contribution path, so the
            // alternative is handing the agent a capability that resolves,
            // reports itself attached, and supplies none of the tools, prompt
            // or MCP servers it names. A turn that stops with the capability
            // named is recoverable; one that quietly loses its tools is not.
            let mut definition =
                deserialize_persisted_definition(row.id, &row.name, &row.definition).map_err(
                    |error| {
                        anyhow::anyhow!(
                            "declarative capability `{}` has a stored definition that cannot be parsed: {error}",
                            row.name
                        )
                    },
                )?;
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
            // Same contract as the declarative branch above: an installed
            // plugin whose definition no longer parses stops the build instead
            // of contributing an empty shell.
            let definition =
                deserialize_persisted_definition(row.id, &cap_id, &row.definition).map_err(
                    |error| {
                        anyhow::anyhow!(
                            "plugin capability `{cap_id}` has a stored definition that cannot be parsed: {error}"
                        )
                    },
                )?;
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
    use crate::storage::models::CreateDeclarativeCapabilityRow;
    use serde_json::json;

    const ORG: i64 = 1;

    fn row_with(definition: serde_json::Value) -> DeclarativeCapabilityRow {
        let now = chrono::Utc::now();
        DeclarativeCapabilityRow {
            id: Uuid::now_v7(),
            org_id: ORG,
            public_id: "cap_01933b5a000070008000000000000001".to_string(),
            name: "research_pack".to_string(),
            display_name: Some("Research Pack".to_string()),
            description: "Adds research instructions.".to_string(),
            status: "active".to_string(),
            definition,
            created_at: now,
            updated_at: now,
            archived_at: None,
            deleted_at: None,
        }
    }

    fn valid_definition() -> serde_json::Value {
        json!({
            "name": "research_pack",
            "description": "Adds research instructions.",
            "system_prompt": "Use the research workflow.",
            "dependencies": ["web_search"],
        })
    }

    /// The three shapes the issue names, plus the two non-object rows. Each has
    /// to be representable as a failure rather than quietly becoming `Default`.
    fn unparseable_definitions() -> Vec<(&'static str, serde_json::Value)> {
        vec![
            // Missing a required field.
            ("missing_description", json!({ "name": "research_pack" })),
            // Unknown enum variant — the shape EVE-1034 produces for legacy
            // rows once a new required discriminant lands.
            (
                "unknown_status_variant",
                json!({
                    "name": "research_pack",
                    "description": "Adds research instructions.",
                    "status": "not_a_real_status",
                }),
            ),
            // Right field, wrong type.
            (
                "wrong_field_type",
                json!({
                    "name": "research_pack",
                    "description": "Adds research instructions.",
                    "skills": "web-research",
                }),
            ),
            ("null_definition", serde_json::Value::Null),
            ("not_an_object", json!("research_pack")),
        ]
    }

    #[test]
    fn an_unparseable_definition_is_a_failure_rather_than_a_default() {
        for (label, definition) in unparseable_definitions() {
            let row = row_with(definition);
            assert!(
                deserialize_persisted_definition(row.id, &row.name, &row.definition).is_err(),
                "{label} should not parse"
            );
        }
    }

    #[test]
    fn an_unparseable_capability_never_reports_available() {
        for (label, definition) in unparseable_definitions() {
            let capability = row_to_declarative_capability(&row_with(definition));

            assert_eq!(
                capability.definition.status,
                everruns_core::CapabilityStatus::Retired,
                "{label} must not present as usable"
            );
            assert!(
                !capability.definition.status.is_active(),
                "{label} must contribute nothing"
            );
            // The row is `active`, so the resource status still reads active —
            // what changes is that the definition is inert and says why.
            assert!(
                capability.definition_error.is_some(),
                "{label} must surface the parse error"
            );
            // Identity survives so a surface can name what is broken.
            assert_eq!(capability.name, "research_pack");
            assert_eq!(capability.description, "Adds research instructions.");
        }
    }

    /// The regression risk: every capability read goes through this path.
    #[test]
    fn a_valid_definition_is_untouched() {
        let capability = row_to_declarative_capability(&row_with(valid_definition()));

        assert_eq!(capability.definition_error, None);
        assert_eq!(
            capability.definition.status,
            everruns_core::CapabilityStatus::Available
        );
        assert_eq!(
            capability.definition.system_prompt.as_deref(),
            Some("Use the research workflow.")
        );
        assert_eq!(capability.definition.dependencies, vec!["web_search"]);
        assert_eq!(capability.name, "research_pack");
    }

    #[test]
    fn a_disabled_row_is_still_retired_without_a_parse_error() {
        let mut row = row_with(valid_definition());
        row.status = "disabled".to_string();

        let capability = row_to_declarative_capability(&row);

        assert_eq!(
            capability.definition.status,
            everruns_core::CapabilityStatus::Retired
        );
        // Deliberately disabled is not corrupt, and must not be reported as it.
        assert_eq!(capability.definition_error, None);
    }

    /// The whole point of EVE-1079: these two used to be indistinguishable.
    #[test]
    fn a_genuinely_empty_capability_is_distinguishable_from_a_broken_one() {
        let empty = row_to_declarative_capability(&row_with(json!({
            "name": "research_pack",
            "description": "Adds research instructions.",
        })));
        let broken = row_to_declarative_capability(&row_with(json!({
            "name": "research_pack",
        })));

        assert_eq!(empty.definition_error, None);
        assert_eq!(
            empty.definition.status,
            everruns_core::CapabilityStatus::Available
        );
        assert!(empty.definition.skills.is_empty());

        assert!(broken.definition_error.is_some());
        assert_eq!(
            broken.definition.status,
            everruns_core::CapabilityStatus::Retired
        );
    }

    #[test]
    fn reading_an_unparseable_row_does_not_rewrite_it() {
        let stored = json!({ "name": "research_pack" });
        let row = row_with(stored.clone());

        let _ = row_to_declarative_capability(&row);
        let _ = deserialize_persisted_definition(row.id, &row.name, &row.definition);

        // A failed read is diagnosable only while the original bytes survive.
        assert_eq!(row.definition, stored);
    }

    async fn store_with(definition: serde_json::Value) -> StorageBackend {
        let db = StorageBackend::in_memory();
        db.create_declarative_capability(
            ORG,
            CreateDeclarativeCapabilityRow {
                public_id: "cap_01933b5a000070008000000000000001".to_string(),
                name: "research_pack".to_string(),
                display_name: Some("Research Pack".to_string()),
                description: "Adds research instructions.".to_string(),
                definition,
            },
        )
        .await
        .unwrap();
        db
    }

    #[tokio::test]
    async fn hydration_fails_closed_on_an_unparseable_definition() {
        for (label, definition) in unparseable_definitions() {
            let db = store_with(definition).await;
            let configs = vec![AgentCapabilityConfig::new("declarative:research_pack")];

            let error = hydrate_declarative_capability_configs(&db, ORG, configs)
                .await
                .expect_err("hydration must not silently contribute nothing");

            // Named cause: the operator has to be able to tell which row.
            let rendered = error.to_string();
            assert!(
                rendered.contains("research_pack"),
                "{label} error must name the capability, got: {rendered}"
            );
            assert!(
                rendered.contains("cannot be parsed"),
                "{label} error must name the cause, got: {rendered}"
            );
        }
    }

    #[tokio::test]
    async fn hydration_is_unaffected_by_a_valid_definition() {
        let db = store_with(valid_definition()).await;
        let configs = vec![AgentCapabilityConfig::new("declarative:research_pack")];

        let hydrated = hydrate_declarative_capability_configs(&db, ORG, configs)
            .await
            .expect("a valid definition still hydrates");

        assert_eq!(hydrated.len(), 1);
        assert_eq!(
            hydrated[0].capability_id().to_string(),
            "declarative:research_pack"
        );
    }

    #[tokio::test]
    async fn an_unrelated_capability_is_passed_through_untouched() {
        let db = store_with(valid_definition()).await;
        let configs = vec![AgentCapabilityConfig::new("web_search")];

        let hydrated = hydrate_declarative_capability_configs(&db, ORG, configs)
            .await
            .unwrap();

        assert_eq!(hydrated.len(), 1);
        assert_eq!(hydrated[0].capability_id().to_string(), "web_search");
    }
}

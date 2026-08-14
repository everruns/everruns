// Harness query helpers — shared by commands, gRPC, and other domains.
//
// No policy checks, no input validation. Pure data access + mapping.

use crate::domains::common::CommandError;
use crate::errors::ResourceNotFoundError;
use crate::kernel_imports::{
    AgentCapabilityConfig, InitialFile, everruns_provider::typed_id::HarnessId,
    is_declarative_capability,
};
use crate::storage::StorageBackend;
use everruns_platform::{Harness, HarnessStatus, merge_harness};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

use super::types::HarnessRow;

// ============================================================================
// Row mapping
// ============================================================================

pub fn row_to_harness(row: HarnessRow, capabilities: Vec<AgentCapabilityConfig>) -> Harness {
    Harness {
        id: row.id,
        name: row.name,
        display_name: row.display_name,
        description: row.description,
        system_prompt: row.system_prompt,
        parent_harness_id: row.parent_harness_id,
        default_model_id: row.default_model_id,
        tags: row.tags,
        capabilities,
        initial_files: serde_json::from_value::<Vec<InitialFile>>(row.initial_files)
            .unwrap_or_default(),
        mcp_servers: serde_json::from_value(row.mcp_servers).unwrap_or_default(),
        network_access: row
            .network_access
            .and_then(|v| serde_json::from_value(v).ok()),
        // Harnesses have no per-harness config column for this; the preference
        // is carried at the agent/session layers (EVE-598).
        parallel_tool_calls: None,
        embedder_metadata: serde_json::from_value(row.embedder_metadata).unwrap_or_default(),
        is_built_in: row.is_built_in,
        status: HarnessStatus::from(row.status.as_str()),
        created_at: row.created_at,
        updated_at: row.updated_at,
        archived_at: row.archived_at,
        deleted_at: row.deleted_at,
    }
}

// ============================================================================
// Data access helpers
// ============================================================================

pub async fn get_capabilities(
    db: &StorageBackend,
    org_id: i64,
    harness_id: Uuid,
) -> anyhow::Result<Vec<AgentCapabilityConfig>> {
    let rows = db.get_harness_capabilities(harness_id).await?;
    let capabilities = rows
        .into_iter()
        .map(|row| AgentCapabilityConfig::with_config(row.capability_id, row.config))
        .collect();
    crate::domains::capabilities::queries::hydrate_declarative_capability_configs(
        db,
        org_id,
        capabilities,
    )
    .await
}

/// Resolve harness by public ID or name. Single lookup path for both
/// HTTP (/harnesses/{id_or_name}) and MCP (get_harness --id=...).
pub async fn resolve(
    db: &StorageBackend,
    org_id: i64,
    id_or_name: &str,
) -> anyhow::Result<Option<Harness>> {
    let row = if let Ok(harness_id) = id_or_name.parse::<HarnessId>() {
        db.get_harness(org_id, harness_id).await?
    } else {
        db.get_harness_by_name(org_id, id_or_name).await?
    };
    match row {
        Some(row) if row.status != "deleted" => {
            let caps = get_capabilities(db, row.org_id, row.id.uuid()).await?;
            Ok(Some(row_to_harness(row, caps)))
        }
        _ => Ok(None),
    }
}

/// Load harness by ID, with capabilities.
pub async fn get_by_id(
    db: &StorageBackend,
    org_id: i64,
    id: HarnessId,
) -> anyhow::Result<Option<Harness>> {
    let row = db.get_harness(org_id, id).await?;
    match row {
        Some(row) if row.status != "deleted" => {
            let caps = get_capabilities(db, row.org_id, row.id.uuid()).await?;
            Ok(Some(row_to_harness(row, caps)))
        }
        _ => Ok(None),
    }
}

/// Load harness by name, with capabilities.
pub async fn get_by_name(
    db: &StorageBackend,
    org_id: i64,
    name: &str,
) -> anyhow::Result<Option<Harness>> {
    let row = db.get_harness_by_name(org_id, name).await?;
    match row {
        Some(row) if row.status != "deleted" => {
            let caps = get_capabilities(db, row.org_id, row.id.uuid()).await?;
            Ok(Some(row_to_harness(row, caps)))
        }
        _ => Ok(None),
    }
}

// ============================================================================
// Validation helpers
// ============================================================================

/// Ensure name is not taken by another harness in the org.
pub async fn ensure_name_available(
    db: &StorageBackend,
    org_id: i64,
    name: &str,
    exclude_id: Option<HarnessId>,
) -> Result<(), CommandError> {
    if let Some(existing) = db.get_harness_by_name(org_id, name).await?
        && exclude_id != Some(existing.id)
    {
        return Err(CommandError::conflict(format!(
            "Harness name '{name}' is already taken"
        )));
    }
    Ok(())
}

/// Find a unique slug name, appending -2, -3, etc. if needed.
pub async fn find_unique_name(
    db: &StorageBackend,
    org_id: i64,
    base_name: &str,
) -> anyhow::Result<String> {
    if db.get_harness_by_name(org_id, base_name).await?.is_none() {
        return Ok(base_name.to_string());
    }
    for n in 2..=100 {
        let candidate = format!("{base_name}-{n}");
        if db.get_harness_by_name(org_id, &candidate).await?.is_none() {
            return Ok(candidate);
        }
    }
    anyhow::bail!("Could not find a unique name for '{base_name}'")
}

/// Validate default_model_id exists in the org.
pub async fn validate_model_id(
    db: &StorageBackend,
    org_id: i64,
    model_id: Option<everruns_provider::typed_id::ModelId>,
) -> anyhow::Result<Option<everruns_provider::typed_id::ModelId>> {
    let Some(model_id) = model_id else {
        return Ok(None);
    };
    db.get_model(org_id, model_id.uuid())
        .await?
        .ok_or_else(|| crate::errors::ResourceNotFoundError::new("Model"))?;
    Ok(Some(model_id))
}

/// Ensure file_system capability is present when initial_files are provided.
pub fn ensure_file_system_capability(
    mut caps: Vec<AgentCapabilityConfig>,
    has_files: bool,
) -> Vec<AgentCapabilityConfig> {
    if has_files
        && !caps
            .iter()
            .any(|c| c.capability_id() == "session_file_system")
    {
        caps.insert(0, AgentCapabilityConfig::new("session_file_system"));
    }
    caps
}

/// Build capability tuples for DB storage.
pub fn cap_tuples(caps: &[AgentCapabilityConfig]) -> Vec<(String, i32, serde_json::Value)> {
    caps.iter()
        .enumerate()
        .map(|(i, c)| {
            let config = if is_declarative_capability(c.capability_id()) {
                serde_json::json!({})
            } else {
                c.config_value().clone()
            };
            (c.capability_id().to_string(), i as i32, config)
        })
        .collect()
}

/// Load harnesses list with capabilities from DB rows.
pub async fn load_harnesses_list(
    db: &StorageBackend,
    rows: Vec<HarnessRow>,
) -> anyhow::Result<Vec<Harness>> {
    let Some(org_id) = rows.first().map(|row| row.org_id) else {
        return Ok(Vec::new());
    };
    let harness_ids = rows.iter().map(|row| row.id).collect::<Vec<_>>();
    let capability_rows = db
        .get_harness_capabilities_by_harness_ids(org_id, &harness_ids)
        .await?;
    let mut capabilities_by_harness = HashMap::<HarnessId, Vec<AgentCapabilityConfig>>::new();
    for row in capability_rows {
        capabilities_by_harness
            .entry(row.harness_id)
            .or_default()
            .push(AgentCapabilityConfig::with_config(
                row.capability_id,
                row.config,
            ));
    }

    let mut harnesses = Vec::with_capacity(rows.len());
    for row in rows {
        let caps = capabilities_by_harness.remove(&row.id).unwrap_or_default();
        let caps = crate::domains::capabilities::queries::hydrate_declarative_capability_configs(
            db, row.org_id, caps,
        )
        .await?;
        harnesses.push(row_to_harness(row, caps));
    }
    Ok(harnesses)
}

/// Load a single harness row + capabilities without inheritance merging.
pub async fn load_raw_harness(
    db: &StorageBackend,
    org_id: i64,
    harness_id: HarnessId,
) -> anyhow::Result<Option<Harness>> {
    let Some(row) = db.get_harness(org_id, harness_id).await? else {
        return Ok(None);
    };
    let capabilities = get_capabilities(db, row.org_id, harness_id.uuid()).await?;
    Ok(Some(row_to_harness(row, capabilities)))
}

/// Resolve effective harness by walking the inheritance chain and merging.
pub async fn resolve_effective(
    db: &StorageBackend,
    org_id: i64,
    id: HarnessId,
) -> anyhow::Result<Option<Harness>> {
    let mut visited = HashSet::new();
    let mut chain = Vec::new();
    let mut cursor = Some(id);

    while let Some(current_id) = cursor {
        if !visited.insert(current_id) {
            anyhow::bail!("Harness inheritance cycle detected");
        }
        let Some(harness) = load_raw_harness(db, org_id, current_id).await? else {
            if chain.is_empty() {
                return Ok(None);
            }
            // EVE-437: typed 404 instead of an unclassified anyhow that
            // would map to 500. The inheritance-cycle bail above stays as
            // an `anyhow::bail!` but is mapped to 400 by `classify_anyhow`'s
            // substring list — the cycle is reachable only when the
            // operator-supplied `parent_harness_id` graph contains a loop,
            // so it is a client-input validation failure, not an internal
            // invariant violation.
            return Err(ResourceNotFoundError::new("Parent harness").into());
        };
        cursor = harness.parent_harness_id;
        chain.push(harness);
    }

    let Some(mut effective) = chain.pop() else {
        return Ok(None);
    };
    while let Some(layer) = chain.pop() {
        effective = merge_harness(&effective, &layer);
    }
    Ok(Some(effective))
}

/// Merge a preview layer onto a parent harness.
pub fn merge_preview_layer(
    parent: Option<&Harness>,
    system_prompt: &str,
    capabilities: &[AgentCapabilityConfig],
) -> (String, Vec<AgentCapabilityConfig>) {
    let Some(parent) = parent else {
        return (system_prompt.to_string(), capabilities.to_vec());
    };
    let draft = Harness {
        id: HarnessId::new(),
        name: "preview".to_string(),
        display_name: Some("Preview".to_string()),
        description: None,
        system_prompt: (!system_prompt.trim().is_empty()).then(|| system_prompt.to_string()),
        parent_harness_id: None,
        default_model_id: None,
        tags: vec![],
        capabilities: capabilities.to_vec(),
        initial_files: vec![],
        mcp_servers: Default::default(),
        network_access: None,
        parallel_tool_calls: None,
        embedder_metadata: Default::default(),
        is_built_in: false,
        status: HarnessStatus::Active,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        archived_at: None,
        deleted_at: None,
    };
    let merged = merge_harness(parent, &draft);
    (
        merged.system_prompt.unwrap_or_default(),
        merged.capabilities,
    )
}

/// Check if a harness is built-in (system-managed, readonly).
pub async fn is_built_in(db: &StorageBackend, org_id: i64, id: HarnessId) -> anyhow::Result<bool> {
    let row = db.get_harness(org_id, id).await?;
    Ok(row.map(|r| r.is_built_in).unwrap_or(false))
}

/// Validate parent harness: must exist, be active, and not create a cycle.
pub async fn validate_parent_harness(
    db: &StorageBackend,
    org_id: i64,
    current_harness_id: Option<HarnessId>,
    parent_harness_id: Option<HarnessId>,
) -> anyhow::Result<Option<HarnessId>> {
    let Some(parent_harness_id) = parent_harness_id else {
        return Ok(None);
    };
    if current_harness_id == Some(parent_harness_id) {
        anyhow::bail!("Harness cannot inherit from itself");
    }
    let parent = db
        .get_harness(org_id, parent_harness_id)
        .await?
        .ok_or_else(|| crate::errors::ResourceNotFoundError::new("Parent harness"))?;
    if parent.status != "active" {
        anyhow::bail!("Parent harness must be active");
    }
    if let Some(current_id) = current_harness_id {
        ensure_no_parent_cycle(db, org_id, current_id, parent_harness_id).await?;
    }
    Ok(Some(parent_harness_id))
}

/// Ensure deleting/archiving won't orphan child harnesses.
pub async fn ensure_no_child_harnesses(
    db: &StorageBackend,
    org_id: i64,
    parent_id: HarnessId,
) -> anyhow::Result<()> {
    let children = db.list_child_harnesses(org_id, parent_id).await?;
    if children.is_empty() {
        return Ok(());
    }
    let child_names = children
        .iter()
        .map(|child| child.display_name.as_deref().unwrap_or(&child.name))
        .collect::<Vec<_>>()
        .join(", ");
    anyhow::bail!(
        "Cannot archive or delete harness while child harnesses still inherit from it: {child_names}"
    );
}

/// Ensure deleting/archiving won't break org-level default harness settings.
pub async fn ensure_not_org_default_harness(
    db: &StorageBackend,
    org_id: i64,
    harness_id: HarnessId,
) -> Result<(), CommandError> {
    let settings = db.get_organization_settings(org_id).await?;
    let Some(settings) = settings else {
        return Ok(());
    };
    if settings.default_harness_id == Some(harness_id) {
        return Err(CommandError::conflict(
            "Cannot archive or delete harness while it is the organization default harness",
        ));
    }
    if settings.base_harness_id == Some(harness_id) {
        return Err(CommandError::conflict(
            "Cannot archive or delete harness while it is the organization base harness",
        ));
    }
    Ok(())
}

async fn ensure_no_parent_cycle(
    db: &StorageBackend,
    org_id: i64,
    current_harness_id: HarnessId,
    candidate_parent_id: HarnessId,
) -> anyhow::Result<()> {
    let mut cursor = Some(candidate_parent_id);
    let mut visited = std::collections::HashSet::new();
    while let Some(harness_id) = cursor {
        if harness_id == current_harness_id {
            anyhow::bail!("Harness inheritance cycle detected");
        }
        if !visited.insert(harness_id) {
            anyhow::bail!("Harness inheritance cycle detected");
        }
        let row = db
            .get_harness(org_id, harness_id)
            .await?
            .ok_or_else(|| crate::errors::ResourceNotFoundError::new("Parent harness"))?;
        cursor = row.parent_harness_id;
    }
    Ok(())
}

/// Reserved harness names that cannot be used for user-created harnesses.
pub const RESERVED_HARNESS_NAMES: &[&str] = &["default"];

/// Validate harness name for create/update — standard addressable name rules
/// plus rejection of reserved names.
pub fn validate_harness_name(name: &str) -> Result<(), CommandError> {
    everruns_platform::validate_addressable_name(name)
        .map_err(|msg| CommandError::bad_request(format!("Harness {msg}")))?;
    if RESERVED_HARNESS_NAMES.contains(&name) {
        return Err(CommandError::bad_request(format!(
            "Harness name '{name}' is reserved and cannot be used"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::models::CreateHarnessRow;

    fn harness_row(name: &str, parent_harness_id: Option<HarnessId>) -> CreateHarnessRow {
        CreateHarnessRow {
            name: name.to_string(),
            display_name: Some(name.to_string()),
            description: None,
            system_prompt: None,
            parent_harness_id,
            default_model_id: None,
            tags: vec![],
            initial_files: serde_json::json!([]),
            mcp_servers: serde_json::json!({}),
            network_access: None,
            embedder_metadata: serde_json::json!({}),
            is_built_in: false,
        }
    }

    #[tokio::test]
    async fn list_loads_local_capabilities_in_one_batched_lookup() {
        let db = StorageBackend::in_memory();
        let org_id = 1;
        let root = db
            .create_harness(org_id, harness_row("root", None))
            .await
            .unwrap();
        let child = db
            .create_harness(org_id, harness_row("child", Some(root.id)))
            .await
            .unwrap();
        let _leaf = db
            .create_harness(org_id, harness_row("leaf", Some(child.id)))
            .await
            .unwrap();
        db.set_harness_capabilities(
            child.id.uuid(),
            vec![("web_fetch".to_string(), 0, serde_json::json!({}))],
        )
        .await
        .unwrap();

        let rows = db.list_harnesses(org_id, None, true).await.unwrap();
        db.reset_session_list_lookup_count();
        let harnesses = load_harnesses_list(&db, rows).await.unwrap();

        assert_eq!(db.session_list_lookup_count(), 1);
        let root = harnesses.iter().find(|h| h.name == "root").unwrap();
        let child = harnesses.iter().find(|h| h.name == "child").unwrap();
        let leaf = harnesses.iter().find(|h| h.name == "leaf").unwrap();
        assert_eq!(root.parent_harness_id, None);
        assert_eq!(child.parent_harness_id, Some(root.id));
        assert_eq!(leaf.parent_harness_id, Some(child.id));
        assert!(root.capabilities.is_empty());
        assert_eq!(child.capabilities[0].capability_id(), "web_fetch");
        assert!(leaf.capabilities.is_empty());
    }
}

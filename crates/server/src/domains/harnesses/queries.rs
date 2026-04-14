// Harness query helpers — shared by commands, gRPC, and other domains.
//
// No policy checks, no input validation. Pure data access + mapping.

use crate::domains::common::CommandError;
use crate::storage::StorageBackend;
use everruns_core::{AgentCapabilityConfig, Harness, HarnessId, HarnessStatus, InitialFile};
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
        network_access: row
            .network_access
            .and_then(|v| serde_json::from_value(v).ok()),
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
    harness_id: Uuid,
) -> anyhow::Result<Vec<AgentCapabilityConfig>> {
    let rows = db.get_harness_capabilities(harness_id).await?;
    Ok(rows
        .into_iter()
        .map(|row| AgentCapabilityConfig::with_config(row.capability_id, row.config))
        .collect())
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
            let caps = get_capabilities(db, row.id.uuid()).await?;
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
            let caps = get_capabilities(db, row.id.uuid()).await?;
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
            let caps = get_capabilities(db, row.id.uuid()).await?;
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
    model_id: Option<everruns_core::ModelId>,
) -> anyhow::Result<Option<everruns_core::ModelId>> {
    let Some(model_id) = model_id else {
        return Ok(None);
    };
    db.get_llm_model(org_id, model_id.uuid())
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
        .map(|(i, c)| (c.capability_ref.to_string(), i as i32, c.config.clone()))
        .collect()
}

/// Load harnesses list with capabilities from DB rows.
pub async fn load_harnesses_list(
    db: &StorageBackend,
    rows: Vec<HarnessRow>,
) -> anyhow::Result<Vec<Harness>> {
    let mut harnesses = Vec::with_capacity(rows.len());
    for row in rows {
        let caps = get_capabilities(db, row.id.uuid()).await?;
        harnesses.push(row_to_harness(row, caps));
    }
    Ok(harnesses)
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
    everruns_core::validate_addressable_name(name)
        .map_err(|msg| CommandError::bad_request(format!("Harness {msg}")))?;
    if RESERVED_HARNESS_NAMES.contains(&name) {
        return Err(CommandError::bad_request(format!(
            "Harness name '{name}' is reserved and cannot be used"
        )));
    }
    Ok(())
}

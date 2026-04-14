// Agent query helpers — shared by commands, gRPC, and other domains.
//
// No policy checks, no input validation. Pure data access + mapping.

use crate::domains::common::CommandError;
use crate::storage::StorageBackend;
use everruns_core::typed_id::AgentId;
use everruns_core::{Agent, AgentCapabilityConfig, AgentStatus, InitialFile, TokenUsage};
use uuid::Uuid;

use super::types::AgentRow;

// ============================================================================
// Row mapping
// ============================================================================

pub fn row_to_agent(row: AgentRow, capabilities: Vec<AgentCapabilityConfig>) -> Agent {
    let usage = if row.total_input_tokens > 0 || row.total_output_tokens > 0 {
        Some(TokenUsage::with_cache(
            row.total_input_tokens as u32,
            row.total_output_tokens as u32,
            if row.total_cache_read_tokens > 0 {
                Some(row.total_cache_read_tokens as u32)
            } else {
                None
            },
            if row.total_cache_creation_tokens > 0 {
                Some(row.total_cache_creation_tokens as u32)
            } else {
                None
            },
        ))
    } else {
        None
    };

    let public_id: AgentId = row
        .public_id
        .parse()
        .unwrap_or_else(|_| AgentId::from_uuid(row.id.uuid()));

    Agent {
        public_id,
        internal_id: row.id.uuid(),
        name: row.name,
        display_name: row.display_name,
        description: row.description,
        system_prompt: row.system_prompt,
        default_model_id: row.default_model_id,
        tags: row.tags,
        capabilities,
        initial_files: serde_json::from_value::<Vec<InitialFile>>(row.initial_files)
            .unwrap_or_default(),
        network_access: row
            .network_access
            .and_then(|v| serde_json::from_value(v).ok()),
        max_iterations: row.max_iterations.map(|v| v as usize),
        tools: serde_json::from_value(row.tools).unwrap_or_default(),
        status: AgentStatus::from(row.status.as_str()),
        created_at: row.created_at,
        updated_at: row.updated_at,
        archived_at: row.archived_at,
        deleted_at: row.deleted_at,
        usage,
    }
}

// ============================================================================
// Data access helpers
// ============================================================================

pub async fn get_capabilities(
    db: &StorageBackend,
    agent_uuid: Uuid,
) -> anyhow::Result<Vec<AgentCapabilityConfig>> {
    let rows = db.get_agent_capabilities(agent_uuid).await?;
    Ok(rows
        .into_iter()
        .map(|row| AgentCapabilityConfig::with_config(row.capability_id, row.config))
        .collect())
}

/// Resolve agent by public ID or name. Single lookup path for both
/// HTTP (/agents/{id_or_name}) and MCP (get_agent --id=...).
pub async fn resolve(
    db: &StorageBackend,
    org_id: i64,
    id_or_name: &str,
) -> anyhow::Result<Option<Agent>> {
    let row = if let Ok(agent_id) = id_or_name.parse::<AgentId>() {
        db.get_agent_by_public_id(org_id, &agent_id.to_string())
            .await?
    } else {
        db.get_agent_by_name(org_id, id_or_name).await?
    };
    match row {
        Some(row) if row.status != "deleted" => {
            let caps = get_capabilities(db, row.id.uuid()).await?;
            Ok(Some(row_to_agent(row, caps)))
        }
        _ => Ok(None),
    }
}

/// Load agent by public ID string, with capabilities.
pub async fn get_by_public_id(
    db: &StorageBackend,
    org_id: i64,
    public_id: &str,
) -> anyhow::Result<Option<Agent>> {
    let row = db.get_agent_by_public_id(org_id, public_id).await?;
    match row {
        Some(row) if row.status != "deleted" => {
            let caps = get_capabilities(db, row.id.uuid()).await?;
            Ok(Some(row_to_agent(row, caps)))
        }
        _ => Ok(None),
    }
}

/// Load agent by name, with capabilities.
pub async fn get_by_name(
    db: &StorageBackend,
    org_id: i64,
    name: &str,
) -> anyhow::Result<Option<Agent>> {
    let row = db.get_agent_by_name(org_id, name).await?;
    match row {
        Some(row) if row.status != "deleted" => {
            let caps = get_capabilities(db, row.id.uuid()).await?;
            Ok(Some(row_to_agent(row, caps)))
        }
        _ => Ok(None),
    }
}

// ============================================================================
// Validation helpers
// ============================================================================

/// Ensure name is not taken by another agent in the org.
pub async fn ensure_name_available(
    db: &StorageBackend,
    org_id: i64,
    name: &str,
    exclude_id: Option<AgentId>,
) -> Result<(), CommandError> {
    if let Some(existing) = db.get_agent_by_name(org_id, name).await?
        && exclude_id != Some(existing.id)
    {
        return Err(CommandError::conflict(format!(
            "Agent name '{name}' is already taken"
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
    if db.get_agent_by_name(org_id, base_name).await?.is_none() {
        return Ok(base_name.to_string());
    }
    for n in 2..=100 {
        let candidate = format!("{base_name}-{n}");
        if db.get_agent_by_name(org_id, &candidate).await?.is_none() {
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

/// Load agents list with capabilities from DB rows.
pub async fn load_agents_list(
    db: &StorageBackend,
    rows: Vec<AgentRow>,
) -> anyhow::Result<Vec<Agent>> {
    let mut agents = Vec::with_capacity(rows.len());
    for row in rows {
        let caps = get_capabilities(db, row.id.uuid()).await?;
        agents.push(row_to_agent(row, caps));
    }
    Ok(agents)
}

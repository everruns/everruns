// Agent query helpers — shared by commands, gRPC, and other domains.
//
// No policy checks, no input validation. Pure data access + mapping.

use crate::domains::common::CommandError;
use crate::max_iterations;
use crate::storage::StorageBackend;
use everruns_core::typed_id::{AgentId, AgentVersionId, HarnessId};
use everruns_core::{AgentCapabilityConfig, InitialFile, TokenUsage, is_declarative_capability};
use everruns_platform::{Agent, AgentStatus, AgentVersion, AgentVersionChangeKind};
use uuid::Uuid;

use super::types::AgentRow;
use crate::storage::models::AgentVersionRow;

// ============================================================================
// Row mapping
// ============================================================================

pub fn row_to_agent(row: AgentRow, capabilities: Vec<AgentCapabilityConfig>) -> Agent {
    let usage = if row.total_input_tokens > 0 || row.total_output_tokens > 0 {
        // Actual and estimated cost totals are tracked separately; the aggregate
        // carries each so consumers can prefer actual and reconcile drift.
        Some(
            TokenUsage::with_cache(
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
            )
            .with_cost(
                (row.total_actual_cost_usd > 0.0).then_some(row.total_actual_cost_usd),
                (row.total_estimated_cost_usd > 0.0).then_some(row.total_estimated_cost_usd),
            )
            .with_effective_cost((row.total_cost_usd > 0.0).then_some(row.total_cost_usd)),
        )
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
        harness_id: row.harness_id,
        default_version_id: row.default_version_id,
        forked_from_agent_id: row.forked_from_agent_id,
        forked_from_version_id: row.forked_from_version_id,
        root_agent_id: row.root_agent_id,
        tags: row.tags,
        capabilities,
        initial_files: serde_json::from_value::<Vec<InitialFile>>(row.initial_files)
            .unwrap_or_default(),
        mcp_servers: serde_json::from_value(row.mcp_servers).unwrap_or_default(),
        network_access: row
            .network_access
            .and_then(|v| serde_json::from_value(v).ok()),
        max_iterations: max_iterations::from_db(row.max_iterations),
        parallel_tool_calls: row.parallel_tool_calls,
        tools: serde_json::from_value(row.tools).unwrap_or_default(),
        status: AgentStatus::from(row.status.as_str()),
        created_at: row.created_at,
        updated_at: row.updated_at,
        archived_at: row.archived_at,
        deleted_at: row.deleted_at,
        usage,
    }
}

pub fn row_to_agent_version(row: AgentVersionRow) -> AgentVersion {
    let public_id = row
        .public_id
        .parse::<AgentVersionId>()
        .unwrap_or_else(|_| AgentVersionId::from_uuid(row.id.uuid()));
    AgentVersion {
        public_id,
        internal_id: row.id.uuid(),
        agent_id: row.agent_id,
        version_number: row.version_number,
        semver_major: row.semver_major,
        semver_minor: row.semver_minor,
        semver_patch: row.semver_patch,
        version: row.version,
        is_published: row.is_published,
        parent_version_id: row.parent_version_id,
        source_version_id: row.source_version_id,
        created_by_principal_id: row.created_by_principal_id,
        change_kind: AgentVersionChangeKind::from(row.change_kind.as_str()),
        summary: row.summary,
        config_hash: row.config_hash,
        authored_config: row.authored_config,
        resolved_config: row.resolved_config,
        created_at: row.created_at,
    }
}

pub fn authored_config(agent: &Agent) -> serde_json::Value {
    serde_json::json!({
        "name": agent.name,
        "display_name": agent.display_name,
        "description": agent.description,
        "system_prompt": agent.system_prompt,
        "default_model_id": agent.default_model_id.map(|id| id.to_string()),
        "harness_id": agent.harness_id.to_string(),
        "tags": agent.tags,
        "capabilities": agent.capabilities,
        "initial_files": agent.initial_files,
        "tools": agent.tools,
        "mcp_servers": agent.mcp_servers,
        "network_access": agent.network_access,
        "max_iterations": agent.max_iterations,
        "parallel_tool_calls": agent.parallel_tool_calls,
    })
}

pub fn config_hash(authored_config: &serde_json::Value) -> String {
    use sha2::{Digest, Sha256};
    let bytes = serde_json::to_vec(authored_config).unwrap_or_default();
    format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
}

pub fn version_to_agent(source: &Agent, version: &AgentVersion) -> Agent {
    let cfg = &version.authored_config;
    let resolved = &version.resolved_config;
    let mut agent = source.clone();
    if let Some(value) = cfg.get("name").and_then(|v| v.as_str()) {
        agent.name = value.to_string();
    }
    agent.display_name = cfg
        .get("display_name")
        .and_then(|v| v.as_str())
        .map(ToOwned::to_owned);
    agent.description = cfg
        .get("description")
        .and_then(|v| v.as_str())
        .map(ToOwned::to_owned);
    if let Some(value) = cfg.get("system_prompt").and_then(|v| v.as_str()) {
        agent.system_prompt = value.to_string();
    }
    agent.default_model_id = cfg
        .get("default_model_id")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse().ok());
    if let Some(value) = cfg
        .get("harness_id")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<HarnessId>().ok())
    {
        agent.harness_id = value;
    }
    agent.tags = cfg
        .get("tags")
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    agent.capabilities = cfg
        .get("capabilities")
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    agent.initial_files = cfg
        .get("initial_files")
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    agent.tools = cfg
        .get("tools")
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    agent.mcp_servers = cfg
        .get("mcp_servers")
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    agent.network_access = cfg
        .get("network_access")
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok());
    agent.max_iterations = cfg
        .get("max_iterations")
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok());
    agent.parallel_tool_calls = cfg
        .get("parallel_tool_calls")
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok());

    if let Some(value) = resolved.get("system_prompt").and_then(|v| v.as_str()) {
        agent.system_prompt = value.to_string();
    }
    if let Some(value) = resolved
        .get("tools")
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok())
    {
        agent.tools = value;
    }
    if let Some(value) = resolved
        .get("capabilities")
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok())
    {
        agent.capabilities = value;
    }
    if let Some(value) = resolved
        .get("mcp_servers")
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok())
    {
        agent.mcp_servers = value;
    }
    if let Some(value) = resolved
        .get("default_model_id")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse().ok())
    {
        agent.default_model_id = Some(value);
    }
    if let Some(value) = resolved
        .get("max_iterations")
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok())
    {
        agent.max_iterations = value;
    }
    agent
}

// ============================================================================
// Data access helpers
// ============================================================================

pub async fn get_capabilities(
    db: &StorageBackend,
    org_id: i64,
    agent_uuid: Uuid,
) -> anyhow::Result<Vec<AgentCapabilityConfig>> {
    let rows = db.get_agent_capabilities(agent_uuid).await?;
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
            let caps = get_capabilities(db, row.org_id, row.id.uuid()).await?;
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
            let caps = get_capabilities(db, row.org_id, row.id.uuid()).await?;
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
            let caps = get_capabilities(db, row.org_id, row.id.uuid()).await?;
            Ok(Some(row_to_agent(row, caps)))
        }
        _ => Ok(None),
    }
}

// ============================================================================
// Built-in agent protection
// ============================================================================

/// Whether the agent is platform-supplied. Mirrors the harness equivalent.
///
/// Takes the same `id_or_name` the command received and resolves it the same
/// way [`resolve`] does. Agents use the dual-ID pattern — the API-facing
/// `public_id` is a different value from the internal primary key — so a guard
/// that looked the caller's identifier up as an internal id would find nothing
/// and wave every mutation through.
///
/// A missing agent reports `false` so callers surface their own "not found",
/// rather than a confusing "cannot modify built-in agent".
pub async fn is_built_in(
    db: &StorageBackend,
    org_id: i64,
    id_or_name: &str,
) -> anyhow::Result<bool> {
    let row = if let Ok(agent_id) = id_or_name.parse::<AgentId>() {
        db.get_agent_by_public_id(org_id, &agent_id.to_string())
            .await?
    } else {
        db.get_agent_by_name(org_id, id_or_name).await?
    };
    Ok(row.map(|r| r.is_built_in).unwrap_or(false))
}

/// Reject a mutation that would change a built-in agent's *definition*.
///
/// The line is definition vs bindings, and it is deliberate:
///
/// - **Definition is protected** — prompt, model, capabilities, versions,
///   status. These come from the platform definition, so an org editing them
///   would silently diverge from what the next platform upgrade ships.
/// - **Bindings stay editable** — triggers, identities, credentials, check
///   rules, health checks. These live in adjacent domains and never touch the
///   agents row. A built-in agent nobody can attach a trigger to is a built-in
///   agent nobody can use.
///
/// Call this from `Command::execute`, never from an HTTP route: the MCP
/// endpoint's `execute` tier dispatches the same command descriptors, so a
/// route-level check would leave the built-in agent editable over MCP by any
/// caller holding agent-management access — including the agent itself.
///
/// `CopyAgent` is intentionally not guarded; it is the escape hatch.
pub async fn ensure_not_built_in(
    db: &StorageBackend,
    org_id: i64,
    id_or_name: &str,
    verb: &str,
) -> Result<(), CommandError> {
    if is_built_in(db, org_id, id_or_name).await? {
        return Err(CommandError::bad_request(format!(
            "Cannot {verb} built-in agent. Copy it first to create an editable version."
        )));
    }
    Ok(())
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

/// Load agents list with capabilities from DB rows.
pub async fn load_agents_list(
    db: &StorageBackend,
    rows: Vec<AgentRow>,
) -> anyhow::Result<Vec<Agent>> {
    let mut agents = Vec::with_capacity(rows.len());
    for row in rows {
        let caps = get_capabilities(db, row.org_id, row.id.uuid()).await?;
        agents.push(row_to_agent(row, caps));
    }
    Ok(agents)
}

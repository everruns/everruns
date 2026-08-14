use crate::domains::agents::queries::row_to_agent;
use crate::domains::common::{CommandError, Ctx, classify_anyhow};
use crate::storage::{
    DbSessionScheduleStore, StorageBackend, create_db_session_storage_store,
    create_db_session_storage_store_without_encryption, session_task_store::DbSessionTaskRegistry,
};
use crate::{max_iterations, org_init};
use everruns_core::config_layer::AgentConfigOverlay;
use everruns_core::session_task::SessionTaskRegistry;
use everruns_core::{AgentCapabilityConfig, HarnessId, SessionId, SessionTask, from_json};
use everruns_core::{
    session_services::SessionScheduleStore, session_services::SessionStorageStore,
    tool_context::ToolContext,
};
use everruns_platform::{Harness, HarnessStatus};
use std::collections::HashSet;
use std::sync::Arc;

pub fn parse_session_id(input: &str) -> Result<SessionId, CommandError> {
    input
        .parse()
        .map_err(|e| CommandError::bad_request(format!("Invalid session ID: {e}")))
}

/// Build the registry, attaching the ctx event service when present so API
/// mutations emit task.* events for live UIs.
pub fn registry_for_ctx(ctx: &Ctx) -> DbSessionTaskRegistry {
    let mut registry = DbSessionTaskRegistry::new(ctx.db.clone());
    if let Some(event_service) = &ctx.event_service {
        registry = registry.with_event_emitter(event_service.clone());
    }
    registry
}

/// Build a ToolContext for executor calls from an API command context.
///
/// # Security
///
/// This factory loads the session's effective network ACL by folding the
/// harness chain → agent → session overlays (the same chain the in-turn
/// executor uses) and sets it on the returned ToolContext. Executor calls
/// from the API therefore run under the same network restrictions as in-turn
/// execution. If the ACL cannot be computed (session/agent/harness not found,
/// storage error), this function returns `Err` — callers MUST treat this as
/// "skip executor invocation, log warn" rather than running unconstrained.
///
/// # Other context fields
///
/// - `session_task_registry`: all executors use this for state updates.
/// - `storage_store`: ExternalAgentTaskExecutor reads/writes the AgentRunRecord
///   (KV key `agent_run:{run_id}`) from session storage. The encrypted store is
///   used when an encryption service is configured (matching the worker path);
///   plain KV works either way, only secrets operations need encryption.
/// - `schedule_store`: MonitorTaskExecutor.cancel disables the linked schedule.
/// - `egress_service`: ExternalAgentTaskExecutor rebuilds the A2A client for
///   network calls to the remote agent.
///
/// `platform_store` is not populated — SubagentTaskExecutor needs it but
/// it is not cheaply available from Ctx; deliver/cancel for subagents will
/// return an error that the caller logs as a warn (best-effort, not a failure).
pub async fn tool_context_for_ctx(
    ctx: &Ctx,
    session_id: SessionId,
) -> Result<ToolContext, CommandError> {
    let org_id = ctx.org_id();

    // --- Load session row ---
    let session_row = ctx
        .db
        .get_session(org_id, session_id)
        .await
        .map_err(classify_anyhow)?
        .ok_or_else(|| CommandError::not_found("Session"))?;

    // --- Build session overlay directly from the row ---
    // Only the overlay fields matter here; we avoid row_to_session which
    // panics when harness_id is absent and no fallback is provided.
    let session_overlay = AgentConfigOverlay {
        system_prompt: session_row.system_prompt.clone(),
        capabilities: serde_json::from_value(session_row.capabilities.clone()).unwrap_or_default(),
        initial_files: serde_json::from_value(session_row.initial_files.clone())
            .unwrap_or_default(),
        network_access: session_row
            .network_access
            .as_ref()
            .and_then(|v| serde_json::from_value(v.clone()).ok()),
        default_model_id: session_row.model_id,
        tools: serde_json::from_value(session_row.tools.clone()).unwrap_or_default(),
        max_iterations: max_iterations::from_db(session_row.max_iterations),
        parallel_tool_calls: session_row.parallel_tool_calls,
        mcp_servers: serde_json::from_value(session_row.mcp_servers.clone()).unwrap_or_default(),
    };

    // --- Load harness chain (root-to-leaf) ---
    // Legacy sessions may have NULL harness_id. Resolve the same base-harness
    // fallback used by normal session reconstruction before folding ACLs so
    // executor calls never silently drop the harness policy.
    let harness_id = match session_row.harness_id {
        Some(id) => id,
        None => org_init::base_harness_id(ctx.db.as_ref(), org_id)
            .await
            .map_err(classify_anyhow)?,
    };
    // Mirrors DbHarnessStore::get_harness_chain, using ctx.db directly so
    // both Postgres and InMemory backends are supported.
    let harness_layers: Vec<Harness> = load_harness_chain(&ctx.db, org_id, harness_id).await?;

    // --- Load agent (if any) ---
    let agent_layer: Option<everruns_platform::Agent> = if let Some(agent_id) = session_row.agent_id
    {
        let agent_row = ctx
            .db
            .get_agent(org_id, agent_id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Agent"))?;
        let caps: Vec<AgentCapabilityConfig> = ctx
            .db
            .get_agent_capabilities(agent_id.uuid())
            .await
            .map_err(classify_anyhow)?
            .into_iter()
            .map(|c| AgentCapabilityConfig::with_config(c.capability_id, c.config))
            .collect();
        Some(row_to_agent(agent_row, caps))
    } else {
        None
    };

    // --- Fold overlays to derive effective network ACL ---
    let harness_overlays = harness_layers.iter().map(AgentConfigOverlay::from);
    let agent_overlays = agent_layer.iter().map(AgentConfigOverlay::from);
    let effective = AgentConfigOverlay::fold(
        harness_overlays
            .chain(agent_overlays)
            .chain([session_overlay]),
    );

    // --- Assemble ToolContext ---
    let registry: Arc<dyn SessionTaskRegistry> = Arc::new(registry_for_ctx(ctx));

    let storage_store: Arc<dyn SessionStorageStore> = match ctx.db.as_ref() {
        StorageBackend::InMemory(mem_db) => mem_db.clone() as Arc<dyn SessionStorageStore>,
        StorageBackend::Postgres(db) => {
            if let Some(enc) = &ctx.encryption {
                Arc::new(create_db_session_storage_store(
                    db.clone(),
                    enc.as_ref().clone(),
                )) as Arc<dyn SessionStorageStore>
            } else {
                Arc::new(create_db_session_storage_store_without_encryption(
                    db.clone(),
                )) as Arc<dyn SessionStorageStore>
            }
        }
    };

    let schedule_store: Arc<dyn SessionScheduleStore> =
        Arc::new(DbSessionScheduleStore::new(ctx.db.clone(), org_id));

    let mut tool_ctx = ToolContext::new(session_id)
        .with_session_task_registry(registry)
        .with_storage_store_arc(storage_store)
        .with_schedule_store(schedule_store)
        .with_network_access(effective.network_access);

    if let Some(egress) = &ctx.egress_service {
        tool_ctx = tool_ctx.with_egress_service(egress.clone());
    }

    Ok(tool_ctx)
}

/// Walk the harness parent chain (leaf → root), then reverse to root-to-leaf.
///
/// Mirrors `DbHarnessStore::get_harness_chain`'s cycle protection, but
/// intentionally **fails closed**: where that helper returns an empty chain for
/// a missing leaf, this returns `Err` if the leaf or any referenced parent is
/// missing, so executor calls never silently drop the harness ACL.
async fn load_harness_chain(
    db: &Arc<StorageBackend>,
    org_id: i64,
    start_id: HarnessId,
) -> Result<Vec<Harness>, CommandError> {
    let mut visited: HashSet<HarnessId> = HashSet::new();
    let mut chain: Vec<Harness> = Vec::new();
    let mut cursor: Option<HarnessId> = Some(start_id);

    while let Some(current_id) = cursor {
        if !visited.insert(current_id) {
            return Err(CommandError::internal(anyhow::anyhow!(
                "Harness inheritance cycle detected"
            )));
        }

        let Some(row) = db
            .get_harness(org_id, current_id)
            .await
            .map_err(classify_anyhow)?
        else {
            let kind = if chain.is_empty() {
                "Leaf harness"
            } else {
                "Parent harness"
            };
            return Err(CommandError::internal(anyhow::anyhow!(
                "{kind} not found (org_id={org_id}, harness_id={current_id})"
            )));
        };

        let cap_rows = db
            .get_harness_capabilities(current_id.uuid())
            .await
            .map_err(classify_anyhow)?;
        let capabilities: Vec<AgentCapabilityConfig> = cap_rows
            .into_iter()
            .map(|c| AgentCapabilityConfig::with_config(c.capability_id, c.config))
            .collect();

        cursor = row.parent_harness_id;
        chain.push(Harness {
            id: row.id,
            name: row.name,
            display_name: row.display_name,
            description: row.description,
            system_prompt: row.system_prompt,
            parent_harness_id: row.parent_harness_id,
            default_model_id: row.default_model_id,
            tags: row.tags,
            capabilities,
            initial_files: from_json(row.initial_files),
            mcp_servers: from_json(row.mcp_servers),
            network_access: row
                .network_access
                .and_then(|v| serde_json::from_value(v).ok()),
            // No per-harness config column (EVE-598).
            parallel_tool_calls: None,
            embedder_metadata: serde_json::from_value(row.embedder_metadata).unwrap_or_default(),
            is_built_in: row.is_built_in,
            status: HarnessStatus::from(row.status.as_str()),
            created_at: row.created_at,
            updated_at: row.updated_at,
            archived_at: row.archived_at,
            deleted_at: row.deleted_at,
        });
    }

    // Leaf-to-root → reverse to root-to-leaf for overlay folding
    chain.reverse();
    Ok(chain)
}

/// Validate org ownership of the session. Returns false when the session
/// does not exist in `org_id`.
pub async fn session_in_org(
    db: &Arc<StorageBackend>,
    org_id: i64,
    session_id: SessionId,
) -> anyhow::Result<bool> {
    Ok(db.get_session(org_id, session_id).await?.is_some())
}

/// Look up one task after validating org ownership.
pub async fn get_task_in_org(
    ctx: &Ctx,
    org_id: i64,
    session_id: SessionId,
    task_id: &str,
) -> Result<Option<SessionTask>, CommandError> {
    if !session_in_org(&ctx.db, org_id, session_id)
        .await
        .map_err(crate::domains::common::classify_anyhow)?
    {
        return Ok(None);
    }
    registry_for_ctx(ctx)
        .get(session_id, task_id)
        .await
        .map_err(|e| CommandError::internal(anyhow::anyhow!(e)))
}

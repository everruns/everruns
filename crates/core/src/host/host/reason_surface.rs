//! The model and tool surface a reason step assembles, shared by the reason
//! itself and by callers that need the same surface without running one.
//!
//! Decision: resuming a turn from the log decides which unfinished calls run
//! again by looking each call's tool up in the surface a reason on this host
//! would hand its act. That surface is not the plain assembled context: the
//! reason adds the host's augmented tools (`HostToolAugmentor`) to the MCP
//! tools `load_resolved_turn_for_execution` returns, and hides `query_history`
//! when the executor does not register it. Building both from these helpers
//! keeps them from drifting apart.

use super::message_filter_only::MessageFilterOnlyCapability;
use super::*;

/// Add the host's augmented tools to the MCP tool definitions a reason
/// exposes, as every reason on this host does.
pub(super) async fn augment_reason_tools<A: RuntimeHostAdapter>(
    adapter: &A,
    org_id: i64,
    session_id: SessionId,
    mcp_tool_definitions: &mut Vec<ToolDefinition>,
) -> everruns_contracts::error::Result<()> {
    if let Some(augmentor) = adapter.tool_augmentor() {
        augmentor
            .augment_reason_tools(
                session_id,
                adapter.session_store(org_id),
                adapter.session_task_registry(org_id),
                mcp_tool_definitions,
            )
            .await?;
    }
    Ok(())
}

/// The capability registry a reason assembles its model surface from.
///
/// When the executor does not register `query_history`, its owning
/// capability keeps only its message filter for this turn.
pub(super) fn reason_capability_registry<A: RuntimeHostAdapter>(
    adapter: &A,
    query_history_allowed: bool,
) -> CapabilityRegistry {
    let mut registry = adapter.capability_registry();
    if !query_history_allowed {
        // Persisted history may contain raw text removed by earlier prompt
        // hooks. Preserve the owning capability's message filter while
        // suppressing its prompt/tool contributions for this turn. Tool
        // ownership is discovered through the neutral capability contract;
        // host does not depend on a concrete implementation or capability ID.
        let query_history_owner = registry
            .list()
            .into_iter()
            .find(|capability| {
                capability
                    .tool_definitions()
                    .iter()
                    .any(|tool| tool.name() == "query_history")
            })
            .map(Arc::clone);
        if let Some(capability) = query_history_owner {
            registry.register(MessageFilterOnlyCapability(capability));
        }
    }
    registry
}

/// The turn context a reason of `session_id` on this host would assemble
/// now, without running the reason or its prompt hooks. Its
/// `runtime_agent.tools` is the tool surface that reason hands its act.
pub(crate) async fn reason_turn_context<A: RuntimeHostAdapter>(
    adapter: &A,
    org_id: i64,
    session_id: SessionId,
    input_message_id: MessageId,
) -> everruns_contracts::error::Result<AssembledTurnContext> {
    let session = adapter
        .session_store(org_id)
        .get_session(session_id)
        .await?
        .ok_or_else(|| everruns_contracts::error::AgentLoopError::session_not_found(session_id))?;
    let mut turn_inputs = adapter
        .load_resolved_turn_for_execution(org_id, session_id, input_message_id)
        .await?;
    let executor = load_execution_capabilities(
        adapter,
        org_id,
        session_id,
        turn_inputs.snapshot.harness_id,
        turn_inputs.snapshot.agent_id,
        session.locale.clone(),
        session.blueprint_id.as_deref(),
    )
    .await?;
    let query_history_allowed = executor.tool_registry.get("query_history").is_some();
    augment_reason_tools(
        adapter,
        org_id,
        session_id,
        &mut turn_inputs.mcp_tool_definitions,
    )
    .await?;
    let registry = reason_capability_registry(adapter, query_history_allowed);
    crate::host::runtime_context::assemble_turn_context_from_snapshot(
        turn_inputs.snapshot,
        adapter.message_store().as_ref(),
        adapter.provider_store(org_id).as_ref(),
        &registry,
        &adapter.driver_registry(),
        &turn_inputs.mcp_tool_definitions,
        Some(adapter.file_store(org_id)),
        adapter.storage_store(org_id),
    )
    .await
}

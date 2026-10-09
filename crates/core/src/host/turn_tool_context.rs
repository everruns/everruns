//! Assembling the per-turn tool context an adapter's services hang off.
//!
// Split out of `host.rs` (EVE / #3709): the file is on the size ratchet's
// debt list, and this is the part of it a capability-gated extension touches.

use crate::tool_context::ToolContextServices;
use everruns_contracts::CapabilityRef;
use everruns_contracts::typed_id::{AgentId, SessionId};
use std::sync::Arc;

use crate::ToolRegistry;
use crate::host::SessionMutatorExt;
use crate::host::runtime_host::{RuntimeHostAdapter, ToolContextRequest};
use crate::org_public_id_from_internal;

pub(crate) struct RuntimeToolCapabilityContext {
    pub(crate) subagent_nesting_policy: crate::delegation_services::SubagentNestingPolicy,
    pub(crate) resolved_capabilities: Vec<CapabilityRef>,
}

pub(crate) fn runtime_tool_context_services<A: RuntimeHostAdapter>(
    adapter: &A,
    org_id: i64,
    session_id: SessionId,
    agent_id: Option<AgentId>,
    tool_registry: Option<Arc<ToolRegistry>>,
    mcp_invoker: Option<Arc<dyn crate::McpToolInvoker>>,
    capability_context: RuntimeToolCapabilityContext,
) -> ToolContextServices {
    let extensions = {
        let mut extensions = adapter.tool_context_extensions(ToolContextRequest {
            org_id,
            session_id,
            resolved_capabilities: &capability_context.resolved_capabilities,
        });
        extensions.insert(Arc::new(SessionMutatorExt(adapter.session_mutator(org_id))));
        extensions
    };
    let public_org_id: everruns_contracts::typed_id::OrgId = org_public_id_from_internal(org_id)
        .parse()
        .expect("internal org id converts to valid public org id");
    // Every tenant request leaving through this turn's tools carries its org
    // and session, so the egress boundary can audit and police it per org.
    let egress_service = adapter.egress_service().map(|inner| {
        Arc::new(crate::ScopedEgressService::new(
            inner,
            crate::EgressScope {
                org_id: Some(public_org_id),
                session_id: Some(session_id),
            },
        )) as Arc<dyn crate::EgressService>
    });
    ToolContextServices {
        file_store: Some(adapter.file_store(org_id)),
        storage_store: adapter.storage_store(org_id),
        image_store: adapter.image_artifact_store(org_id),
        provider_credential_store: adapter.provider_credential_store(org_id),
        utility_llm_service: adapter.utility_llm_service(),
        decisions: adapter.decisions(),
        mcp_invoker,
        egress_service,
        message_retriever: Some(adapter.message_store()),
        session_store: Some(adapter.session_store(org_id)),
        agent_store: Some(adapter.agent_store(org_id)),
        connection_resolver: adapter.connection_resolver(),
        schedule_store: adapter.schedule_store(org_id),
        subagent_delegate: adapter.subagent_delegate(org_id, session_id),
        extensions,
        leased_resource_store: adapter.leased_resource_store(org_id),
        session_resource_registry: adapter.session_resource_registry(org_id),
        session_task_registry: adapter.session_task_registry(org_id),
        event_emitter: Some(adapter.event_emitter()),
        capability_registry: Some(adapter.capability_registry()),
        tool_registry,
        org_id: Some(public_org_id),
        network_access: None,
        budget_checker: adapter.budget_checker(org_id, agent_id),
        payment_authority: adapter.payment_authority(org_id, agent_id),
        session_creation_authority: adapter.session_creation_authority(org_id, session_id),
        subagent_spawn_store: adapter.subagent_spawn_store(),
        subagent_nesting_policy: capability_context.subagent_nesting_policy,
        reasoning_effort_handle: adapter.reasoning_effort_handle(session_id),
    }
}

//! The platform services an Engine turn gets, the ones a worker turn gets
//! (data plane v2 step 4).
//!
//! Decisions:
//! - Every service comes from [`WorkerRuntimeHost`], the worker's own host:
//!   budgets, payment, egress, the utility model, hosted MCP, knowledge,
//!   sandboxes and the platform's tool context. One source, so an Engine
//!   turn and a worker turn of the same session cannot be served
//!   differently.
//! - Budgets, payment and hosted MCP are chosen per platform agent. The
//!   Engine names its own agent (derived from the declaration, free of
//!   platform ids), so the platform agent comes from the session's binding
//!   and the Engine's argument is ignored.
//! - The tool context gates platform tools on the declared capabilities, the
//!   list the agent's revision names.
//! - The platform's person-facing calls (an approval prompt, a URL
//!   elicitation consent) wait on a person on the Engine too, so a turn
//!   resumed after a crash asks again instead of settling them as lost.
//! - Long tool calls detach as on the worker host without a slot here: the
//!   lease's task provides the worker's detacher (`tool_detach::provide`) and
//!   the Engine activates in that task.

use std::sync::Arc;

use everruns_contracts::CapabilityRef;
use everruns_contracts::typed_id::{AgentId, SessionId};

use crate::host::{HostBackends, HostExtensions, RuntimeHostAdapter, ToolContextRequest};
use crate::runtime_host::WorkerRuntimeHost;
use crate::worker_adapters::WorkerAdapters;

/// One Engine activation's platform services.
pub struct PlatformTurn {
    pub org_id: i64,
    pub session_id: SessionId,
    /// The platform agent budgets and MCP servers are chosen by.
    pub agent_id: Option<AgentId>,
    /// The revision's declared capabilities.
    pub capabilities: Vec<CapabilityRef>,
}

/// `backends` with the platform services of `turn`.
pub fn with_platform_services<A: WorkerAdapters>(
    backends: HostBackends,
    adapters: &A,
    turn: PlatformTurn,
) -> HostBackends {
    let host = WorkerRuntimeHost::new(adapters.clone());
    let PlatformTurn {
        org_id: org,
        session_id: session,
        agent_id: agent,
        capabilities,
    } = turn;
    let mut extensions = HostExtensions::default();

    extensions.composition.egress = host.egress_service();
    extensions.composition.utility_llm = host.utility_llm_service();
    extensions.composition.decisions = host.decisions();

    let budget = host.clone();
    extensions.authorities.budget = Some(Arc::new(move |_| budget.budget_checker(org, agent)));
    let payment = host.clone();
    extensions.authorities.payment = Some(Arc::new(move |_| payment.payment_authority(org, agent)));
    extensions.authorities.session_creation = host.session_creation_authority(org, session);
    extensions.authorities.outbound_rate_limiter = host.outbound_tool_rate_limiter(org);

    extensions.stores.provider_credentials = host.provider_credential_store(org);
    extensions.stores.leased_resources = host.leased_resource_store(org);
    extensions.stores.durable_tool_results = host.durable_tool_result_store();
    extensions.stores.session_resources = host.session_resource_registry(org);
    extensions.stores.subagent_spawns = host.subagent_spawn_store();
    extensions.stores.images = host.image_resolver(org);
    extensions.stores.files = host.file_resolver(org);
    extensions.stores.image_artifacts = host.image_artifact_store(org);

    let mcp = host.clone();
    extensions.hooks.hosted_mcp = Some(Arc::new(move |_, input| {
        mcp.hosted_mcp_resolver(org, session, agent, input)
    }));
    extensions.hooks.knowledge = adapters.knowledge_runtime(org);
    extensions.hooks.waits_on_person = Some(Arc::new(crate::actor_runtime::waits_on_person));

    let tools = host;
    backends
        .with_extensions(extensions)
        .with_tool_context_extensions_factory(Arc::new(move |org_id, session_id| {
            tools.tool_context_extensions(ToolContextRequest {
                org_id,
                session_id,
                resolved_capabilities: &capabilities,
            })
        }))
}

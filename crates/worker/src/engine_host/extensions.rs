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
//! - Subagents, session tasks, schedules, connections and shell hooks come
//!   from the worker host too. The platform's turn tools (a child session's
//!   report tools, a thread's worker tools) look up other sessions, its
//!   parent and the parent's tasks, so they read the platform's session and
//!   file stores ([`PlatformTools`]), never the Engine's, which holds only
//!   this session's id-free record.

use std::sync::Arc;

use async_trait::async_trait;
use everruns_capabilities::{PlatformStoreSubagentDelegate, PlatformToolAugmentor};
use everruns_contracts::CapabilityRef;
use everruns_contracts::error::Result;
use everruns_contracts::tool_types::ToolDefinition;
use everruns_contracts::typed_id::{AgentId, SessionId};

use crate::core::execution_loading::SessionStore;
use crate::core::session_files::SessionFileSystem;
use crate::core::session_task::SessionTaskRegistry;
use crate::core::tools::ToolRegistry;
use crate::host::{
    HostBackends, HostExtensions, HostToolAugmentor, RuntimeHostAdapter, ToolContextRequest,
};
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

    let mut backends = backends;
    if let Some(registry) = host.session_task_registry(org) {
        backends = backends.with_session_task_registry(registry);
    }
    if let Some(resolver) = host.connection_resolver(org) {
        backends = backends.with_connection_resolver(resolver);
    }
    if let Some(store) = host.native_async_store() {
        backends = backends.with_native_async_store(store);
    }
    let schedules = adapters.clone();
    let delegate = adapters.clone();
    let hooks = host.clone();
    let tools = host.clone();
    backends
        .with_extensions(extensions)
        .with_schedule_store_factory(Arc::new(move |org_id| schedules.schedule_store(org_id)))
        .with_subagent_delegate_factory(Arc::new(move |org_id, session_id| {
            Arc::new(PlatformStoreSubagentDelegate(
                delegate.platform_store(org_id, session_id),
            ))
        }))
        .with_bash_hook_dispatcher_factory(Arc::new(move |_| hooks.bash_hook_dispatcher(org)))
        .with_tool_augmentor(Arc::new(PlatformTools { host, org_id: org }))
        .with_tool_context_extensions_factory(Arc::new(move |org_id, session_id| {
            tools.tool_context_extensions(ToolContextRequest {
                org_id,
                session_id,
                resolved_capabilities: &capabilities,
            })
        }))
}

/// The platform's turn tools over the platform's session and file stores.
struct PlatformTools<A: WorkerAdapters> {
    host: WorkerRuntimeHost<A>,
    org_id: i64,
}

#[async_trait]
impl<A: WorkerAdapters> HostToolAugmentor for PlatformTools<A> {
    fn disabled_hook_contributions(
        &self,
        capability_id: &str,
        config: &serde_json::Value,
    ) -> Vec<String> {
        PlatformToolAugmentor.disabled_hook_contributions(capability_id, config)
    }

    async fn augment_reason_tools(
        &self,
        session_id: SessionId,
        _engine_sessions: Arc<dyn SessionStore>,
        task_registry: Option<Arc<dyn SessionTaskRegistry>>,
        definitions: &mut Vec<ToolDefinition>,
    ) -> Result<()> {
        PlatformToolAugmentor
            .augment_reason_tools(
                session_id,
                self.host.session_store(self.org_id),
                task_registry,
                definitions,
            )
            .await
    }

    async fn augment_act_tools(
        &self,
        session_id: SessionId,
        _engine_sessions: Arc<dyn SessionStore>,
        task_registry: Option<Arc<dyn SessionTaskRegistry>>,
        _engine_files: Arc<dyn SessionFileSystem>,
        requested_definitions: &[ToolDefinition],
        registry: &mut ToolRegistry,
    ) -> Result<()> {
        PlatformToolAugmentor
            .augment_act_tools(
                session_id,
                self.host.session_store(self.org_id),
                task_registry,
                self.host.file_store(self.org_id),
                requested_definitions,
                registry,
            )
            .await
    }
}

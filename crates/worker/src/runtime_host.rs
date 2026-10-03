// Runtime-host adapter bridge for durable/server-backed workers.
// Decision: everruns-worker exposes first-party adapters from WorkerAdapters to
// the neutral everruns-host execution contract.

use crate::core::tool_context::ToolContextExtensions;
use crate::core::{
    CapabilityRegistry, EgressService, ResolvedExecutionSnapshot, SessionExecutionState,
    UtilityLlmService,
};
use crate::core::{
    connection_services::ProviderCredentialStore, delegation_services::SessionCreationAuthority,
    event_emitter::EventEmitter, execution_loading::AgentStore, execution_loading::HarnessStore,
    execution_loading::SessionStore, file_services::FileResolver,
    image_services::ImageArtifactStore, image_services::ImageResolver,
    provider_resolution::ProviderStore, session_files::SessionFileSystem,
    tool_execution::PaymentAuthority,
};
use crate::host::{ResolvedTurnInputs, RuntimeHostAdapter, ToolContextRequest};
use crate::mcp::{
    McpClient, McpConnection, McpConnectionResolver, McpEndpoint, McpExecutor, NoAuthProvider,
};
use async_trait::async_trait;
use everruns_capabilities::SessionMutator;
use everruns_capabilities::capabilities::PLATFORM_CAPABILITY_ID;
use everruns_capabilities::{
    DurableToolResultStoreExt, KnowledgeIndexSearchExt, KnowledgeStoreExt, PlatformStoreExt,
    PlatformStoreSubagentDelegate, PlatformToolAugmentor, SandboxCheckpointStoreExt,
    SandboxStateStoreExt, SessionSqlDbStoreExt,
};
use everruns_contracts::driver_registry::DriverRegistry;
use everruns_contracts::error::Result;
use everruns_contracts::tool_types::{ConnectionRequired, ConnectionRequiredSubject};
use everruns_contracts::typed_id::{AgentId, MessageId, SessionId};
use std::sync::Arc;
use uuid::Uuid;

use crate::phase_reads::PhaseReads;
use crate::worker_adapters::{OrgAdapter, SessionAdapter, WorkerAdapters};
use crate::write_behind::WriteBehind;

/// Resolves an `mcp_*` server prefix to a connection by asking the control
/// plane over gRPC (`get_mcp_server_by_prefix`). The control plane returns a
/// fully resolved descriptor.
///
/// Credential handling:
/// - **API-key** servers carry the decrypted key, which we bake in as a
///   `Bearer` Authorization header.
/// - **OAuth** servers resolve the session's connection token for
///   `oauth_provider_id` via the host's `UserConnectionResolver` and bake it in
///   as a `Bearer` header. Which lookup is used depends on the attachment:
///   - An attachment declaring an acting identity (`actsAs` of `service` or
///     `user`) uses `get_mcp_connection_token`, which reads exactly one
///     connection store and never falls back to another (EVE-1029).
///   - An attachment declaring none — legacy org-level MCP servers and inline
///     scoped entries — keeps `get_connection_token`, whose identity-preferring
///     lookup is unchanged, so configs predating `actsAs` behave as they did.
///
///   When no token resolves, *or the lookup errors*, the connection is marked
///   `pending_oauth_provider` so the executor returns a `connection_required`
///   tool result instead of issuing an unauthenticated request that a
///   permissive server might accept.
/// - The client uses `NoAuthProvider`, so auth is always expressed via
///   `headers`.
#[derive(Clone)]
struct WorkerMcpResolver<A: WorkerAdapters> {
    input_message_id: Option<Uuid>,
    adapters: A,
    org_id: i64,
    session_id: Uuid,
    agent_id: Option<AgentId>,
}

fn pending_oauth_connection(
    provider: &str,
    acts_as: crate::core::McpServerActsAs,
    agent_id: Option<AgentId>,
) -> anyhow::Result<ConnectionRequired> {
    match (acts_as, agent_id) {
        (crate::core::McpServerActsAs::Service, Some(agent_id)) => {
            Ok(ConnectionRequired::with_setup(
                provider,
                ConnectionRequiredSubject::Agent,
                format!("/agents/{agent_id}?tab=mcp"),
            ))
        }
        (crate::core::McpServerActsAs::Service, None) => {
            anyhow::bail!("MCP service attachment requires an agent")
        }
        (crate::core::McpServerActsAs::User, _) => Ok(ConnectionRequired::with_setup(
            provider,
            ConnectionRequiredSubject::User,
            "/settings/connections",
        )),
        _ => Ok(ConnectionRequired::provider_only(provider)),
    }
}

#[async_trait]
impl<A: WorkerAdapters> McpConnectionResolver for WorkerMcpResolver<A> {
    fn for_execution(&self, id: Uuid) -> Option<Arc<dyn McpConnectionResolver>> {
        let mut bound = self.clone();
        bound.input_message_id = Some(id);
        Some(Arc::new(bound))
    }

    async fn resolve(&self, server_prefix: &str) -> anyhow::Result<Option<McpConnection>> {
        let info = match self.input_message_id {
            Some(id) => {
                self.adapters
                    .get_mcp_server_for_execution(self.org_id, self.session_id, server_prefix, id)
                    .await
            }
            None => {
                self.adapters
                    .get_mcp_server_by_prefix(self.org_id, Some(self.session_id), server_prefix)
                    .await
            }
        }
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;

        let mut headers = info.headers;
        let has_authorization = |headers: &std::collections::HashMap<String, String>| {
            headers
                .keys()
                .any(|k| k.eq_ignore_ascii_case("authorization"))
        };
        if let Some(api_key) = info.api_key
            && !has_authorization(&headers)
        {
            headers.insert("Authorization".to_string(), format!("Bearer {api_key}"));
        }

        let mut pending_oauth_provider = None;
        if info.auth_mode == crate::core::McpServerAuthMode::OAuth
            && !has_authorization(&headers)
            && let Some(provider) = info.oauth_provider_id.as_deref()
        {
            // Every OAuth lookup uses the attachment's verified acting identity.
            let base = self.adapters.connection_resolver();
            let resolver = self
                .input_message_id
                .and_then(|id| base.for_execution(id))
                .unwrap_or(base);
            let resolver = resolver
                .for_mcp_operation(server_prefix)
                .unwrap_or(resolver);
            let resolved = resolver
                .get_mcp_connection_token(self.session_id.into(), provider, info.acts_as)
                .await;

            match resolved {
                Ok(Some(token)) => {
                    headers.insert("Authorization".to_string(), format!("Bearer {token}"));
                }
                Ok(None) => {
                    pending_oauth_provider = Some(pending_oauth_connection(
                        provider,
                        info.acts_as,
                        self.agent_id,
                    )?);
                }
                Err(error) => {
                    tracing::warn!(
                        server = %info.name,
                        provider,
                        %error,
                        "failed to resolve MCP OAuth connection token"
                    );
                    pending_oauth_provider = Some(pending_oauth_connection(
                        provider,
                        info.acts_as,
                        self.agent_id,
                    )?);
                }
            }
        }

        Ok(Some(McpConnection {
            name: info.name,
            endpoint: McpEndpoint::Http {
                url: info.url,
                headers,
            },
            auth_mode: info.auth_mode,
            protocol_mode: info.protocol_mode,
            elicitation_policy: info.elicitation_policy,
            oauth_provider_id: info.oauth_provider_id,
            pending_oauth_provider,
            secret_bindings: info.secret_bindings,
        }))
    }

    async fn invalidate(
        &self,
        server_prefix: &str,
        rejected_connection: &McpConnection,
    ) -> anyhow::Result<()> {
        let info = match self.input_message_id {
            Some(id) => {
                self.adapters
                    .get_mcp_server_for_execution(self.org_id, self.session_id, server_prefix, id)
                    .await
            }
            None => {
                self.adapters
                    .get_mcp_server_by_prefix(self.org_id, Some(self.session_id), server_prefix)
                    .await
            }
        }
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        if info.auth_mode != crate::core::McpServerAuthMode::OAuth
            || info.acts_as == crate::core::McpServerActsAs::None
        {
            return Ok(());
        }
        let Some(provider) = info.oauth_provider_id.as_deref() else {
            return Ok(());
        };
        let rejected_credential_fingerprint = match &rejected_connection.endpoint {
            McpEndpoint::Http { headers, .. } => headers
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))
                .and_then(|(_, value)| value.split_once(' '))
                .filter(|(scheme, token)| {
                    scheme.eq_ignore_ascii_case("bearer") && !token.is_empty()
                })
                .map(|(_, token)| everruns_internal_protocol::credential_fingerprint(token)),
            #[allow(unreachable_patterns)]
            _ => None,
        };
        let Some(rejected_credential_fingerprint) = rejected_credential_fingerprint else {
            return Ok(());
        };
        let base = self.adapters.connection_resolver();
        let resolver = self
            .input_message_id
            .and_then(|id| base.for_execution(id))
            .unwrap_or(base);
        let resolver = resolver
            .for_mcp_operation(server_prefix)
            .unwrap_or(resolver);
        resolver
            .invalidate_mcp_connection(
                self.session_id.into(),
                provider,
                info.acts_as,
                &rejected_credential_fingerprint,
            )
            .await
            .map_err(|e| anyhow::anyhow!(e.to_string()))
    }
}

/// First-party adapter from worker backends into `everruns-host` execution.
///
/// This is the bridge that lets durable workers execute the shared runtime
/// host phases without depending on in-process-only stores.
///
/// ```ignore
/// use crate::host::execute_reason_activity;
/// use everruns_worker::{GrpcWorkerAdapters, WorkerRuntimeHost};
///
/// let adapters = GrpcWorkerAdapters::connect("127.0.0.1:9001").await?;
/// let host = WorkerRuntimeHost::new(adapters);
/// let result = execute_reason_activity(&host, org_id, reason_input).await?;
/// # Ok::<(), everruns_contracts::error::AgentLoopError>(())
/// ```
#[derive(Clone)]
pub struct WorkerRuntimeHost<A: WorkerAdapters> {
    adapters: A,
    cancellation: Option<tokio::sync::watch::Receiver<bool>>,
    /// Explicit turn cancel only; `cancellation` also fires on ownership loss.
    cancel_requested: Option<tokio::sync::watch::Receiver<bool>>,
    event_metadata: Option<serde_json::Map<String, serde_json::Value>>,
    reads: PhaseReads,
    write_behind: WriteBehind,
}

impl<A: WorkerAdapters> WorkerRuntimeHost<A> {
    /// Wait for the events a phase queued to be stored (see `write_behind`).
    /// Call after the phase, so nothing emitted later lands before them.
    pub async fn flush_events(&self) {
        self.write_behind.flush().await;
    }

    /// Start the phase's setup reads now, concurrently (see `phase_reads`).
    pub fn prefetching(self, ids: Option<crate::phase_reads::PhaseIds>) -> Self {
        if let Some(ids) = ids {
            self.reads.prefetch(&self.adapters, ids);
        }
        self
    }

    pub fn with_turn_cancellation(
        mut self,
        cancellation: tokio::sync::watch::Receiver<bool>,
        cancel_requested: tokio::sync::watch::Receiver<bool>,
    ) -> Self {
        self.cancellation = Some(cancellation);
        self.cancel_requested = Some(cancel_requested);
        self
    }
    pub fn new(adapters: A) -> Self {
        Self::with_event_metadata(adapters, None)
    }

    pub fn with_event_metadata(
        adapters: A,
        metadata: Option<serde_json::Map<String, serde_json::Value>>,
    ) -> Self {
        Self {
            adapters,
            cancellation: None,
            cancel_requested: None,
            event_metadata: metadata,
            reads: PhaseReads::new(),
            write_behind: WriteBehind::new(),
        }
    }
}

#[async_trait]
impl<A: WorkerAdapters> RuntimeHostAdapter for WorkerRuntimeHost<A> {
    fn turn_cancellation(&self) -> Option<tokio::sync::watch::Receiver<bool>> {
        self.cancellation.clone()
    }
    fn turn_cancel_requested(&self) -> Option<tokio::sync::watch::Receiver<bool>> {
        self.cancel_requested.clone()
    }
    async fn set_session_status(
        &self,
        org_id: i64,
        session_id: SessionId,
        status: SessionExecutionState,
    ) -> Result<()> {
        // Status mutation is an acknowledged effect end to end (EVE-882):
        // neither the adapter nor the host contract exposes a session record.
        self.reads.invalidate_session(org_id, session_id.uuid());
        self.adapters
            .set_session_status(org_id, session_id.uuid(), &status.to_string())
            .await?;
        Ok(())
    }

    async fn load_resolved_turn(
        &self,
        org_id: i64,
        session_id: SessionId,
    ) -> Result<ResolvedTurnInputs> {
        self.load_resolved_turn_for_execution(
            org_id,
            session_id,
            everruns_contracts::typed_id::MessageId::from_uuid(Uuid::nil()),
        )
        .await
    }
    async fn load_resolved_turn_for_execution(
        &self,
        org_id: i64,
        session_id: SessionId,
        input_message_id: everruns_contracts::typed_id::MessageId,
    ) -> Result<ResolvedTurnInputs> {
        // The control plane projects batched records into portable execution
        // inputs before this worker boundary (EVE-872). The harness arrives
        // pre-merged, so the effective definition folds identically.
        let context = if input_message_id.uuid().is_nil() {
            self.adapters
                .load_turn_context(org_id, session_id.uuid())
                .await?
        } else {
            let message_id = input_message_id.uuid();
            self.reads
                .turn_context(&self.adapters, org_id, session_id.uuid(), message_id)
                .await?
        };
        // Loading seam (EVE-877/EVE-881): seed the already validated, pinned
        // batch agent and session. Later setup reads reuse these definitions.
        let (reads, agent) = (&self.reads, context.agent.as_ref());
        reads.seed(org_id, &context.session, agent);
        let harness_definition = self
            .reads
            .harness(&self.adapters, org_id, context.session.harness_id.uuid())
            .await?
            .ok_or_else(|| {
                everruns_contracts::error::AgentLoopError::harness_not_found(
                    context.session.harness_id,
                )
            })?;
        let agent_definition = context.agent.as_ref();
        let snapshot = ResolvedExecutionSnapshot::project(
            &harness_definition,
            agent_definition,
            &context.session,
        )?;
        if let Some(model_id) = snapshot.default_model_id {
            self.reads
                .prefetch_model(&self.adapters, org_id, model_id.uuid(), session_id);
        }
        Ok(ResolvedTurnInputs {
            snapshot,
            messages: context.messages,
            mcp_tool_definitions: context.mcp_tool_definitions,
        })
    }

    fn capability_registry(&self) -> CapabilityRegistry {
        self.adapters.capability_registry()
    }

    fn driver_registry(&self) -> DriverRegistry {
        self.adapters.driver_registry()
    }

    fn harness_store(&self, org_id: i64) -> Arc<dyn HarnessStore> {
        Arc::new(OrgAdapter::new(self.adapters.clone(), org_id).with_reads(self.reads.clone()))
    }

    fn agent_store(&self, org_id: i64) -> Arc<dyn AgentStore> {
        Arc::new(OrgAdapter::new(self.adapters.clone(), org_id).with_reads(self.reads.clone()))
    }

    fn session_store(&self, org_id: i64) -> Arc<dyn SessionStore> {
        Arc::new(OrgAdapter::new(self.adapters.clone(), org_id).with_reads(self.reads.clone()))
    }

    fn session_mutator(&self, org_id: i64) -> Arc<dyn SessionMutator> {
        Arc::new(OrgAdapter::new(self.adapters.clone(), org_id).with_reads(self.reads.clone()))
    }

    fn provider_store(&self, org_id: i64) -> Arc<dyn ProviderStore> {
        Arc::new(OrgAdapter::new(self.adapters.clone(), org_id).with_reads(self.reads.clone()))
    }

    fn message_store(&self) -> Arc<dyn crate::core::MessageRetriever> {
        Arc::new(SessionAdapter::new(self.adapters.clone()))
    }

    fn native_async_store(
        &self,
    ) -> Option<Arc<dyn crate::core::native_async_store::NativeAsyncStore>> {
        self.adapters.native_async_store()
    }

    fn agents_api_store(&self) -> Option<Arc<dyn crate::core::agents_api_store::AgentsApiStore>> {
        self.adapters.agents_api_store()
    }

    fn compaction_checkpoint_store(
        &self,
    ) -> Option<Arc<dyn crate::core::CompactionCheckpointStore>> {
        self.adapters.compaction_checkpoint_store()
    }

    fn event_emitter(&self) -> Arc<dyn EventEmitter> {
        Arc::new(
            SessionAdapter::new(self.adapters.clone())
                .with_event_metadata(self.event_metadata.clone())
                .with_reads(self.reads.clone())
                .with_write_behind(self.write_behind.clone()),
        )
    }

    fn bash_hook_dispatcher(
        &self,
        org_id: i64,
    ) -> Arc<dyn crate::core::hook_executor::BashHookDispatcher> {
        Arc::new(
            everruns_integrations_bashkit::BashkitShellHookDispatcher::new(self.file_store(org_id)),
        )
    }

    fn file_store(&self, org_id: i64) -> Arc<dyn SessionFileSystem> {
        // Org-scoped like `provider_store` and `sqldb_store`: the file surface
        // reaches the server through the org's command transport, so it needs
        // the org the turn is running for.
        Arc::new(SessionAdapter::new(self.adapters.clone()).for_org(org_id))
    }

    fn image_resolver(&self, org_id: i64) -> Option<Arc<dyn ImageResolver>> {
        Some(Arc::new(OrgAdapter::new(self.adapters.clone(), org_id)))
    }

    fn file_resolver(&self, org_id: i64) -> Option<Arc<dyn FileResolver>> {
        Some(Arc::new(OrgAdapter::new(self.adapters.clone(), org_id)))
    }

    fn image_artifact_store(&self, org_id: i64) -> Option<Arc<dyn ImageArtifactStore>> {
        Some(self.adapters.image_artifact_store(org_id))
    }

    fn provider_credential_store(&self, org_id: i64) -> Option<Arc<dyn ProviderCredentialStore>> {
        Some(self.adapters.provider_credential_store(org_id))
    }

    fn utility_llm_service(&self) -> Option<Arc<dyn UtilityLlmService>> {
        self.adapters.utility_llm_service()
    }

    fn decisions(&self) -> Option<Arc<dyn crate::core::DecisionsService>> {
        self.adapters.decisions()
    }

    fn egress_service(&self) -> Option<Arc<dyn EgressService>> {
        self.adapters.egress_service()
    }

    fn storage_store(
        &self,
        org_id: i64,
    ) -> Option<Arc<dyn crate::core::session_services::SessionStorageStore>> {
        Some(self.adapters.storage_store(org_id))
    }

    fn connection_resolver(
        &self,
    ) -> Option<Arc<dyn crate::core::connection_services::UserConnectionResolver>> {
        Some(self.adapters.connection_resolver())
    }

    fn tool_context_extensions(&self, request: ToolContextRequest<'_>) -> ToolContextExtensions {
        let ToolContextRequest {
            org_id,
            session_id,
            resolved_capabilities,
        } = request;
        let mut extensions = ToolContextExtensions::default();
        let platform_store = self.adapters.platform_store(org_id, session_id);
        // Shell-surface platform harnesses omit the forwarding tools, so install
        // their catalog directly. Never expose it to a shell-only harness: the
        // `everruns` builtin is the whole platform surface, so a session without
        // the capability that grants it must not receive a command source.
        // Read from the resolved set, not the declared one — `platform` can
        // arrive by dependency expansion or under an alias, and the server-side
        // gate on InvokePlatformCommandSurface resolves the same way.
        let has_platform_capability = resolved_capabilities
            .iter()
            .any(|capability| capability.capability_id() == PLATFORM_CAPABILITY_ID);
        if has_platform_capability {
            extensions.insert(Arc::new(crate::catalog_cli::CatalogCommandSource::handle(
                platform_store.clone(),
            )));
        }
        extensions.insert(Arc::new(crate::core::tool_context::ExecutionServicesExt(
            Arc::new(PlatformExecutionScope {
                store: platform_store.clone(),
                has_catalog: has_platform_capability,
            }),
        )));
        extensions.insert(Arc::new(PlatformStoreExt(platform_store)));
        if let Some(store) = self.adapters.knowledge_store() {
            extensions.insert(Arc::new(KnowledgeStoreExt(store)));
        }
        if let Some(search) = self.adapters.knowledge_index_search(org_id) {
            extensions.insert(Arc::new(KnowledgeIndexSearchExt(search)));
        }
        extensions.insert(Arc::new(SessionSqlDbStoreExt(
            self.adapters.sqldb_store(org_id),
        )));
        // EVE-1024. Installed per session because the invoker is bound to this
        // org and session; a deployment without a control-plane route provides
        // none and the Slack capability's tools fail closed.
        if let Some(invoker) = self.adapters.slack_action_invoker(org_id, session_id) {
            everruns_capabilities::channel_message_sender::install(&mut extensions, invoker);
        }
        if let Some(store) = self.adapters.sandbox_persistence_store() {
            let checkpoints: Arc<dyn everruns_capabilities::SandboxCheckpointStore> = store.clone();
            let state: Arc<dyn everruns_capabilities::SandboxStateStore> = store;
            extensions.insert(Arc::new(SandboxCheckpointStoreExt(checkpoints)));
            extensions.insert(Arc::new(SandboxStateStoreExt(state)));
            // Checkpoint reconciliation needs both; installing one without the
            // other silently disables it (EVE-870).
            if let Some(durable) = self.adapters.durable_tool_result_store() {
                extensions.insert(Arc::new(DurableToolResultStoreExt(durable)));
            }
        }
        extensions
    }

    fn subagent_delegate(
        &self,
        org_id: i64,
        session_id: SessionId,
    ) -> Option<Arc<dyn crate::core::subagent_delegation::SubagentSessionDelegate>> {
        Some(Arc::new(PlatformStoreSubagentDelegate(
            self.adapters.platform_store(org_id, session_id),
        )))
    }

    fn tool_augmentor(&self) -> Option<Arc<dyn crate::host::HostToolAugmentor>> {
        Some(Arc::new(PlatformToolAugmentor))
    }

    fn leased_resource_store(
        &self,
    ) -> Option<Arc<dyn crate::core::session_services::LeasedResourceStore>> {
        Some(self.adapters.leased_resource_store())
    }

    fn session_resource_registry(
        &self,
    ) -> Option<Arc<dyn crate::core::session_services::SessionResourceRegistry>> {
        self.adapters.session_resource_registry()
    }

    fn session_task_registry(
        &self,
    ) -> Option<Arc<dyn crate::core::session_task::SessionTaskRegistry>> {
        self.adapters.session_task_registry()
    }

    fn schedule_store(
        &self,
        org_id: i64,
    ) -> Option<Arc<dyn crate::core::session_services::SessionScheduleStore>> {
        Some(self.adapters.schedule_store(org_id))
    }

    fn budget_checker(
        &self,
        org_id: i64,
        agent_id: Option<AgentId>,
    ) -> Option<Arc<dyn crate::core::tool_execution::BudgetChecker>> {
        self.adapters.budget_checker(org_id, agent_id)
    }

    fn payment_authority(
        &self,
        org_id: i64,
        agent_id: Option<AgentId>,
    ) -> Option<Arc<dyn PaymentAuthority>> {
        self.adapters.payment_authority(org_id, agent_id)
    }

    fn session_creation_authority(
        &self,
        org_id: i64,
        session_id: SessionId,
    ) -> Option<Arc<dyn SessionCreationAuthority>> {
        self.adapters.session_creation_authority(org_id, session_id)
    }

    fn outbound_tool_rate_limiter(
        &self,
        org_id: i64,
    ) -> Option<Arc<dyn crate::core::tool_execution::OutboundToolRateLimiter>> {
        self.adapters.outbound_tool_rate_limiter(org_id)
    }

    fn durable_tool_result_store(
        &self,
    ) -> Option<Arc<dyn crate::core::durability::DurableToolResultStore>> {
        self.adapters.durable_tool_result_store()
    }

    fn subagent_spawn_store(
        &self,
    ) -> Option<Arc<dyn crate::core::delegation_services::SubagentSpawnStore>> {
        self.adapters.subagent_spawn_store()
    }

    fn stream_heartbeater(&self) -> Option<Arc<dyn crate::core::durability::StreamHeartbeater>> {
        self.adapters.stream_heartbeater()
    }

    fn provider_stall_timeout(&self) -> Option<std::time::Duration> {
        self.adapters.provider_stall_timeout()
    }

    /// Execute `mcp_*` tool calls by resolving server connections over gRPC and
    /// calling them through the shared MCP client over the platform egress
    /// boundary (SSRF-guarded). Returns `None` when no egress service is
    /// available, in which case MCP tools are not registered for execution.
    async fn mcp_executor(
        &self,
        org_id: i64,
        session_id: SessionId,
        agent_id: Option<AgentId>,
    ) -> Option<Arc<dyn crate::core::McpToolInvoker>> {
        let egress = self.adapters.egress_service()?;
        // A session has a user who can be shown a URL and asked about it, so
        // this host declares URL mode elicitation. It never opens anything and
        // never invents consent: the first call stands the elicitation down and
        // the turn pauses on a consent card (`UrlElicitationHook`); the run that
        // follows the user's consent finds it recorded here and answers the
        // server `accept`.
        //
        // Form mode rides the same store: the turn pauses on an `ask_user` card
        // (`FormElicitationHook`), and the retry sends the recorded answer. Only
        // servers whose policy allows it are told this client does forms.
        let answers = Arc::new(
            crate::mcp_elicitation_consent::SessionElicitationConsents::new(
                self.adapters.storage_store(org_id),
                session_id,
            ),
        );
        let client = Arc::new(McpClient::with_url_and_form_elicitation(
            egress,
            Arc::new(NoAuthProvider),
            Arc::new(crate::mcp::ConsentingUrlElicitations::new(answers.clone())),
            Arc::new(crate::mcp::StoredFormAnswers::new(answers)),
        ));
        let resolver = Arc::new(WorkerMcpResolver {
            input_message_id: None,
            adapters: self.adapters.clone(),
            org_id,
            session_id: session_id.uuid(),
            agent_id,
        });
        Some(Arc::new(McpExecutor::new(client, resolver)))
    }

    fn hosted_mcp_resolver(
        &self,
        org_id: i64,
        session_id: SessionId,
        agent_id: Option<AgentId>,
        input_message_id: MessageId,
    ) -> Option<Arc<dyn everruns_contracts::hosted_mcp::HostedMcpResolver>> {
        Some(Arc::new(WorkerMcpResolver {
            input_message_id: Some(input_message_id.uuid()),
            adapters: self.adapters.clone(),
            org_id,
            session_id: session_id.uuid(),
            agent_id,
        }))
    }
}

/// Registered MCP servers that OpenAI calls as a hosted tool (EVE-1115).
///
/// Same lookup as `mcp_*` execution, so scoping, API keys and OAuth tokens
/// match what the agent's own MCP tools get. OpenAI calls the server itself,
/// so there is no tool call to answer `connection_required`: a missing grant
/// fails the turn with a message naming where to connect it, and a server with
/// secret-bound tool parameters is refused because OpenAI cannot inject them.
#[async_trait]
impl<A: WorkerAdapters> everruns_contracts::hosted_mcp::HostedMcpResolver for WorkerMcpResolver<A> {
    async fn resolve(
        &self,
        server: &str,
    ) -> Result<everruns_contracts::hosted_mcp::ResolvedHostedMcp> {
        let configuration = everruns_contracts::error::AgentLoopError::Configuration;
        let prefix = crate::core::mcp_server::sanitize_mcp_server_name(server);
        let connection = McpConnectionResolver::resolve(self, &prefix)
            .await
            .map_err(|error| configuration(format!("MCP server {server}: {error}")))?
            .ok_or_else(|| {
                configuration(format!(
                    "MCP server {server} is not available to this session"
                ))
            })?;
        if let Some(pending) = connection.pending_oauth_provider {
            let setup = pending
                .setup_url
                .as_deref()
                .unwrap_or("/settings/connections");
            return Err(configuration(format!(
                "MCP server {server} needs a {} connection; connect it at {setup}",
                pending.provider
            )));
        }
        if !connection.secret_bindings.is_empty() {
            return Err(configuration(format!(
                "MCP server {server} binds secrets to tool parameters, which OpenAI cannot supply"
            )));
        }
        #[allow(unreachable_patterns)]
        let (url, headers) = match connection.endpoint {
            McpEndpoint::Http { url, headers } => (url, headers.into_iter().collect()),
            _ => {
                return Err(configuration(format!(
                    "MCP server {server} is not a remote server"
                )));
            }
        };
        Ok(everruns_contracts::hosted_mcp::ResolvedHostedMcp { url, headers })
    }
}

struct PlatformExecutionScope {
    store: Arc<dyn everruns_capabilities::PlatformStore>,
    has_catalog: bool,
}
impl crate::core::tool_context::ExecutionServices for PlatformExecutionScope {
    fn bind(&self, context: &mut crate::core::tool_context::ToolContext, id: Uuid) {
        if let Some(store) = self.store.for_execution(id) {
            if self.has_catalog {
                context.extensions.insert(Arc::new(
                    crate::catalog_cli::CatalogCommandSource::handle(store.clone()),
                ));
            }
            context.subagent_delegate =
                Some(Arc::new(PlatformStoreSubagentDelegate(store.clone())));
            context.extensions.insert(Arc::new(PlatformStoreExt(store)));
        }
    }
}

#[cfg(test)]
#[path = "runtime_host_mcp_credential_tests.rs"]
mod mcp_credential_tests;

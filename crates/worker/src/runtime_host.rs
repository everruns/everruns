// Runtime-host adapter bridge for durable/server-backed workers.
// Decision: everruns-worker exposes first-party adapters from WorkerAdapters to
// the neutral everruns-host execution contract.

use async_trait::async_trait;
use everruns_core::tool_context::ToolContextExtensions;
use everruns_core::{
    CapabilityRegistry, EgressService, ResolvedExecutionSnapshot, SessionExecutionState,
    UtilityLlmService,
};
use everruns_core::{
    connection_services::ProviderCredentialStore, delegation_services::SessionCreationAuthority,
    event_emitter::EventEmitter, execution_loading::AgentStore, execution_loading::HarnessStore,
    execution_loading::SessionStore, file_services::FileResolver,
    image_services::ImageArtifactStore, image_services::ImageResolver,
    provider_resolution::ProviderStore, session_files::SessionFileSystem,
    tool_execution::PaymentAuthority,
};
use everruns_host::{ResolvedTurnInputs, RuntimeHostAdapter};
use everruns_mcp::{
    McpClient, McpConnection, McpConnectionResolver, McpEndpoint, McpExecutor, NoAuthProvider,
};
use everruns_platform::SessionMutator;
use everruns_platform::{
    DurableToolResultStoreExt, KnowledgeIndexSearchExt, KnowledgeStoreExt, PlatformStoreExt,
    PlatformStoreSubagentDelegate, PlatformToolAugmentor, SandboxCheckpointStoreExt,
    SessionSqlDbStoreExt,
};
use everruns_provider::driver_registry::DriverRegistry;
use everruns_provider::error::Result;
use everruns_provider::typed_id::{AgentId, SessionId};
use std::sync::Arc;
use uuid::Uuid;

use crate::worker_adapters::{OrgAdapter, SessionAdapter, WorkerAdapters};

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
struct WorkerMcpResolver<A: WorkerAdapters> {
    adapters: A,
    org_id: i64,
    session_id: Uuid,
}

#[async_trait]
impl<A: WorkerAdapters> McpConnectionResolver for WorkerMcpResolver<A> {
    async fn resolve(&self, server_prefix: &str) -> anyhow::Result<Option<McpConnection>> {
        let info = self
            .adapters
            .get_mcp_server_by_prefix(self.org_id, Some(self.session_id), server_prefix)
            .await
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
        if info.auth_mode == everruns_core::McpServerAuthMode::OAuth
            && !has_authorization(&headers)
            && let Some(provider) = info.oauth_provider_id.as_deref()
        {
            // An attachment that declares an acting identity resolves through
            // the `actsAs`-aware lookup, which reads exactly one store and has
            // no fallback. Attachments that declare none (legacy org-level MCP
            // servers and inline scoped entries) keep the existing lookup, so
            // configs predating `actsAs` behave exactly as they do today
            // (EVE-1029).
            let resolver = self.adapters.connection_resolver();
            let resolved = if info.acts_as.is_none() {
                resolver
                    .get_connection_token(self.session_id.into(), provider)
                    .await
            } else {
                resolver
                    .get_mcp_connection_token(self.session_id.into(), provider, info.acts_as)
                    .await
            };

            match resolved {
                Ok(Some(token)) => {
                    headers.insert("Authorization".to_string(), format!("Bearer {token}"));
                }
                Ok(None) => pending_oauth_provider = Some(provider.to_string()),
                Err(error) => {
                    tracing::warn!(
                        server = %info.name,
                        provider,
                        %error,
                        "failed to resolve MCP OAuth connection token"
                    );
                    pending_oauth_provider = Some(provider.to_string());
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
            oauth_provider_id: info.oauth_provider_id,
            pending_oauth_provider,
            secret_bindings: info.secret_bindings,
        }))
    }
}

/// First-party adapter from worker backends into `everruns-host` execution.
///
/// This is the bridge that lets durable workers execute the shared runtime
/// host phases without depending on in-process-only stores.
///
/// ```ignore
/// use everruns_host::execute_reason_activity;
/// use everruns_worker::{GrpcWorkerAdapters, WorkerRuntimeHost};
///
/// let adapters = GrpcWorkerAdapters::connect("127.0.0.1:9001").await?;
/// let host = WorkerRuntimeHost::new(adapters);
/// let result = execute_reason_activity(&host, org_id, reason_input).await?;
/// # Ok::<(), everruns_provider::error::AgentLoopError>(())
/// ```
#[derive(Clone)]
pub struct WorkerRuntimeHost<A: WorkerAdapters> {
    adapters: A,
    cancellation: Option<tokio::sync::watch::Receiver<bool>>,
    event_metadata: Option<serde_json::Map<String, serde_json::Value>>,
}

impl<A: WorkerAdapters> WorkerRuntimeHost<A> {
    pub fn with_turn_cancellation(
        mut self,
        cancellation: tokio::sync::watch::Receiver<bool>,
    ) -> Self {
        self.cancellation = Some(cancellation);
        self
    }
    pub fn new(adapters: A) -> Self {
        Self {
            adapters,
            cancellation: None,
            event_metadata: None,
        }
    }

    pub fn with_event_metadata(
        adapters: A,
        metadata: Option<serde_json::Map<String, serde_json::Value>>,
    ) -> Self {
        Self {
            adapters,
            cancellation: None,
            event_metadata: metadata,
        }
    }
}

#[async_trait]
impl<A: WorkerAdapters> RuntimeHostAdapter for WorkerRuntimeHost<A> {
    fn turn_cancellation(&self) -> Option<tokio::sync::watch::Receiver<bool>> {
        self.cancellation.clone()
    }
    async fn set_session_status(
        &self,
        org_id: i64,
        session_id: SessionId,
        status: SessionExecutionState,
    ) -> Result<()> {
        // Status mutation is an acknowledged effect end to end (EVE-882):
        // neither the adapter nor the host contract exposes a session record.
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
        // The batched control-plane transport still ships stored records (see
        // `WorkerAdapters::load_turn_context`).
        // They are projected into the canonical resolved execution snapshot
        // here, at the platform boundary, so host execution never sees them
        // (EVE-872). The control plane returns the harness pre-merged, so the
        // effective definition folds identically to the in-process runtime.
        let context = self
            .adapters
            .load_turn_context(org_id, session_id.uuid())
            .await?;
        // Loading seam (EVE-877/EVE-881): project the stored records into the
        // portable execution definitions; archived/deleted harnesses and
        // agents fail here, before the snapshot is built.
        let harness_definition = self
            .adapters
            .get_harness(org_id, context.session.harness_id.uuid())
            .await?
            .ok_or_else(|| {
                everruns_provider::error::AgentLoopError::harness_not_found(
                    context.session.harness_id,
                )
            })?
            .execution_definition()?;
        let agent_definition = context
            .agent
            .as_ref()
            .map(|agent| agent.execution_definition())
            .transpose()?;
        let snapshot = ResolvedExecutionSnapshot::project(
            &harness_definition,
            agent_definition.as_ref(),
            &context.session,
        )?;
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
        Arc::new(OrgAdapter::new(self.adapters.clone(), org_id))
    }

    fn agent_store(&self, org_id: i64) -> Arc<dyn AgentStore> {
        Arc::new(OrgAdapter::new(self.adapters.clone(), org_id))
    }

    fn session_store(&self, org_id: i64) -> Arc<dyn SessionStore> {
        Arc::new(OrgAdapter::new(self.adapters.clone(), org_id))
    }

    fn session_mutator(&self, org_id: i64) -> Arc<dyn SessionMutator> {
        Arc::new(OrgAdapter::new(self.adapters.clone(), org_id))
    }

    fn provider_store(&self, org_id: i64) -> Arc<dyn ProviderStore> {
        Arc::new(OrgAdapter::new(self.adapters.clone(), org_id))
    }

    fn message_store(&self) -> Arc<dyn everruns_core::MessageRetriever> {
        Arc::new(SessionAdapter::new(self.adapters.clone()))
    }

    fn native_async_store(
        &self,
    ) -> Option<Arc<dyn everruns_core::native_async_store::NativeAsyncStore>> {
        self.adapters.native_async_store()
    }

    fn compaction_checkpoint_store(
        &self,
    ) -> Option<Arc<dyn everruns_core::CompactionCheckpointStore>> {
        self.adapters.compaction_checkpoint_store()
    }

    fn event_emitter(&self) -> Arc<dyn EventEmitter> {
        Arc::new(
            SessionAdapter::new(self.adapters.clone())
                .with_event_metadata(self.event_metadata.clone()),
        )
    }

    fn file_store(&self) -> Arc<dyn SessionFileSystem> {
        Arc::new(SessionAdapter::new(self.adapters.clone()))
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

    fn classifier(&self) -> Option<Arc<dyn everruns_core::ClassifierService>> {
        self.adapters.classifier()
    }

    fn egress_service(&self) -> Option<Arc<dyn EgressService>> {
        self.adapters.egress_service()
    }

    fn storage_store(
        &self,
    ) -> Option<Arc<dyn everruns_core::session_services::SessionStorageStore>> {
        Some(self.adapters.storage_store())
    }

    fn connection_resolver(
        &self,
    ) -> Option<Arc<dyn everruns_core::connection_services::UserConnectionResolver>> {
        Some(self.adapters.connection_resolver())
    }

    fn tool_context_extensions(&self, org_id: i64, session_id: SessionId) -> ToolContextExtensions {
        let mut extensions = ToolContextExtensions::default();
        let platform_store = self.adapters.platform_store(org_id, session_id);
        // The `everruns` command in the session's own shell. Inserted for every
        // session: the shell only installs the builtin when a source is present,
        // and a harness without a shell never asks. A shell-surface harness has
        // no `execute` tool to forward to, so this is how it reaches the catalog
        // at all.
        extensions.insert(Arc::new(crate::catalog_cli::CatalogCommandSource::handle(
            platform_store.clone(),
        )));
        extensions.insert(Arc::new(PlatformStoreExt(platform_store)));
        if let Some(store) = self.adapters.knowledge_store() {
            extensions.insert(Arc::new(KnowledgeStoreExt(store)));
        }
        if let Some(search) = self.adapters.knowledge_index_search(org_id) {
            extensions.insert(Arc::new(KnowledgeIndexSearchExt(search)));
        }
        extensions.insert(Arc::new(SessionSqlDbStoreExt(self.adapters.sqldb_store())));
        if let Some(store) = self.adapters.sandbox_checkpoint_store() {
            extensions.insert(Arc::new(SandboxCheckpointStoreExt(store)));
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
    ) -> Option<Arc<dyn everruns_core::subagent_delegation::SubagentSessionDelegate>> {
        Some(Arc::new(PlatformStoreSubagentDelegate(
            self.adapters.platform_store(org_id, session_id),
        )))
    }

    fn tool_augmentor(&self) -> Option<Arc<dyn everruns_host::HostToolAugmentor>> {
        Some(Arc::new(PlatformToolAugmentor))
    }

    fn leased_resource_store(
        &self,
    ) -> Option<Arc<dyn everruns_core::session_services::LeasedResourceStore>> {
        Some(self.adapters.leased_resource_store())
    }

    fn session_resource_registry(
        &self,
    ) -> Option<Arc<dyn everruns_core::session_services::SessionResourceRegistry>> {
        self.adapters.session_resource_registry()
    }

    fn session_task_registry(
        &self,
    ) -> Option<Arc<dyn everruns_core::session_task::SessionTaskRegistry>> {
        self.adapters.session_task_registry()
    }

    fn schedule_store(
        &self,
        org_id: i64,
    ) -> Option<Arc<dyn everruns_core::session_services::SessionScheduleStore>> {
        Some(self.adapters.schedule_store(org_id))
    }

    fn budget_checker(
        &self,
        org_id: i64,
        agent_id: Option<AgentId>,
    ) -> Option<Arc<dyn everruns_core::tool_execution::BudgetChecker>> {
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
    ) -> Option<Arc<dyn everruns_core::tool_execution::OutboundToolRateLimiter>> {
        self.adapters.outbound_tool_rate_limiter(org_id)
    }

    fn durable_tool_result_store(
        &self,
    ) -> Option<Arc<dyn everruns_core::durability::DurableToolResultStore>> {
        self.adapters.durable_tool_result_store()
    }

    fn subagent_spawn_store(
        &self,
    ) -> Option<Arc<dyn everruns_core::delegation_services::SubagentSpawnStore>> {
        self.adapters.subagent_spawn_store()
    }

    fn stream_heartbeater(&self) -> Option<Arc<dyn everruns_core::durability::StreamHeartbeater>> {
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
    ) -> Option<Arc<dyn everruns_core::McpToolInvoker>> {
        let egress = self.adapters.egress_service()?;
        // A session has a user who can be shown a URL and asked about it, so
        // this host declares URL mode elicitation. It never opens anything and
        // never invents consent: the first call stands the elicitation down and
        // the turn pauses on a consent card (`UrlElicitationHook`); the run that
        // follows the user's consent finds it recorded here and answers the
        // server `accept`.
        let client = Arc::new(McpClient::with_url_elicitation(
            egress,
            Arc::new(NoAuthProvider),
            Arc::new(everruns_mcp::ConsentingUrlElicitations::new(Arc::new(
                crate::mcp_elicitation_consent::SessionElicitationConsents::new(
                    self.adapters.storage_store(),
                    session_id,
                ),
            ))),
        ));
        let resolver = Arc::new(WorkerMcpResolver {
            adapters: self.adapters.clone(),
            org_id,
            session_id: session_id.uuid(),
        });
        Some(Arc::new(McpExecutor::new(client, resolver)))
    }
}

#[cfg(test)]
mod mcp_credential_tests {
    //! EVE-1029: the worker path must send the credential the attachment's
    //! `actsAs` names, and nothing else.
    //!
    //! Resolver unit tests can pass while this path sends something different,
    //! so these assert on the `Authorization` header that actually reaches the
    //! transport, plus which lookup the worker chose.

    use super::*;
    use crate::worker_adapters::WorkerAdapters;
    use everruns_core::McpServerActsAs;
    use everruns_core::connection_services::UserConnectionResolver;
    use everruns_provider::error::Result as CoreResult;
    use everruns_provider::typed_id::SessionId as CoreSessionId;
    use std::collections::HashMap;
    use std::sync::Mutex as StdMutex;

    /// Records which lookup the worker used. The legacy lookup and the
    /// `actsAs` lookup return distinguishable tokens so a test can tell which
    /// one produced the header.
    #[derive(Default)]
    struct RecordingResolver {
        acts_as_calls: StdMutex<Vec<McpServerActsAs>>,
        legacy_calls: StdMutex<usize>,
        acts_as_token: Option<String>,
        legacy_token: Option<String>,
        fail: bool,
    }

    #[async_trait::async_trait]
    impl UserConnectionResolver for RecordingResolver {
        async fn get_connection_token(
            &self,
            _session_id: CoreSessionId,
            _provider: &str,
        ) -> CoreResult<Option<String>> {
            *self.legacy_calls.lock().unwrap() += 1;
            if self.fail {
                return Err(everruns_provider::error::AgentLoopError::store("db down"));
            }
            Ok(self.legacy_token.clone())
        }

        async fn get_mcp_connection_token(
            &self,
            _session_id: CoreSessionId,
            _provider: &str,
            acts_as: McpServerActsAs,
        ) -> CoreResult<Option<String>> {
            self.acts_as_calls.lock().unwrap().push(acts_as);
            if self.fail {
                return Err(everruns_provider::error::AgentLoopError::store("db down"));
            }
            Ok(self.acts_as_token.clone())
        }
    }

    fn server_info(
        acts_as: McpServerActsAs,
        auth_mode: everruns_core::McpServerAuthMode,
        api_key: Option<&str>,
        headers: &[(&str, &str)],
    ) -> crate::mcp_executor::McpServerInfo {
        crate::mcp_executor::McpServerInfo {
            id: Uuid::new_v4(),
            name: "linear".to_string(),
            url: "https://mcp.linear.app/mcp".to_string(),
            auth_mode,
            protocol_mode: everruns_core::McpProtocolMode::Auto,
            oauth_provider_id: Some(format!("mcp_oauth_{}", Uuid::new_v4())),
            acts_as,
            api_key: api_key.map(str::to_string),
            headers: headers
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            secret_bindings: HashMap::new(),
        }
    }

    /// The header the transport would actually send.
    fn authorization_of(connection: &McpConnection) -> Option<String> {
        match &connection.endpoint {
            McpEndpoint::Http { headers, .. } => headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("authorization"))
                .map(|(_, v)| v.clone()),
            // `McpEndpoint::Stdio` exists only when `everruns-mcp/stdio` is
            // enabled, which a workspace-wide `--all-features` build does. The
            // arm must therefore compile both with and without it, so the
            // wildcard stays and the lint is silenced rather than cfg-gated on
            // a feature this crate does not declare.
            #[allow(unreachable_patterns)]
            _ => None,
        }
    }

    async fn resolve_with(
        info: crate::mcp_executor::McpServerInfo,
        resolver: RecordingResolver,
    ) -> (McpConnection, Arc<RecordingResolver>) {
        let resolver = Arc::new(resolver);
        let adapters = StubAdapters {
            info,
            resolver: resolver.clone(),
        };
        let worker_resolver = WorkerMcpResolver {
            adapters,
            org_id: everruns_core::DEFAULT_ORG_ID,
            session_id: Uuid::new_v4(),
        };
        let connection = worker_resolver
            .resolve("linear")
            .await
            .expect("resolve")
            .expect("connection");
        (connection, resolver)
    }

    #[tokio::test]
    async fn an_acting_identity_routes_through_the_acts_as_lookup_not_the_legacy_one() {
        for acts_as in [McpServerActsAs::Service, McpServerActsAs::User] {
            let (connection, resolver) = resolve_with(
                server_info(acts_as, everruns_core::McpServerAuthMode::OAuth, None, &[]),
                RecordingResolver {
                    acts_as_token: Some("scoped-token".to_string()),
                    legacy_token: Some("fallback-token".to_string()),
                    ..Default::default()
                },
            )
            .await;

            assert_eq!(
                authorization_of(&connection).as_deref(),
                Some("Bearer scoped-token")
            );
            assert_eq!(*resolver.acts_as_calls.lock().unwrap(), vec![acts_as]);
            assert_eq!(
                *resolver.legacy_calls.lock().unwrap(),
                0,
                "{acts_as} must not reach the identity-preferring fallback"
            );
            assert!(connection.pending_oauth_provider.is_none());
        }
    }

    #[tokio::test]
    async fn an_attachment_without_an_acting_identity_keeps_the_existing_lookup() {
        // Legacy org-level MCP servers and inline scoped entries resolve
        // exactly as they did before actsAs existed.
        let (connection, resolver) = resolve_with(
            server_info(
                McpServerActsAs::None,
                everruns_core::McpServerAuthMode::OAuth,
                None,
                &[],
            ),
            RecordingResolver {
                acts_as_token: Some("scoped-token".to_string()),
                legacy_token: Some("fallback-token".to_string()),
                ..Default::default()
            },
        )
        .await;

        assert_eq!(
            authorization_of(&connection).as_deref(),
            Some("Bearer fallback-token")
        );
        assert_eq!(*resolver.legacy_calls.lock().unwrap(), 1);
        assert!(resolver.acts_as_calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_missing_grant_becomes_connection_required_never_an_unauthenticated_call() {
        for acts_as in [McpServerActsAs::Service, McpServerActsAs::User] {
            let (connection, _resolver) = resolve_with(
                server_info(acts_as, everruns_core::McpServerAuthMode::OAuth, None, &[]),
                RecordingResolver {
                    acts_as_token: None,
                    legacy_token: Some("fallback-token".to_string()),
                    ..Default::default()
                },
            )
            .await;

            // No header at all, and the executor is told to prompt rather than
            // let the call go out bare against a permissive server.
            assert_eq!(authorization_of(&connection), None, "{acts_as}");
            assert!(
                connection.pending_oauth_provider.is_some(),
                "{acts_as} must surface connection_required"
            );
        }
    }

    #[tokio::test]
    async fn a_resolver_error_fails_closed_rather_than_calling_unauthenticated() {
        // A DB outage or decryption failure must degrade to a connect prompt,
        // not to a request with no Authorization that a permissive server
        // would happily accept.
        for acts_as in [McpServerActsAs::Service, McpServerActsAs::User] {
            let (connection, resolver) = resolve_with(
                server_info(acts_as, everruns_core::McpServerAuthMode::OAuth, None, &[]),
                RecordingResolver {
                    acts_as_token: Some("scoped-token".to_string()),
                    legacy_token: Some("fallback-token".to_string()),
                    fail: true,
                    ..Default::default()
                },
            )
            .await;

            assert_eq!(authorization_of(&connection), None, "{acts_as}");
            assert!(
                connection.pending_oauth_provider.is_some(),
                "{acts_as} must surface connection_required on resolver error"
            );
            // It failed on the acts_as lookup and did not then try the fallback.
            assert_eq!(*resolver.acts_as_calls.lock().unwrap(), vec![acts_as]);
            assert_eq!(*resolver.legacy_calls.lock().unwrap(), 0);
        }
    }

    #[tokio::test]
    async fn a_literal_authorization_header_is_never_overwritten_by_a_resolved_token() {
        // `none` semantics: literal headers are the credential, so resolution
        // must not run at all.
        let (connection, resolver) = resolve_with(
            server_info(
                McpServerActsAs::None,
                everruns_core::McpServerAuthMode::OAuth,
                None,
                &[("Authorization", "Bearer literal-header")],
            ),
            RecordingResolver {
                acts_as_token: Some("scoped-token".to_string()),
                legacy_token: Some("fallback-token".to_string()),
                ..Default::default()
            },
        )
        .await;

        assert_eq!(
            authorization_of(&connection).as_deref(),
            Some("Bearer literal-header")
        );
        assert_eq!(*resolver.legacy_calls.lock().unwrap(), 0);
        assert!(resolver.acts_as_calls.lock().unwrap().is_empty());
    }

    #[derive(Clone)]
    struct StubAdapters {
        info: crate::mcp_executor::McpServerInfo,
        resolver: Arc<RecordingResolver>,
    }

    #[async_trait::async_trait]
    impl WorkerAdapters for StubAdapters {
        async fn get_agent(
            &self,
            _org_id: i64,
            _agent_id: Uuid,
        ) -> CoreResult<Option<everruns_platform::Agent>> {
            unimplemented!()
        }
        async fn get_harness(
            &self,
            _org_id: i64,
            _harness_id: Uuid,
        ) -> CoreResult<Option<everruns_platform::Harness>> {
            unimplemented!()
        }
        async fn get_session(
            &self,
            _org_id: i64,
            _session_id: Uuid,
        ) -> CoreResult<Option<everruns_core::ExecutionSession>> {
            unimplemented!()
        }
        async fn set_session_status(
            &self,
            _org_id: i64,
            _session_id: Uuid,
            _status: &str,
        ) -> CoreResult<()> {
            unimplemented!()
        }
        async fn set_session_title(
            &self,
            _org_id: i64,
            _session_id: Uuid,
            _title: String,
        ) -> CoreResult<everruns_core::ExecutionSession> {
            unimplemented!()
        }
        async fn get_message(
            &self,
            _session_id: Uuid,
            _message_id: Uuid,
        ) -> CoreResult<Option<everruns_core::RuntimeMessage>> {
            unimplemented!()
        }
        async fn load_messages(
            &self,
            _session_id: Uuid,
        ) -> CoreResult<Vec<everruns_core::RuntimeMessage>> {
            unimplemented!()
        }
        async fn emit_event(
            &self,
            _request: everruns_core::events::EventRequest,
        ) -> CoreResult<everruns_core::events::Event> {
            unimplemented!()
        }
        async fn get_model_spec(
            &self,
            _org_id: i64,
            _model_id: Uuid,
        ) -> CoreResult<Option<everruns_provider::model_spec::ModelSpec>> {
            unimplemented!()
        }
        async fn get_default_model_spec(
            &self,
            _org_id: i64,
        ) -> CoreResult<Option<everruns_provider::model_spec::ModelSpec>> {
            unimplemented!()
        }
        async fn get_provider_config(
            &self,
            _org_id: i64,
            _provider: &everruns_provider::runtime_provider::ProviderKey,
        ) -> CoreResult<Option<everruns_provider::driver_registry::ProviderConfig>> {
            unimplemented!()
        }
        async fn resolve_image(
            &self,
            _org_id: i64,
            _image_id: Uuid,
        ) -> CoreResult<Option<everruns_core::image_services::ResolvedImage>> {
            unimplemented!()
        }
        async fn resolve_images_batch(
            &self,
            _org_id: i64,
            _image_ids: &[Uuid],
        ) -> CoreResult<HashMap<Uuid, everruns_core::image_services::ResolvedImage>> {
            unimplemented!()
        }
        async fn resolve_files_batch(
            &self,
            _org_id: i64,
            _file_ids: &[Uuid],
        ) -> CoreResult<HashMap<Uuid, everruns_core::file_services::ResolvedFile>> {
            unimplemented!()
        }
        async fn read_file(
            &self,
            _session_id: Uuid,
            _path: &str,
        ) -> CoreResult<Option<everruns_core::session_file::SessionFile>> {
            unimplemented!()
        }
        async fn write_file(
            &self,
            _session_id: Uuid,
            _path: &str,
            _content: &str,
            _encoding: &str,
        ) -> CoreResult<everruns_core::session_file::SessionFile> {
            unimplemented!()
        }
        async fn delete_file(
            &self,
            _session_id: Uuid,
            _path: &str,
            _recursive: bool,
        ) -> CoreResult<bool> {
            unimplemented!()
        }
        async fn list_directory(
            &self,
            _session_id: Uuid,
            _path: &str,
        ) -> CoreResult<Vec<everruns_core::session_file::FileInfo>> {
            unimplemented!()
        }
        async fn stat_file(
            &self,
            _session_id: Uuid,
            _path: &str,
        ) -> CoreResult<Option<everruns_core::session_file::FileStat>> {
            unimplemented!()
        }
        async fn grep_files(
            &self,
            _session_id: Uuid,
            _pattern: &str,
            _path_pattern: Option<&str>,
        ) -> CoreResult<Vec<everruns_core::session_file::GrepMatch>> {
            unimplemented!()
        }
        async fn create_directory(
            &self,
            _session_id: Uuid,
            _path: &str,
        ) -> CoreResult<everruns_core::session_file::FileInfo> {
            unimplemented!()
        }
        async fn get_mcp_server_by_prefix(
            &self,
            _org_id: i64,
            _session_id: Option<Uuid>,
            _server_prefix: &str,
        ) -> CoreResult<crate::mcp_executor::McpServerInfo> {
            Ok(self.info.clone())
        }
        async fn load_turn_context(
            &self,
            _org_id: i64,
            _session_id: Uuid,
        ) -> CoreResult<crate::worker_adapters::TurnContext> {
            unimplemented!()
        }
        async fn invoke_scheduled_app_channel(
            &self,
            _org_id: i64,
            _app_id: &str,
            _channel_id: &str,
        ) -> CoreResult<serde_json::Value> {
            unimplemented!()
        }
        async fn invoke_agent_trigger(
            &self,
            _org_id: i64,
            _agent_id: &str,
            _trigger_id: &str,
        ) -> CoreResult<serde_json::Value> {
            unimplemented!()
        }
        async fn claim_due_leased_resources(
            &self,
            _limit: u32,
            _stale_after_seconds: u32,
        ) -> CoreResult<Vec<everruns_core::leased_resource::LeasedResource>> {
            unimplemented!()
        }
        async fn mark_leased_resource_released(
            &self,
            _resource_id: everruns_provider::typed_id::LeasedResourceId,
            _expected_cleanup_started_at: chrono::DateTime<chrono::Utc>,
        ) -> CoreResult<bool> {
            unimplemented!()
        }
        async fn mark_leased_resource_cleanup_failed(
            &self,
            _resource_id: everruns_provider::typed_id::LeasedResourceId,
            _expected_cleanup_started_at: chrono::DateTime<chrono::Utc>,
            _retry_after_seconds: u32,
            _error: &str,
        ) -> CoreResult<bool> {
            unimplemented!()
        }
        async fn list_orphaned_session_task_ids(
            &self,
            _stale_after: chrono::Duration,
            _limit: i64,
        ) -> CoreResult<Vec<(everruns_provider::typed_id::SessionId, String)>> {
            unimplemented!()
        }
        async fn prune_terminal_session_tasks(
            &self,
            _ttl: chrono::Duration,
            _limit: i64,
        ) -> CoreResult<usize> {
            unimplemented!()
        }

        fn capability_registry(&self) -> everruns_core::capabilities::CapabilityRegistry {
            unimplemented!()
        }
        fn driver_registry(&self) -> everruns_provider::DriverRegistry {
            unimplemented!()
        }
        fn sqldb_store(
            &self,
        ) -> std::sync::Arc<dyn everruns_platform::session_sqldb::SessionSqlDbStore> {
            unimplemented!()
        }
        fn storage_store(&self) -> Arc<dyn everruns_core::session_services::SessionStorageStore> {
            unimplemented!()
        }
        fn image_artifact_store(
            &self,
            _org_id: i64,
        ) -> Arc<dyn everruns_core::image_services::ImageArtifactStore> {
            unimplemented!()
        }
        fn provider_credential_store(
            &self,
            _org_id: i64,
        ) -> Arc<dyn everruns_core::connection_services::ProviderCredentialStore> {
            unimplemented!()
        }
        fn utility_llm_service(&self) -> Option<Arc<dyn everruns_core::UtilityLlmService>> {
            unimplemented!()
        }
        fn egress_service(&self) -> Option<Arc<dyn everruns_core::EgressService>> {
            unimplemented!()
        }
        fn platform_store(
            &self,
            _org_id: i64,
            _session_id: everruns_provider::typed_id::SessionId,
        ) -> Arc<dyn everruns_platform::PlatformStore> {
            unimplemented!()
        }
        fn connection_resolver(
            &self,
        ) -> Arc<dyn everruns_core::connection_services::UserConnectionResolver> {
            self.resolver.clone()
        }
        fn leased_resource_store(
            &self,
        ) -> Arc<dyn everruns_core::session_services::LeasedResourceStore> {
            unimplemented!()
        }
        fn schedule_store(
            &self,
            _org_id: i64,
        ) -> Arc<dyn everruns_core::session_services::SessionScheduleStore> {
            unimplemented!()
        }
        fn reaper_session_task_registry(
            &self,
        ) -> Arc<dyn everruns_core::session_task::SessionTaskRegistry> {
            unimplemented!()
        }
    }
}

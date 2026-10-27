// Internal gRPC Service for Worker Communication
//
// Decision: Workers communicate with control plane via gRPC for all database operations
// Decision: This provides a clean boundary and simplifies worker deployment
// Decision: gRPC uses the same domain entrypoints as HTTP/MCP for user-facing logic
// Decision: Remaining top-level services are cross-cutting infra helpers only
// Decision: Split into submodules for maintainability (EVE-101).

mod worker;
// The one wire shape of a command failure, shared with the in-process worker.
pub(crate) use worker::support::command_error_to_proto;
mod worker_service_impl;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_platform_command_surface;
#[cfg(test)]
mod tests_provider_services;
#[cfg(test)]
mod tests_sqldb_sharing;
#[cfg(test)]
mod tests_turn_context;
#[cfg(test)]
mod tests_worker_file_ctx;

use crate::domains::mcp_servers::McpServerService;
use crate::domains::session_files::WorkspaceFileService;
use crate::domains::sessions::SessionService;
use crate::live_updates::task_notifications::TaskBroadcaster;
use crate::records::wire::{schema_agent_to_proto, schema_harness_to_proto};
use crate::services::{CapabilityService, EventService, ProviderResolverService};
use crate::storage::{EncryptionService, StorageBackend};
use everruns_core::host::HostComposition;
use everruns_core::permissions::PermissionResolver;
use everruns_durable::{
    CircuitBreakerConfig, CircuitState, DistributedCircuitBreaker, PostgresWorkflowEventStore,
    StoreError, TaskFailureOutcome, WorkerInfo, WorkflowError, WorkflowEventStore, WorkflowStatus,
    record_activity_completed, record_activity_failed, record_workflow_failed,
};
use everruns_internal_protocol::proto::{
    self,
    AddMessageRequest,
    AddMessageResponse,
    AuthorizeSessionCreationRequest,
    AuthorizeSessionCreationResponse,
    BudgetSummaryProto,
    // Session schedule operations
    // Budget operations
    CheckBudgetsForSessionRequest,
    CheckBudgetsForSessionResponse,
    CheckCircuitBreakerRequest,
    CheckCircuitBreakerResponse,
    CheckOutboundToolRateLimitRequest,
    CheckOutboundToolRateLimitResponse,
    CircuitBreakerState as ProtoCircuitBreakerState,
    // Leased resource sweeper
    ClaimDueLeasedResourcesRequest,
    ClaimDueLeasedResourcesResponse,
    ClaimDurableTasksRequest,
    ClaimDurableTasksResponse,
    CommandCatalogEntry,
    CommandError as ProtoCommandError,
    CommitExecRequest,
    CommitExecResponse,
    CompleteDurableTaskRequest,
    CompleteDurableTaskResponse,
    DeregisterDurableWorkerRequest,
    DeregisterDurableWorkerResponse,
    DrainDurableWorkerRequest,
    DrainDurableWorkerResponse,
    DurableWorkflowStatus,
    EmitEventRequest,
    EmitEventResponse,
    EmitEventStreamResponse,
    ExecuteCommandRequest,
    ExecuteCommandResponse,
    ExecuteMachinePaymentRequest,
    ExecuteMachinePaymentResponse,
    FailDurableTaskRequest,
    FailDurableTaskResponse,
    GetAgentRequest,
    GetAgentResponse,
    GetDefaultModelRequest,
    GetDefaultModelResponse,
    GetDefaultProviderCredentialsRequest,
    GetDefaultProviderCredentialsResponse,
    GetHarnessRequest,
    GetHarnessResponse,
    GetImageArtifactRequest,
    GetImageArtifactResponse,
    GetMcpServerByPrefixRequest,
    GetMcpServerByPrefixResponse,
    GetMessageRequest,
    GetMessageResponse,
    GetResolvedModelRequest,
    GetResolvedModelResponse,
    GetSessionRequest,
    GetSessionResponse,
    GetTurnContextRequest,
    GetTurnContextResponse,
    HeartbeatDurableTaskRequest,
    HeartbeatDurableTaskResponse,
    HeartbeatDurableWorkerRequest,
    HeartbeatDurableWorkerResponse,
    InvokeAgentTriggerRequest,
    InvokeAgentTriggerResponse,
    InvokePlatformCommandSurfaceRequest,
    InvokePlatformCommandSurfaceResponse,
    InvokeScheduledAppChannelRequest,
    InvokeScheduledAppChannelResponse,
    InvokeSlackActionRequest,
    InvokeSlackActionResponse,
    ListCommandsRequest,
    ListCommandsResponse,
    ListOrphanedSessionTasksRequest,
    ListOrphanedSessionTasksResponse,
    LoadMessagesRequest,
    LoadMessagesResponse,
    MarkLeasedResourceCleanupFailedRequest,
    MarkLeasedResourceCleanupFailedResponse,
    MarkLeasedResourceReleasedRequest,
    MarkLeasedResourceReleasedResponse,
    McpServerInfo,
    McpToolDef,
    OrphanedSessionTaskEntry, // orphan-scan entry for ListOrphanedSessionTasks
    PlatformCommandSurfaceOperation,
    // Platform management types
    PruneTerminalSessionTasksRequest,
    PruneTerminalSessionTasksResponse,
    RecordCircuitBreakerFailureRequest,
    RecordCircuitBreakerFailureResponse,
    RecordCircuitBreakerSuccessRequest,
    RecordCircuitBreakerSuccessResponse,
    RegisterDurableWorkerRequest,
    RegisterDurableWorkerResponse,
    ReportDurableWorkerChecksRequest,
    ReportDurableWorkerChecksResponse,
    ResolveFilesRequest,
    ResolveFilesResponse,
    ResolveImageRequest,
    ResolveImageResponse,
    ResolveImagesRequest,
    ResolveImagesResponse,
    ResolvedFileData,
    ResolvedImageData,
    SessionSqlDbExecuteRequest,
    SessionSqlDbExecuteResponse,
    SessionSqlDbQueryRequest,
    SessionSqlDbQueryResponse,
    SetSessionStatusRequest,
    SetSessionStatusResponse,
    SetSessionTitleRequest,
    SetSessionTitleResponse,
    SubscribeTaskNotificationsRequest,
    TaskNotification,
    TaskNotificationType,
};
use everruns_internal_protocol::{
    WorkerService, WorkerServiceServer,
    datetime_to_proto_timestamp as ip_datetime_to_proto_timestamp, proto_event_request_to_schema,
    schema_event_to_proto,
};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status, Streaming};

/// Read the gRPC auth token from the environment. When set, all gRPC
/// requests must include `authorization: Bearer <token>` metadata.
/// Returns `None` if `WORKER_GRPC_AUTH_TOKEN` is unset (auth disabled — dev only).
pub fn grpc_auth_token_from_env() -> Option<String> {
    std::env::var("WORKER_GRPC_AUTH_TOKEN")
        .ok()
        .filter(|t| !t.is_empty())
}

/// TM-DURABLE-002: Validate that WORKER_GRPC_AUTH_TOKEN is set in production.
/// Panics if the token is missing when not in dev mode.
pub fn require_grpc_auth_token() -> String {
    require_grpc_auth_token_result().unwrap_or_else(|error| panic!("{error}"))
}

/// Fallible variant used by server startup so invalid infrastructure config
/// fails before the gRPC server is spawned.
pub fn require_grpc_auth_token_result() -> anyhow::Result<String> {
    match grpc_auth_token_from_env() {
        Some(token) => Ok(token),
        None => anyhow::bail!(
            "WORKER_GRPC_AUTH_TOKEN must be set when not in dev mode. \
             Without it, gRPC endpoints are unauthenticated."
        ),
    }
}

/// Build server-side TLS config from environment variables for mutual TLS (mTLS).
///
/// When `WORKER_GRPC_TLS_CERT` and `WORKER_GRPC_TLS_KEY` are set, TLS is enabled.
/// When `WORKER_GRPC_TLS_CA_CERT` is also set, client certificate verification is
/// enforced (mutual TLS).
///
/// Returns `None` when TLS env vars are not configured (plain HTTP/2).
pub fn grpc_server_tls_from_env() -> Option<tonic::transport::ServerTlsConfig> {
    grpc_server_tls_from_env_result().unwrap_or_else(|error| panic!("{error}"))
}

/// Fallible variant used by server startup to validate TLS material before
/// spawning the gRPC server task.
pub fn grpc_server_tls_from_env_result() -> anyhow::Result<Option<tonic::transport::ServerTlsConfig>>
{
    use tonic::transport::{Certificate, Identity, ServerTlsConfig};

    let cert_path = std::env::var("WORKER_GRPC_TLS_CERT")
        .ok()
        .filter(|s| !s.is_empty());
    let Some(cert_path) = cert_path else {
        return Ok(None);
    };
    let key_path = std::env::var("WORKER_GRPC_TLS_KEY")
        .ok()
        .filter(|s| !s.is_empty());
    let Some(key_path) = key_path else {
        return Ok(None);
    };

    let cert_pem = std::fs::read_to_string(&cert_path)
        .map_err(|e| anyhow::anyhow!("Failed to read WORKER_GRPC_TLS_CERT at {cert_path}: {e}"))?;
    let key_pem = std::fs::read_to_string(&key_path)
        .map_err(|e| anyhow::anyhow!("Failed to read WORKER_GRPC_TLS_KEY at {key_path}: {e}"))?;

    let identity = Identity::from_pem(cert_pem, key_pem);
    let mut tls = ServerTlsConfig::new().identity(identity);

    // When CA cert is provided, require client certificates (mTLS)
    if let Some(ca_path) = std::env::var("WORKER_GRPC_TLS_CA_CERT")
        .ok()
        .filter(|s| !s.is_empty())
    {
        let ca_pem = std::fs::read_to_string(&ca_path).map_err(|e| {
            anyhow::anyhow!("Failed to read WORKER_GRPC_TLS_CA_CERT at {ca_path}: {e}")
        })?;
        tls = tls.client_ca_root(Certificate::from_pem(ca_pem));
        tracing::info!("gRPC mTLS enabled: server cert + client CA verification");
    } else {
        tracing::info!("gRPC TLS enabled: server cert only (no client verification)");
    }

    Ok(Some(tls))
}

/// Tonic interceptor that validates bearer token on every gRPC request.
/// If `expected_token` is `None`, all requests are allowed (dev mode).
///
/// THREAT[TM-AUTHZ-002]: This bearer token is the ONLY thing guarding the
/// worker control-plane boundary. It is a single shared secret with no
/// per-org scoping, so any client that presents it is fully trusted: every
/// worker RPC builds `Caller::internal(req.org_id)` from the client-supplied
/// `org_id`, which bypasses all policy checks. A single valid token can
/// therefore act on any org. The worker boundary MUST never be reachable from
/// untrusted networks. The token is compared in constant time below to avoid
/// leaking it via a timing side-channel.
#[derive(Clone)]
pub struct GrpcAuthInterceptor {
    expected_token: Option<String>,
}

impl GrpcAuthInterceptor {
    pub fn new(expected_token: Option<String>) -> Self {
        Self { expected_token }
    }
}

impl tonic::service::Interceptor for GrpcAuthInterceptor {
    fn call(&mut self, request: Request<()>) -> Result<Request<()>, Status> {
        let Some(ref expected) = self.expected_token else {
            return Ok(request); // No auth configured
        };

        let token = request
            .metadata()
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "));

        match token {
            // Constant-time compare: the worker token is a shared secret, so a
            // short-circuiting `==` would leak it byte-by-byte via timing.
            Some(t) if crate::security::constant_time_eq(t.as_bytes(), expected.as_bytes()) => {
                Ok(request)
            }
            Some(_) => {
                note_rejected_worker();
                Err(Status::unauthenticated("Invalid gRPC auth token"))
            }
            None => {
                note_rejected_worker();
                Err(Status::unauthenticated("Missing gRPC auth token"))
            }
        }
    }
}

/// Worker calls this process refused for a missing or wrong token, and when
/// the last one was. Until now only the worker's own log showed a token
/// mismatch; the system doctor reads these.
static REJECTED_WORKER_CALLS: AtomicU64 = AtomicU64::new(0);
static LAST_REJECTED_WORKER_CALL: AtomicI64 = AtomicI64::new(0);

fn note_rejected_worker() {
    REJECTED_WORKER_CALLS.fetch_add(1, Ordering::Relaxed);
    LAST_REJECTED_WORKER_CALL.store(chrono::Utc::now().timestamp(), Ordering::Relaxed);
}

/// How many worker calls this process refused since it started, and when it
/// last refused one.
pub(crate) fn rejected_worker_calls() -> (u64, Option<chrono::DateTime<chrono::Utc>>) {
    let last = match LAST_REJECTED_WORKER_CALL.load(Ordering::Relaxed) {
        0 => None,
        at => chrono::DateTime::from_timestamp(at, 0),
    };
    (REJECTED_WORKER_CALLS.load(Ordering::Relaxed), last)
}

/// gRPC service implementation for worker communication
///
/// Uses domain commands/queries for agents and harnesses, and services for
/// sessions, events, and other domains not yet migrated.
pub struct WorkerServiceImpl {
    event_service: EventService,
    session_service: Arc<SessionService>,
    session_file_service: Arc<WorkspaceFileService>,
    provider_resolver_service: Arc<ProviderResolverService>,
    mcp_server_service: McpServerService,
    capability_service: Arc<CapabilityService>,
    durable_store: Option<Arc<PostgresWorkflowEventStore>>,
    /// Task notification broadcaster for push-based notifications (PG NOTIFY or NATS)
    task_broadcaster: Option<Arc<TaskBroadcaster>>,
    /// Storage backend for image resolution
    db: Arc<StorageBackend>,
    /// Encryption service for domains that decrypt stored config blobs.
    encryption: Option<Arc<EncryptionService>>,
    /// Session storage store for key/value and secret operations
    session_storage_store: Option<Arc<dyn everruns_core::session_services::SessionStorageStore>>,
    /// Session SQL database store for session-scoped databases
    sqldb_store: Option<Arc<dyn everruns_contracts::session_sqldb::SessionSqlDbStore>>,
    /// Agent runner for triggering turn workflows (platform management send_message)
    runner: Option<Arc<dyn everruns_core::host::TurnBackend>>,
    /// Lazy connection token resolver (decrypts stored tokens / mints GitHub App tokens)
    connection_resolver:
        Option<Arc<dyn everruns_core::connection_services::UserConnectionResolver>>,
    /// Signs the paths workers GET image and file bytes from. Set by
    /// [`Self::into_routes`], which also serves those paths.
    blob_links: Option<crate::api::internal_blobs::BlobLinkSigner>,
    /// Budget service for check_budget tool
    budget_service: Arc<crate::domains::budgets::BudgetService>,
    /// Central org rate limiter for gRPC-backed outbound tool calls.
    org_rate_limiter: Option<Arc<crate::auth::rate_limit::OrgRateLimiter>>,
    /// Active permission resolver for user-scoped command execution over gRPC.
    /// The channel host targeted schedule triggers run on.
    channels: Option<Arc<crate::channels::host::ServerChannels>>,
    slack_provisioner: Option<
        Arc<dyn crate::domains::agent_channels::record::slack_provisioning::SlackAppProvisioner>,
    >,
    permission_resolver: Arc<dyn PermissionResolver>,
    /// System utility LLM for sanctioned internal analysis commands.
    utility_llm_service: Arc<dyn everruns_core::UtilityLlmService>,
    connector_registry: everruns_contracts::connector::ConnectorRegistry,
    /// Provider-domain services for provider commands the worker dispatches.
    provider_services: crate::domains::providers::ProviderServices,
}

impl WorkerServiceImpl {
    pub fn new(
        event_service: EventService,
        db: Arc<StorageBackend>,
        encryption: Option<Arc<EncryptionService>>,
        runner: Option<Arc<dyn everruns_core::host::TurnBackend>>,
        host_composition: HostComposition,
    ) -> Self {
        Self::with_virtual_registry(
            event_service,
            db,
            encryption,
            runner,
            host_composition,
            None,
            None,
        )
    }

    pub fn with_virtual_registry(
        event_service: EventService,
        db: Arc<StorageBackend>,
        encryption: Option<Arc<EncryptionService>>,
        runner: Option<Arc<dyn everruns_core::host::TurnBackend>>,
        host_composition: HostComposition,
        virtual_registry: Option<
            Arc<crate::domains::session_files::virtual_mount_registry::VirtualMountRegistry>,
        >,
        provider_resolver_service: Option<Arc<ProviderResolverService>>,
    ) -> Self {
        let capability_registry = (*host_composition.capability_registry()).clone();
        let session_service = {
            let svc = SessionService::with_registry(db.clone(), capability_registry.clone());
            if let Some(ref reg) = virtual_registry {
                svc.with_virtual_registry(reg.clone())
            } else {
                svc
            }
        };
        let session_file_service = Arc::new(if let Some(ref reg) = virtual_registry {
            WorkspaceFileService::new(db.clone()).with_virtual_registry(reg.clone())
        } else {
            WorkspaceFileService::new(db.clone())
        });
        let provider_resolver_service = provider_resolver_service.unwrap_or_else(|| {
            Arc::new(
                ProviderResolverService::new(db.clone(), encryption.clone())
                    .with_driver_registry(host_composition.driver_registry().clone()),
            )
        });
        // Built over the shared resolver when it stands alone; `app_builder`
        // replaces it with the `/v1/providers` instances (EVE-1234).
        let provider_services = crate::domains::providers::ProviderServices::new(
            db.clone(),
            encryption.clone(),
            Arc::new(host_composition.driver_registry().clone()),
            Some(provider_resolver_service.clone()),
        );
        let mcp_server_service = McpServerService::with_egress_service(
            db.clone(),
            encryption.clone(),
            host_composition.egress_service(),
        );
        // The platform's egress boundary, so commands reaching outbound HTTP
        // through it (MCP discovery, Poppy calls) get the same policy and
        // audit log as the in-process worker.
        let capability_service = Arc::new(
            CapabilityService::with_registry(db.clone(), encryption.clone(), capability_registry)
                .with_mcp_egress_service(host_composition.egress_service()),
        );
        let utility_llm_service = host_composition.utility_llm_service();
        // Hosted connector catalog (EVE-879): defaults to the OSS inventory
        // preset; `ServerAppBuilder` overrides via `set_connector_registry`.
        let connector_registry = crate::platform::oss_connector_registry();

        let durable_store = Some(Arc::new(PostgresWorkflowEventStore::new(db.pool().clone())));

        let database = crate::storage::Database::new(db.pool().clone());
        let session_storage_store: Option<
            Arc<dyn everruns_core::session_services::SessionStorageStore>,
        > = Some(if let Some(enc) = &encryption {
            Arc::new(crate::storage::create_db_session_storage_store(
                database,
                enc.as_ref().clone(),
            ))
        } else {
            Arc::new(crate::storage::create_db_session_storage_store_without_encryption(database))
        });

        // Create connection resolver (requires encryption for token decryption)
        let connection_resolver: Option<
            Arc<dyn everruns_core::connection_services::UserConnectionResolver>,
        > = encryption.as_ref().map(|enc| {
            // GitHub App token minting requires env vars (set in production via Doppler)
            let github_app_minter = Self::github_app_minter_from_env();
            Arc::new(crate::storage::DbConnectionResolver::new(
                db.as_ref().clone(),
                enc.as_ref().clone(),
                github_app_minter,
                host_composition.egress_service(),
            )) as Arc<dyn everruns_core::connection_services::UserConnectionResolver>
        });

        // Session SQL databases persist to PostgreSQL. The server replaces this
        // with the store its HTTP routes share (`set_sqldb_store`); both read
        // the same rows, so a standalone instance stays consistent too.
        let sqldb_store: Option<Arc<dyn everruns_contracts::session_sqldb::SessionSqlDbStore>> =
            Some(Arc::new(crate::session_sqldb::PostgresSqlDbStore::new(
                db.database().clone(),
            )));

        let budget_service = Arc::new(crate::domains::budgets::BudgetService::new(db.clone()));

        Self {
            event_service,
            session_service: Arc::new(session_service),
            session_file_service,
            provider_resolver_service,
            mcp_server_service,
            capability_service,
            durable_store,
            task_broadcaster: None, // Set via set_task_broadcaster() after async initialization
            db,
            encryption,
            session_storage_store,
            sqldb_store,
            runner,
            connection_resolver,
            blob_links: None,
            budget_service,
            org_rate_limiter: None,
            slack_provisioner: None,
            channels: None,
            permission_resolver: Arc::new(everruns_core::DefaultPermissionResolver),
            utility_llm_service,
            connector_registry,
            provider_services,
        }
    }

    /// Set the task notification broadcaster (must be called after async initialization)
    pub fn set_connector_registry(
        &mut self,
        registry: everruns_contracts::connector::ConnectorRegistry,
    ) {
        self.connector_registry = registry;
    }

    pub fn set_task_broadcaster(&mut self, broadcaster: Arc<TaskBroadcaster>) {
        self.task_broadcaster = Some(broadcaster);
    }

    pub fn set_org_rate_limiter(&mut self, limiter: Arc<crate::auth::rate_limit::OrgRateLimiter>) {
        self.org_rate_limiter = Some(limiter);
    }

    /// Share the HTTP app's session SQL database store with the worker.
    ///
    /// Without this the constructor's own in-memory backend stands, and the two
    /// never share state: a database an agent created through the worker is
    /// invisible to `GET /v1/sessions/{id}/databases`, which answers `200` with
    /// an empty list rather than an error (EVE-1047). Both run in the same
    /// process — `app_builder` spawns the gRPC server — so one store is all
    /// that was ever needed.
    ///
    /// Injected rather than made a process-wide singleton so tests that want
    /// their own isolated backend still get one.
    pub fn set_sqldb_store(
        &mut self,
        store: Arc<dyn everruns_contracts::session_sqldb::SessionSqlDbStore>,
    ) {
        self.sqldb_store = Some(store);
    }

    pub fn set_slack_provisioner(
        &mut self,
        provisioner: Option<
            Arc<
                dyn crate::domains::agent_channels::record::slack_provisioning::SlackAppProvisioner,
            >,
        >,
    ) {
        self.slack_provisioner = provisioner;
    }

    pub fn set_channels(&mut self, channels: Option<Arc<crate::channels::host::ServerChannels>>) {
        self.channels = channels;
    }

    pub fn set_permission_resolver(&mut self, resolver: Arc<dyn PermissionResolver>) {
        self.permission_resolver = resolver;
    }

    /// Share the `/v1/providers` service instances, so a provider write over
    /// the worker invalidates the same resolver cache the routes do.
    pub fn set_provider_services(&mut self, services: crate::domains::providers::ProviderServices) {
        self.provider_services = services;
    }

    /// Signed path to an image or file attachment's bytes on this endpoint.
    fn blob_url(
        &self,
        kind: crate::api::internal_blobs::BlobKind,
        id: uuid::Uuid,
        org_id: i64,
    ) -> Result<String, Status> {
        self.blob_links
            .as_ref()
            .map(|signer| signer.path(kind, id, org_id))
            .ok_or_else(|| {
                Status::failed_precondition("this endpoint does not serve worker byte paths")
            })
    }

    fn domain_ctx_for_caller(&self, caller: everruns_core::Caller) -> crate::domains::common::Ctx {
        let resolver: Arc<dyn PermissionResolver> = if caller.is_internal {
            Arc::new(everruns_core::DefaultPermissionResolver)
        } else {
            self.permission_resolver.clone()
        };

        let mut ctx = crate::domains::common::Ctx::new(
            caller,
            self.db.clone(),
            self.capability_service.clone(),
            self.encryption.clone(),
            resolver,
        )
        .with_slack_provisioner(self.slack_provisioner.clone())
        .with_connector_registry(self.connector_registry.clone())
        .with_utility_llm_service(self.utility_llm_service.clone())
        .with_workflow_store(
            self.durable_store
                .clone()
                .map(|store| store as Arc<dyn WorkflowEventStore + Send + Sync>),
        )
        .with_session_service(self.session_service.clone())
        // The worker's connection and MCP-credential lookups run as internal
        // commands that decrypt and refresh grants through this resolver.
        .with_connection_resolver(self.connection_resolver.clone())
        // The worker's file operations run as `session_files` commands, and
        // those resolve their store from the ctx. Without this they would build
        // a bare `WorkspaceFileService` and the session's virtual mounts would
        // be invisible to the agent — the RPCs this replaced always carried the
        // registry-aware service.
        .with_session_file_service(self.session_file_service.clone())
        // Wire the event service so event-backed platform commands (e.g.
        // list_events, used by subagent/handoff idle-settling) work over the
        // gRPC worker dispatch, not just the in-process direct path.
        .with_event_service(Arc::new(self.event_service.clone()))
        // `sync_provider_models` answered 503 here and provider create/update
        // skipped model discovery without these (EVE-1234).
        .with_provider_services(&self.provider_services);

        // The session-database commands are the worker's only path to sqldb since
        // the bespoke RPCs went away, so a Ctx without this store fails every one
        // of them with "not configured".
        if let Some(store) = &self.sqldb_store {
            ctx = ctx.with_sqldb_store(store.clone());
        }

        if let Some(runner) = &self.runner {
            let message_service = Arc::new(crate::domains::messages::MessageService::new(
                self.db.clone(),
                runner.clone(),
                self.event_service.event_delivery().clone(),
            ));
            ctx = ctx
                .with_message_service(message_service)
                .with_runner(runner.clone());
        }

        ctx
    }

    /// Build a domain command context using the organization's effective feature flags.
    async fn org_domain_ctx_for_caller(
        &self,
        caller: everruns_core::Caller,
    ) -> Result<crate::domains::common::Ctx, Status> {
        let org_id = caller.org_id;
        let feature_flags = crate::services::org_feature_flags::resolve_org_feature_flags(
            &self.db,
            org_id,
            &crate::records::FeatureFlagPolicy::current(),
        )
        .await
        .map_err(|error| {
            tracing::error!(%error, org_id, "Failed to resolve gRPC command feature flags");
            Status::internal("Failed to resolve organization feature flags")
        })?;

        Ok(self
            .domain_ctx_for_caller(caller)
            .with_feature_flags(feature_flags))
    }

    /// Get durable store or return unavailable error
    #[allow(clippy::result_large_err)] // tonic::Status is the standard gRPC error type
    fn durable_store(&self) -> Result<&Arc<PostgresWorkflowEventStore>, Status> {
        self.durable_store
            .as_ref()
            .ok_or_else(|| Status::unavailable("Durable execution not enabled"))
    }

    /// Get session storage store or return unavailable error
    #[allow(clippy::result_large_err)]
    fn storage_store(
        &self,
    ) -> Result<&Arc<dyn everruns_core::session_services::SessionStorageStore>, Status> {
        self.session_storage_store
            .as_ref()
            .ok_or_else(|| Status::unavailable("Session storage not available"))
    }

    /// Build a GitHubAppTokenMinter from environment variables (if configured).
    fn github_app_minter_from_env() -> Option<crate::storage::GitHubAppTokenMinter> {
        let app_id = std::env::var("GITHUB_APP_ID")
            .ok()
            .filter(|s| !s.is_empty())?;
        let private_key = std::env::var("GITHUB_APP_PRIVATE_KEY")
            .ok()
            .filter(|s| !s.is_empty())?;
        Some(
            crate::storage::GitHubAppTokenMinter::new(app_id, private_key)
                .with_user_client(crate::github_user_token::GitHubUserClient::from_env()),
        )
    }

    /// Get session SQL database store or return unavailable error
    #[allow(clippy::result_large_err)]
    fn sqldb_store(
        &self,
    ) -> Result<&Arc<dyn everruns_contracts::session_sqldb::SessionSqlDbStore>, Status> {
        self.sqldb_store
            .as_ref()
            .ok_or_else(|| Status::unavailable("Session SQL database not available"))
    }

    /// Create a tonic server for this service
    pub fn into_server(self) -> WorkerServiceServer<Self> {
        WorkerServiceServer::new(self)
            .max_decoding_message_size(everruns_internal_protocol::MAX_GRPC_MESSAGE_SIZE)
            .max_encoding_message_size(everruns_internal_protocol::MAX_GRPC_MESSAGE_SIZE)
    }

    /// The whole worker endpoint: this gRPC service plus the signed paths
    /// workers GET image and file bytes from (`api::internal_blobs`). One
    /// secret keys both the paths this service hands out and the route that
    /// checks them, so they can never disagree.
    pub fn into_routes(mut self, signing_secret: String) -> tonic::service::Routes {
        let db = self.db.clone();
        self.blob_links = Some(crate::api::internal_blobs::BlobLinkSigner::new(
            signing_secret.clone(),
        ));
        tonic::service::Routes::new(self.into_server())
            .into_axum_router()
            .merge(crate::api::internal_blobs::routes(
                crate::api::internal_blobs::AppState { db, signing_secret },
            ))
            .into()
    }

    /// Convert ResolvedModel to proto ResolvedModel
    fn resolved_model_to_proto(resolved: crate::services::ResolvedModel) -> proto::ResolvedModel {
        proto::ResolvedModel {
            model: resolved.model_id,
            provider_id: resolved.provider_id,
            provider_type: resolved.provider_type,
        }
    }

    fn image_info_row_to_proto(row: crate::storage::ImageInfoRow) -> proto::StoredImageInfo {
        proto::StoredImageInfo {
            id: Some(proto::Uuid {
                value: row.id.uuid().to_string(),
            }),
            filename: row.filename,
            content_type: row.content_type,
            size_bytes: row.size_bytes,
            metadata: Some(everruns_internal_protocol::json_to_proto_struct(
                &row.metadata,
            )),
            created_at: Some(ip_datetime_to_proto_timestamp(row.created_at)),
        }
    }

    /// Build MCP tool definitions from agent's MCP capabilities.
    ///
    /// Shares the mapping with the in-process worker
    /// (`tool_labels::org_mcp_tool_definitions`): one batch fetch of servers,
    /// one query for saved labels, and the hints travel in the proto.
    async fn build_mcp_tool_definitions(
        &self,
        org_id: i64,
        agent: &crate::domains::agents::record::Agent,
    ) -> Vec<McpToolDef> {
        crate::domains::mcp_servers::tool_labels::org_mcp_tool_definitions(
            &self.mcp_server_service,
            &self.db,
            org_id,
            agent.capabilities.iter().map(|cap| cap.capability_id()),
        )
        .await
        .iter()
        .map(worker::support::mcp_tool_definition_to_proto)
        .collect()
    }
}

// Helper to convert uuid parse error to tonic status
#[allow(clippy::result_large_err)] // tonic::Status is the standard gRPC error type
fn parse_uuid(proto_uuid: Option<&proto::Uuid>) -> Result<uuid::Uuid, Status> {
    let uuid_str = proto_uuid
        .map(|u| &u.value)
        .ok_or_else(|| Status::invalid_argument("Missing UUID"))?;
    uuid::Uuid::parse_str(uuid_str)
        .map_err(|e| Status::invalid_argument(format!("Invalid UUID: {}", e)))
}

/// Convert a leased resource to proto representation.
fn leased_resource_to_proto(
    org_id: i64,
    s: &everruns_core::LeasedResource,
) -> proto::LeasedResourceProto {
    use everruns_internal_protocol::datetime_to_proto_timestamp;

    proto::LeasedResourceProto {
        id: Some(proto::Uuid {
            value: s.id.uuid().to_string(),
        }),
        session_id: s.session_id.map(|id| proto::Uuid {
            value: id.uuid().to_string(),
        }),
        provider: s.provider.clone(),
        resource_type: s.resource_type.clone(),
        external_id: s.external_id.clone(),
        display_name: s.display_name.clone(),
        status: s.status.to_string(),
        owner_user_id: s.owner_user_id.map(|id| proto::Uuid {
            value: id.to_string(),
        }),
        connection_id: s.connection_id.map(|id| proto::Uuid {
            value: id.to_string(),
        }),
        lease_duration_seconds: s.lease_duration_seconds,
        last_touched_at: Some(datetime_to_proto_timestamp(s.last_touched_at)),
        lease_expires_at: Some(datetime_to_proto_timestamp(s.lease_expires_at)),
        cleanup_started_at: s.cleanup_started_at.map(datetime_to_proto_timestamp),
        cleanup_completed_at: s.cleanup_completed_at.map(datetime_to_proto_timestamp),
        cleanup_attempts: s.cleanup_attempts,
        last_cleanup_error: s.last_cleanup_error.clone(),
        metadata: Some(everruns_internal_protocol::json_to_proto_struct(
            &s.metadata,
        )),
        created_at: Some(datetime_to_proto_timestamp(s.created_at)),
        updated_at: Some(datetime_to_proto_timestamp(s.updated_at)),
        org_id,
    }
}

/// Convert a SessionSqlDbError to a gRPC Status with appropriate error codes.
fn sqldb_error_to_status(e: everruns_contracts::session_sqldb::SessionSqlDbError) -> Status {
    use everruns_contracts::session_sqldb::SessionSqlDbError;
    match &e {
        SessionSqlDbError::DatabaseNotFound(_) => Status::not_found(e.to_string()),
        SessionSqlDbError::DatabaseAlreadyExists(_) => Status::already_exists(e.to_string()),
        SessionSqlDbError::InvalidDatabaseName(_) => Status::invalid_argument(e.to_string()),
        SessionSqlDbError::LimitExceeded(_) => Status::resource_exhausted(e.to_string()),
        SessionSqlDbError::QueryTimeout(_) => Status::deadline_exceeded(e.to_string()),
        SessionSqlDbError::AuthorizerBlocked(_) => Status::permission_denied(e.to_string()),
        SessionSqlDbError::QueryError(_) | SessionSqlDbError::ResultTooLarge(_) => {
            Status::failed_precondition(e.to_string())
        }
        SessionSqlDbError::Internal(_) => {
            tracing::error!("Internal sqldb error: {}", e);
            Status::internal("Internal SQL database error")
        }
    }
}

/// Convert a serde_json::Value to a proto Value for query result rows.
fn json_value_to_proto(value: serde_json::Value) -> prost_types::Value {
    let kind = match value {
        serde_json::Value::Null => Some(prost_types::value::Kind::NullValue(0)),
        serde_json::Value::Bool(b) => Some(prost_types::value::Kind::BoolValue(b)),
        serde_json::Value::Number(n) => Some(prost_types::value::Kind::NumberValue(
            n.as_f64().unwrap_or(0.0),
        )),
        serde_json::Value::String(s) => Some(prost_types::value::Kind::StringValue(s)),
        serde_json::Value::Array(arr) => Some(prost_types::value::Kind::ListValue(
            prost_types::ListValue {
                values: arr.into_iter().map(json_value_to_proto).collect(),
            },
        )),
        serde_json::Value::Object(map) => {
            Some(prost_types::value::Kind::StructValue(prost_types::Struct {
                fields: map
                    .into_iter()
                    .map(|(k, v)| (k, json_value_to_proto(v)))
                    .collect(),
            }))
        }
    };
    prost_types::Value { kind }
}

/// Convert a domain `RuntimeMessage` to a proto `Message`.
fn message_to_proto(message: &everruns_core::RuntimeMessage) -> proto::Message {
    everruns_internal_protocol::schema_message_to_proto(message)
}

/// Extract a RuntimeMessage from an Event's data field
///
/// Events returned from EventService already have data parsed into EventData.
pub(crate) fn event_to_message(
    event: &everruns_core::Event,
) -> Option<everruns_core::RuntimeMessage> {
    use everruns_core::EventData;

    match &event.data {
        EventData::InputMessage(d) => Some(d.message.clone()),
        EventData::OutputMessageCompleted(d) => Some(d.message.clone()),
        EventData::ToolCompleted(d) => Some(
            crate::storage::runtime::message::tool_completed_to_message(d.clone()),
        ),
        _ => None,
    }
}

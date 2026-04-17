// Internal gRPC Service for Worker Communication
//
// Decision: Workers communicate with control plane via gRPC for all database operations
// Decision: This provides a clean boundary and simplifies worker deployment
// Decision: gRPC service uses the same services layer as HTTP API for consistency
// Decision: Operations go through domain commands/queries or services layer
// Decision: Split into submodules for maintainability (EVE-101).

mod worker_service_impl;

#[cfg(test)]
mod tests;

use crate::services::{
    CapabilityService, EventService, LlmResolverService, McpServerService, SessionFileService,
    SessionService,
    session_file::{CreateDirectoryInput, CreateFileInput, GrepInput, UpdateFileInput},
};
use crate::storage::{EncryptionService, StorageBackend};
use crate::task_notifications::TaskBroadcaster;
use base64::Engine;
use everruns_core::PlatformDefinition;
use everruns_durable::{
    ActivityOptions, CircuitBreakerConfig, CircuitState, DistributedCircuitBreaker,
    PostgresWorkflowEventStore, StoreError, TaskDefinition, TaskFailureOutcome, WorkerInfo,
    WorkflowError, WorkflowEvent, WorkflowEventStore, WorkflowStatus, append_event,
    record_activity_completed, record_activity_failed, record_activity_started,
    record_workflow_cancelled, record_workflow_completed, record_workflow_failed,
};
use everruns_internal_protocol::proto::{
    self,
    AddMessageRequest,
    AddMessageResponse,
    BudgetSummaryProto,
    // Session schedule operations
    CancelSessionScheduleRequest,
    CancelSessionScheduleResponse,
    // Budget operations
    CheckBudgetsForSessionRequest,
    CheckBudgetsForSessionResponse,
    CheckCircuitBreakerRequest,
    CheckCircuitBreakerResponse,
    CircuitBreakerState as ProtoCircuitBreakerState,
    // Connection token resolution
    ClaimDueLeasedResourcesRequest,
    ClaimDueLeasedResourcesResponse,
    ClaimDurableTasksRequest,
    ClaimDurableTasksResponse,
    CommitExecRequest,
    CommitExecResponse,
    CompleteDurableTaskRequest,
    CompleteDurableTaskResponse,
    CountActiveDurableWorkflowsRequest,
    CountActiveDurableWorkflowsResponse,
    CountActiveSessionSchedulesRequest,
    CountActiveSessionSchedulesResponse,
    CreateDurableWorkflowRequest,
    CreateDurableWorkflowResponse,
    CreateSessionScheduleRequest,
    CreateSessionScheduleResponse,
    DeregisterDurableWorkerRequest,
    DeregisterDurableWorkerResponse,
    // Session resource registry
    DeregisterSessionResourceRequest,
    DeregisterSessionResourceResponse,
    DurableWorkflowSignal as ProtoDurableWorkflowSignal,
    DurableWorkflowStatus,
    EmitEventRequest,
    EmitEventResponse,
    EmitEventStreamResponse,
    EnqueueDurableTaskRequest,
    EnqueueDurableTaskResponse,
    FailDurableTaskRequest,
    FailDurableTaskResponse,
    GetAgentRequest,
    GetAgentResponse,
    GetAndConsumeDurableWorkflowSignalsRequest,
    GetAndConsumeDurableWorkflowSignalsResponse,
    GetConnectionTokenForUserRequest,
    GetConnectionTokenForUserResponse,
    GetConnectionTokenRequest,
    GetConnectionTokenResponse,
    GetConnectionUserRequest,
    GetConnectionUserResponse,
    GetDefaultModelRequest,
    GetDefaultModelResponse,
    GetDurableWorkflowStatusRequest,
    GetDurableWorkflowStatusResponse,
    GetHarnessRequest,
    GetHarnessResponse,
    GetMcpServerByPrefixRequest,
    GetMcpServerByPrefixResponse,
    GetMessageRequest,
    GetMessageResponse,
    GetModelWithProviderRequest,
    GetModelWithProviderResponse,
    GetSessionRequest,
    GetSessionResponse,
    GetTurnContextRequest,
    GetTurnContextResponse,
    HeartbeatDurableTaskRequest,
    HeartbeatDurableTaskResponse,
    HeartbeatDurableWorkerRequest,
    HeartbeatDurableWorkerResponse,
    ListSessionLeasedResourcesRequest,
    ListSessionLeasedResourcesResponse,
    ListSessionResourcesRequest,
    ListSessionResourcesResponse,
    ListSessionSchedulesRequest,
    ListSessionSchedulesResponse,
    LoadMessagesRequest,
    LoadMessagesResponse,
    MarkLeasedResourceCleanupFailedRequest,
    MarkLeasedResourceCleanupFailedResponse,
    MarkLeasedResourceReleasedRequest,
    MarkLeasedResourceReleasedResponse,
    McpServerInfo,
    McpToolDef,
    PlatformCapabilityInfo,
    // Platform management types
    PlatformCopyHarnessRequest,
    PlatformCopyHarnessResponse,
    PlatformCreateAgentRequest,
    PlatformCreateAgentResponse,
    PlatformCreateHarnessRequest,
    PlatformCreateHarnessResponse,
    PlatformCreateSessionRequest,
    PlatformCreateSessionResponse,
    PlatformDeleteAgentRequest,
    PlatformDeleteAgentResponse,
    PlatformDeleteHarnessRequest,
    PlatformDeleteHarnessResponse,
    PlatformDeleteSessionRequest,
    PlatformDeleteSessionResponse,
    PlatformGetBaseUrlRequest,
    PlatformGetBaseUrlResponse,
    PlatformGetMessagesRequest,
    PlatformGetMessagesResponse,
    PlatformListAgentsRequest,
    PlatformListAgentsResponse,
    PlatformListCapabilitiesRequest,
    PlatformListCapabilitiesResponse,
    PlatformListHarnessesRequest,
    PlatformListHarnessesResponse,
    PlatformListSessionsRequest,
    PlatformListSessionsResponse,
    PlatformSendMessageRequest,
    PlatformSendMessageResponse,
    PlatformUpdateAgentRequest,
    PlatformUpdateAgentResponse,
    PlatformUpdateHarnessRequest,
    PlatformUpdateHarnessResponse,
    PlatformWaitForIdleRequest,
    PlatformWaitForIdleResponse,
    RecordCircuitBreakerFailureRequest,
    RecordCircuitBreakerFailureResponse,
    RecordCircuitBreakerSuccessRequest,
    RecordCircuitBreakerSuccessResponse,
    RegisterDurableWorkerRequest,
    RegisterDurableWorkerResponse,
    RegisterSessionResourceRequest,
    RegisterSessionResourceResponse,
    ReleaseLeasedResourceRequest,
    ReleaseLeasedResourceResponse,
    ResolveImageRequest,
    ResolveImageResponse,
    ResolveImagesRequest,
    ResolveImagesResponse,
    ResolvedImageData,
    SendDurableWorkflowSignalRequest,
    SendDurableWorkflowSignalResponse,
    SessionCreateDirectoryRequest,
    SessionCreateDirectoryResponse,
    SessionDeleteFileRequest,
    SessionDeleteFileResponse,
    SessionGrepFilesRequest,
    SessionGrepFilesResponse,
    SessionListDirectoryRequest,
    SessionListDirectoryResponse,
    SessionReadFileRequest,
    SessionReadFileResponse,
    SessionSqlDbCreateDatabaseRequest,
    SessionSqlDbCreateDatabaseResponse,
    SessionSqlDbDeleteDatabaseRequest,
    SessionSqlDbDeleteDatabaseResponse,
    SessionSqlDbExecuteRequest,
    SessionSqlDbExecuteResponse,
    SessionSqlDbGetDatabaseRequest,
    SessionSqlDbGetDatabaseResponse,
    SessionSqlDbListDatabasesRequest,
    SessionSqlDbListDatabasesResponse,
    SessionSqlDbQueryRequest,
    SessionSqlDbQueryResponse,
    SessionSqlDbSchemaRequest,
    SessionSqlDbSchemaResponse,
    SessionStatFileRequest,
    SessionStatFileResponse,
    SessionStorageDeleteSecretRequest,
    SessionStorageDeleteSecretResponse,
    SessionStorageDeleteValueRequest,
    SessionStorageDeleteValueResponse,
    SessionStorageGetSecretRequest,
    SessionStorageGetSecretResponse,
    SessionStorageGetValueRequest,
    SessionStorageGetValueResponse,
    SessionStorageListKeysRequest,
    SessionStorageListKeysResponse,
    SessionStorageListSecretsRequest,
    SessionStorageListSecretsResponse,
    SessionStorageSetSecretRequest,
    SessionStorageSetSecretResponse,
    SessionStorageSetValueRequest,
    SessionStorageSetValueResponse,
    SessionWriteFileIfContentMatchesRequest,
    SessionWriteFileIfContentMatchesResponse,
    SessionWriteFileRequest,
    SessionWriteFileResponse,
    SetSessionStatusRequest,
    SetSessionStatusResponse,
    SetSessionTitleRequest,
    SetSessionTitleResponse,
    SubscribeTaskNotificationsRequest,
    TaskNotification,
    TaskNotificationType,
    UpdateDurableWorkflowStatusRequest,
    UpdateDurableWorkflowStatusResponse,
    UpdateSessionResourceStatusRequest,
    UpdateSessionResourceStatusResponse,
    UpsertLeasedResourceRequest,
    UpsertLeasedResourceResponse,
};
use everruns_internal_protocol::{
    WorkerService, WorkerServiceServer,
    datetime_to_proto_timestamp as ip_datetime_to_proto_timestamp, proto_event_request_to_schema,
    schema_agent_to_proto, schema_event_to_proto, schema_harness_to_proto, schema_session_to_proto,
};
use std::pin::Pin;
use std::sync::Arc;
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
    grpc_auth_token_from_env().unwrap_or_else(|| {
        panic!(
            "WORKER_GRPC_AUTH_TOKEN must be set when not in dev mode. \
             Without it, gRPC endpoints are unauthenticated."
        )
    })
}

/// Build server-side TLS config from environment variables for mutual TLS (mTLS).
///
/// When `WORKER_GRPC_TLS_CERT` and `WORKER_GRPC_TLS_KEY` are set, TLS is enabled.
/// When `WORKER_GRPC_TLS_CA_CERT` is also set, client certificate verification is
/// enforced (mutual TLS).
///
/// Returns `None` when TLS env vars are not configured (plain HTTP/2).
pub fn grpc_server_tls_from_env() -> Option<tonic::transport::ServerTlsConfig> {
    use tonic::transport::{Certificate, Identity, ServerTlsConfig};

    let cert_path = std::env::var("WORKER_GRPC_TLS_CERT")
        .ok()
        .filter(|s| !s.is_empty())?;
    let key_path = std::env::var("WORKER_GRPC_TLS_KEY")
        .ok()
        .filter(|s| !s.is_empty())?;

    let cert_pem = std::fs::read_to_string(&cert_path)
        .unwrap_or_else(|e| panic!("Failed to read WORKER_GRPC_TLS_CERT at {cert_path}: {e}"));
    let key_pem = std::fs::read_to_string(&key_path)
        .unwrap_or_else(|e| panic!("Failed to read WORKER_GRPC_TLS_KEY at {key_path}: {e}"));

    let identity = Identity::from_pem(cert_pem, key_pem);
    let mut tls = ServerTlsConfig::new().identity(identity);

    // When CA cert is provided, require client certificates (mTLS)
    if let Some(ca_path) = std::env::var("WORKER_GRPC_TLS_CA_CERT")
        .ok()
        .filter(|s| !s.is_empty())
    {
        let ca_pem = std::fs::read_to_string(&ca_path)
            .unwrap_or_else(|e| panic!("Failed to read WORKER_GRPC_TLS_CA_CERT at {ca_path}: {e}"));
        tls = tls.client_ca_root(Certificate::from_pem(ca_pem));
        tracing::info!("gRPC mTLS enabled: server cert + client CA verification");
    } else {
        tracing::info!("gRPC TLS enabled: server cert only (no client verification)");
    }

    Some(tls)
}

/// Tonic interceptor that validates bearer token on every gRPC request.
/// If `expected_token` is `None`, all requests are allowed (dev mode).
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
            Some(t) if t == expected => Ok(request),
            Some(_) => Err(Status::unauthenticated("Invalid gRPC auth token")),
            None => Err(Status::unauthenticated("Missing gRPC auth token")),
        }
    }
}

/// gRPC service implementation for worker communication
///
/// Uses domain commands/queries for agents and harnesses, and services for
/// sessions, events, and other domains not yet migrated.
pub struct WorkerServiceImpl {
    event_service: EventService,
    session_service: SessionService,
    session_file_service: SessionFileService,
    llm_resolver_service: LlmResolverService,
    mcp_server_service: McpServerService,
    capability_service: Arc<CapabilityService>,
    durable_store: Option<Arc<PostgresWorkflowEventStore>>,
    /// Task notification broadcaster for push-based notifications (PG NOTIFY or NATS)
    task_broadcaster: Option<Arc<TaskBroadcaster>>,
    /// Storage backend for image resolution
    db: Arc<StorageBackend>,
    /// Session storage store for key/value and secret operations
    session_storage_store: Option<Arc<dyn everruns_core::traits::SessionStorageStore>>,
    /// Session SQL database store for session-scoped databases
    sqldb_store: Option<Arc<dyn everruns_core::session_sqldb::SessionSqlDbStore>>,
    /// Agent runner for triggering turn workflows (platform management send_message)
    runner: Option<Arc<dyn everruns_worker::AgentRunner>>,
    /// Lazy connection token resolver (decrypts stored tokens / mints GitHub App tokens)
    connection_resolver: Option<Arc<dyn everruns_core::traits::UserConnectionResolver>>,
    /// Base URL for generating presigned image URLs (e.g., "http://127.0.0.1:9000")
    /// When set, image resolution returns presigned URLs instead of base64 data.
    api_base_url: Option<String>,
    /// Signing secret for presigned URLs (WORKER_GRPC_AUTH_TOKEN)
    presign_secret: Option<String>,
    /// Budget service for check_budget tool
    budget_service: Arc<crate::services::BudgetService>,
}

impl WorkerServiceImpl {
    pub fn new(
        event_service: EventService,
        db: Arc<StorageBackend>,
        encryption: Option<Arc<EncryptionService>>,
        runner: Option<Arc<dyn everruns_worker::AgentRunner>>,
        platform_definition: PlatformDefinition,
    ) -> Self {
        Self::with_virtual_registry(
            event_service,
            db,
            encryption,
            runner,
            platform_definition,
            None,
        )
    }

    pub fn with_virtual_registry(
        event_service: EventService,
        db: Arc<StorageBackend>,
        encryption: Option<Arc<EncryptionService>>,
        runner: Option<Arc<dyn everruns_worker::AgentRunner>>,
        platform_definition: PlatformDefinition,
        virtual_registry: Option<
            Arc<crate::services::virtual_mount_registry::VirtualMountRegistry>,
        >,
    ) -> Self {
        let capability_registry = platform_definition.capability_registry().clone();
        let session_service = {
            let svc = SessionService::with_registry(db.clone(), capability_registry.clone());
            if let Some(ref reg) = virtual_registry {
                svc.with_virtual_registry(reg.clone())
            } else {
                svc
            }
        };
        let session_file_service = if let Some(ref reg) = virtual_registry {
            SessionFileService::new(db.clone()).with_virtual_registry(reg.clone())
        } else {
            SessionFileService::new(db.clone())
        };
        let llm_resolver_service = LlmResolverService::new(db.clone(), encryption.clone());
        let mcp_server_service = McpServerService::new(db.clone(), encryption.clone());
        let capability_service = Arc::new(CapabilityService::with_registry(
            db.clone(),
            encryption.clone(),
            capability_registry,
        ));

        // Create durable store using the pool if available (PostgreSQL mode only)
        // In dev mode (in-memory), durable execution is handled differently
        let durable_store = db
            .pool()
            .map(|pool| Arc::new(PostgresWorkflowEventStore::new(pool.clone())));

        // Create session storage store (PostgreSQL mode only)
        let session_storage_store: Option<Arc<dyn everruns_core::traits::SessionStorageStore>> =
            db.pool().map(|pool| {
                let database = crate::storage::Database::new(pool.clone());
                if let Some(enc) = &encryption {
                    Arc::new(crate::storage::create_db_session_storage_store(
                        database,
                        enc.as_ref().clone(),
                    )) as Arc<dyn everruns_core::traits::SessionStorageStore>
                } else {
                    Arc::new(
                        crate::storage::create_db_session_storage_store_without_encryption(
                            database,
                        ),
                    ) as Arc<dyn everruns_core::traits::SessionStorageStore>
                }
            });

        // Create connection resolver (requires encryption for token decryption)
        let connection_resolver: Option<Arc<dyn everruns_core::traits::UserConnectionResolver>> =
            encryption.as_ref().map(|enc| {
                // GitHub App token minting requires env vars (set in production via Doppler)
                let github_app_minter = Self::github_app_minter_from_env();
                Arc::new(crate::storage::DbConnectionResolver::new(
                    db.as_ref().clone(),
                    enc.as_ref().clone(),
                    github_app_minter,
                )) as Arc<dyn everruns_core::traits::UserConnectionResolver>
            });

        // Create session SQL database store (always available, in-memory backend)
        let sqldb_backend = Arc::new(everruns_session_sqldb::InMemorySqlDbBackend::new());
        let sqldb_store: Option<Arc<dyn everruns_core::session_sqldb::SessionSqlDbStore>> =
            Some(Arc::new(everruns_session_sqldb::InMemorySqlDbStore::new(
                sqldb_backend,
            )));

        // Presigned URL support: when API_BASE_URL is set, image resolution
        // returns presigned HTTP URLs instead of base64 data over gRPC.
        let api_base_url = std::env::var("API_BASE_URL").ok().filter(|s| !s.is_empty());
        let presign_secret = std::env::var("WORKER_GRPC_AUTH_TOKEN")
            .ok()
            .filter(|s| !s.is_empty());

        let budget_service = Arc::new(crate::services::BudgetService::new(db.clone()));

        Self {
            event_service,
            session_service,
            session_file_service,
            llm_resolver_service,
            mcp_server_service,
            capability_service,
            durable_store,
            task_broadcaster: None, // Set via set_task_broadcaster() after async initialization
            db,
            session_storage_store,
            sqldb_store,
            runner,
            connection_resolver,
            api_base_url,
            presign_secret,
            budget_service,
        }
    }

    /// Set the task notification broadcaster (must be called after async initialization)
    pub fn set_task_broadcaster(&mut self, broadcaster: Arc<TaskBroadcaster>) {
        self.task_broadcaster = Some(broadcaster);
    }

    /// Generate a presigned URL for an image, or None if presigned URLs are not configured.
    fn presigned_image_url(&self, image_id: uuid::Uuid, org_id: i64) -> Option<String> {
        match (&self.api_base_url, &self.presign_secret) {
            (Some(base_url), Some(secret)) => {
                Some(crate::api::internal_images::generate_presigned_url(
                    base_url, image_id, org_id, secret,
                ))
            }
            _ => None,
        }
    }

    /// Build a domain command context for the given org.
    fn domain_ctx(&self, org_id: i64) -> crate::domains::common::Ctx {
        crate::domains::common::Ctx::new(
            everruns_core::Caller::internal(org_id),
            self.db.clone(),
            self.capability_service.clone(),
            None,
        )
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
    ) -> Result<&Arc<dyn everruns_core::traits::SessionStorageStore>, Status> {
        self.session_storage_store
            .as_ref()
            .ok_or_else(|| Status::unavailable("Session storage not available"))
    }

    /// Get connection resolver or return unavailable error
    #[allow(clippy::result_large_err)]
    fn connection_resolver(
        &self,
    ) -> Result<&Arc<dyn everruns_core::traits::UserConnectionResolver>, Status> {
        self.connection_resolver
            .as_ref()
            .ok_or_else(|| Status::unavailable("Connection resolver not available (no encryption)"))
    }

    /// Create the session resource registry used by tools over gRPC.
    fn session_resource_registry(&self) -> Arc<dyn everruns_core::traits::SessionResourceRegistry> {
        Arc::new(crate::storage::DbSessionResourceRegistry::new(
            self.db.clone(),
        ))
    }

    /// Create the leased-resource store used by tools over gRPC.
    fn leased_resource_store(&self) -> Arc<dyn everruns_core::traits::LeasedResourceStore> {
        let registry = self.session_resource_registry();
        Arc::new(
            crate::storage::DbLeasedResourceStore::new(self.db.clone()).with_registry(registry),
        )
    }

    /// Create schedule store scoped to the given org_id, or return unavailable if no pool.
    #[allow(clippy::result_large_err)]
    fn schedule_store(
        &self,
        org_id: i64,
    ) -> Result<Arc<dyn everruns_core::traits::SessionScheduleStore>, Status> {
        if self.db.pool().is_some() {
            Ok(Arc::new(crate::storage::DbSessionScheduleStore::new(
                self.db.clone(),
                org_id,
            )))
        } else {
            Err(Status::unavailable(
                "Schedule store not available (requires PostgreSQL)",
            ))
        }
    }

    /// Build a GitHubAppTokenMinter from environment variables (if configured).
    fn github_app_minter_from_env() -> Option<crate::storage::GitHubAppTokenMinter> {
        let app_id = std::env::var("GITHUB_APP_ID")
            .ok()
            .filter(|s| !s.is_empty())?;
        let private_key = std::env::var("GITHUB_APP_PRIVATE_KEY")
            .ok()
            .filter(|s| !s.is_empty())?;
        Some(crate::storage::GitHubAppTokenMinter::new(
            app_id,
            private_key,
        ))
    }

    /// Get session SQL database store or return unavailable error
    #[allow(clippy::result_large_err)]
    fn sqldb_store(
        &self,
    ) -> Result<&Arc<dyn everruns_core::session_sqldb::SessionSqlDbStore>, Status> {
        self.sqldb_store
            .as_ref()
            .ok_or_else(|| Status::unavailable("Session SQL database not available"))
    }

    /// Max gRPC message size (16MB)
    ///
    /// Image data no longer flows through gRPC — workers fetch images via presigned
    /// HTTP URLs returned by resolve_image/resolve_images RPCs. The limit only needs
    /// to accommodate metadata, proto messages, and session file content.
    const MAX_GRPC_MESSAGE_SIZE: usize = 16 * 1024 * 1024;

    /// Create a tonic server for this service
    pub fn into_server(self) -> WorkerServiceServer<Self> {
        WorkerServiceServer::new(self)
            .max_decoding_message_size(Self::MAX_GRPC_MESSAGE_SIZE)
            .max_encoding_message_size(Self::MAX_GRPC_MESSAGE_SIZE)
    }

    /// Convert ResolvedModel to proto ModelWithProvider
    fn resolved_model_to_proto(
        resolved: crate::services::ResolvedModel,
    ) -> proto::ModelWithProvider {
        proto::ModelWithProvider {
            model: resolved.model_id,
            provider_type: resolved.provider_type,
            api_key: resolved.api_key,
            base_url: resolved.base_url,
        }
    }

    /// Build MCP tool definitions from agent's MCP capabilities.
    ///
    /// Extracts MCP server UUIDs from capability IDs (format: "mcp:{uuid}"),
    /// batch-fetches all servers with cached tools in a single query,
    /// and converts to proto McpToolDef.
    async fn build_mcp_tool_definitions(
        &self,
        org_id: i64,
        agent: &everruns_core::Agent,
    ) -> Vec<McpToolDef> {
        use everruns_core::capabilities::mcp::parse_mcp_capability_id;
        use everruns_core::mcp_server::mcp_tool_name;

        // Collect unique MCP server IDs from capabilities
        let server_ids: Vec<uuid::Uuid> = agent
            .capabilities
            .iter()
            .filter_map(|cap| parse_mcp_capability_id(cap.capability_id()))
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();

        if server_ids.is_empty() {
            return vec![];
        }

        // Batch fetch all MCP servers with cached tools in one query
        let servers = match self
            .mcp_server_service
            .get_batch_with_tools(&everruns_core::Caller::internal(org_id), &server_ids)
            .await
        {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "Failed to batch-fetch MCP servers");
                return vec![];
            }
        };

        let mut mcp_tools = Vec::new();

        for cap in &agent.capabilities {
            let server_id = match parse_mcp_capability_id(cap.capability_id()) {
                Some(id) => id,
                None => continue,
            };

            let Some((server, tools)) = servers.get(&server_id) else {
                tracing::warn!(server_id = %server_id, "MCP server not found, skipping");
                continue;
            };

            for tool in tools {
                let prefixed_name = mcp_tool_name(&server.name, &tool.name);
                let description = tool
                    .description
                    .clone()
                    .unwrap_or_else(|| format!("Tool from MCP server: {}", server.name));
                let parameters =
                    everruns_internal_protocol::json_to_proto_struct(&tool.input_schema);

                mcp_tools.push(McpToolDef {
                    name: prefixed_name,
                    description,
                    parameters: Some(parameters),
                });
            }
        }

        mcp_tools
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

/// Convert a SessionSchedule to proto representation.
fn session_schedule_to_proto(
    s: &everruns_core::session_schedule::SessionSchedule,
) -> proto::SessionScheduleProto {
    use everruns_internal_protocol::datetime_to_proto_timestamp;

    let schedule_type = match s.schedule_type {
        everruns_core::session_schedule::ScheduleType::Recurring => "recurring".to_string(),
        everruns_core::session_schedule::ScheduleType::OneShot => "oneshot".to_string(),
    };

    proto::SessionScheduleProto {
        id: Some(proto::Uuid {
            value: s.id.uuid().to_string(),
        }),
        session_id: Some(proto::Uuid {
            value: s.session_id.uuid().to_string(),
        }),
        description: s.description.clone(),
        cron_expression: s.cron_expression.clone(),
        scheduled_at: s.scheduled_at.map(datetime_to_proto_timestamp),
        timezone: s.timezone.clone(),
        schedule_type,
        enabled: s.enabled,
        next_trigger_at: s.next_trigger_at.map(datetime_to_proto_timestamp),
        last_triggered_at: s.last_triggered_at.map(datetime_to_proto_timestamp),
        trigger_count: s.trigger_count as i32,
        created_at: Some(datetime_to_proto_timestamp(s.created_at)),
        updated_at: Some(datetime_to_proto_timestamp(s.updated_at)),
    }
}

/// Convert a session resource entry to proto representation.
fn session_resource_entry_to_proto(
    e: &everruns_core::SessionResourceEntry,
) -> proto::SessionResourceEntryProto {
    use everruns_internal_protocol::datetime_to_proto_timestamp;

    proto::SessionResourceEntryProto {
        resource_id: e.resource_id.clone(),
        session_id: Some(proto::Uuid {
            value: e.session_id.uuid().to_string(),
        }),
        kind: e.kind.clone(),
        display_name: e.display_name.clone(),
        status: e.status.to_string(),
        metadata: Some(everruns_internal_protocol::json_to_proto_struct(
            &e.metadata,
        )),
        created_at: Some(datetime_to_proto_timestamp(e.created_at)),
        updated_at: Some(datetime_to_proto_timestamp(e.updated_at)),
    }
}

/// Convert a leased resource to proto representation.
fn leased_resource_to_proto(s: &everruns_core::LeasedResource) -> proto::LeasedResourceProto {
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
    }
}

/// Convert a SessionSqlDbError to a gRPC Status with appropriate error codes.
fn sqldb_error_to_status(e: everruns_core::session_sqldb::SessionSqlDbError) -> Status {
    use everruns_core::session_sqldb::SessionSqlDbError;
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

/// Convert a DatabaseInfo to its proto representation.
fn db_info_to_proto(
    db: everruns_core::session_sqldb::DatabaseInfo,
) -> proto::SessionSqlDbDatabaseInfo {
    use everruns_internal_protocol::datetime_to_proto_timestamp;
    proto::SessionSqlDbDatabaseInfo {
        name: db.name,
        size_bytes: db.size_bytes,
        page_count: db.page_count,
        created_at: Some(datetime_to_proto_timestamp(db.created_at)),
        updated_at: Some(datetime_to_proto_timestamp(db.updated_at)),
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

/// Convert a domain Message to a proto Message.
fn message_to_proto(message: &everruns_core::Message) -> proto::Message {
    use everruns_internal_protocol::{datetime_to_proto_timestamp, uuid_to_proto_uuid};

    let content_json_val = serde_json::to_value(&message.content).unwrap_or_default();
    let content = Some(everruns_internal_protocol::json_to_proto_list(
        &content_json_val,
    ));

    let controls = message.controls.as_ref().map(|c| {
        let json = serde_json::to_value(c).unwrap_or_default();
        everruns_internal_protocol::json_to_proto_struct(&json)
    });

    let metadata = message.metadata.as_ref().map(|m| {
        let json = serde_json::to_value(m).unwrap_or_default();
        everruns_internal_protocol::json_to_proto_struct(&json)
    });

    let external_actor = message.external_actor.as_ref().map(|ea| {
        let json = serde_json::to_value(ea).unwrap_or_default();
        everruns_internal_protocol::json_to_proto_struct(&json)
    });

    proto::Message {
        id: Some(uuid_to_proto_uuid(message.id.uuid())),
        role: message.role.to_string(),
        content,
        controls,
        metadata,
        created_at: Some(datetime_to_proto_timestamp(message.created_at)),
        thinking: message.thinking.clone(),
        thinking_signature: message.thinking_signature.clone(),
        external_actor,
    }
}

/// Extract a Message from an Event's data field
///
/// Events returned from EventService already have data parsed into EventData.
fn event_to_message(event: &everruns_core::Event) -> Option<everruns_core::Message> {
    use everruns_core::{ContentPart, EventData, Message};

    match &event.data {
        EventData::InputMessage(d) => Some(d.message.clone()),
        EventData::OutputMessageCompleted(d) => Some(d.message.clone()),
        EventData::ToolCompleted(d) => {
            let result: Option<serde_json::Value> =
                d.result.as_ref().map(|parts: &Vec<ContentPart>| {
                    if parts.len() == 1
                        && let ContentPart::Text(t) = &parts[0]
                    {
                        return serde_json::Value::String(t.text.clone());
                    }
                    serde_json::to_value(parts).unwrap_or_default()
                });
            Some(Message::tool_result(
                &d.tool_call_id,
                result,
                d.error.clone(),
            ))
        }
        _ => None,
    }
}

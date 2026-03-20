// Internal gRPC Service for Worker Communication
//
// Decision: Workers communicate with control plane via gRPC for all database operations
// Decision: This provides a clean boundary and simplifies worker deployment
// Decision: gRPC service uses the same services layer as HTTP API for consistency
// Decision: No direct database access - all operations go through services layer

use crate::services::{
    AgentService, CapabilityService, EventService, HarnessService, LlmResolverService,
    McpServerService, SessionFileService, SessionService,
    session_file::{CreateDirectoryInput, CreateFileInput, GrepInput, UpdateFileInput},
};
use crate::storage::{EncryptionService, StorageBackend};
use crate::task_notifications::TaskNotificationBroadcaster;
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
    // Session schedule operations
    CancelSessionScheduleRequest,
    CancelSessionScheduleResponse,
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
    ReleaseLeasedResourceRequest,
    ReleaseLeasedResourceResponse,
    ResolveImageRequest,
    ResolveImageResponse,
    ResolveImagesRequest,
    ResolveImagesResponse,
    ResolvedImageData,
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
/// This service follows the layered architecture: gRPC -> Services -> Storage
/// No direct database access is allowed - all operations go through the services layer.
pub struct WorkerServiceImpl {
    event_service: EventService,
    agent_service: AgentService,
    harness_service: HarnessService,
    session_service: SessionService,
    session_file_service: SessionFileService,
    llm_resolver_service: LlmResolverService,
    mcp_server_service: McpServerService,
    capability_service: CapabilityService,
    durable_store: Option<Arc<PostgresWorkflowEventStore>>,
    /// Task notification broadcaster for push-based notifications
    task_broadcaster: Option<Arc<TaskNotificationBroadcaster>>,
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
}

impl WorkerServiceImpl {
    pub fn new(
        event_service: EventService,
        db: Arc<StorageBackend>,
        encryption: Option<Arc<EncryptionService>>,
        runner: Option<Arc<dyn everruns_worker::AgentRunner>>,
        platform_definition: PlatformDefinition,
    ) -> Self {
        let agent_service = AgentService::new(db.clone());
        let harness_service = HarnessService::new(db.clone());
        let capability_registry = platform_definition.capability_registry().clone();
        let session_service =
            SessionService::with_registry(db.clone(), capability_registry.clone());
        let session_file_service = SessionFileService::new(db.clone());
        let llm_resolver_service = LlmResolverService::new(db.clone(), encryption.clone());
        let mcp_server_service = McpServerService::new(db.clone(), encryption.clone());
        let capability_service =
            CapabilityService::with_registry(db.clone(), encryption.clone(), capability_registry);

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

        Self {
            event_service,
            agent_service,
            harness_service,
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
        }
    }

    /// Set the task notification broadcaster (must be called after async initialization)
    pub fn set_task_broadcaster(&mut self, broadcaster: Arc<TaskNotificationBroadcaster>) {
        self.task_broadcaster = Some(broadcaster);
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

    /// Create the leased-resource store used by tools over gRPC.
    fn leased_resource_store(&self) -> Arc<dyn everruns_core::traits::LeasedResourceStore> {
        Arc::new(crate::storage::DbLeasedResourceStore::new(self.db.clone()))
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

    /// Get organization public_id from org_id
    #[allow(dead_code)]
    async fn get_org_public_id(&self, org_id: i64) -> Result<String, Status> {
        self.db
            .get_organization(org_id)
            .await
            .map_err(|e| {
                tracing::error!("Failed to get organization: {}", e);
                Status::internal("Failed to get organization")
            })?
            .map(|org| org.public_id)
            .ok_or_else(|| Status::not_found("Organization not found"))
    }

    /// Max gRPC message size (150MB for base64-encoded images + overhead)
    ///
    /// TODO: Sending large images over gRPC is inefficient. Future improvements:
    /// - Use presigned URLs for workers to fetch images directly from storage
    /// - Stream images in chunks instead of single large messages
    /// - Move to S3/blob storage with direct worker access
    const MAX_GRPC_MESSAGE_SIZE: usize = 150 * 1024 * 1024;

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

#[tonic::async_trait]
impl WorkerService for WorkerServiceImpl {
    // ========================================================================
    // Batched operations
    // ========================================================================

    async fn get_turn_context(
        &self,
        request: Request<GetTurnContextRequest>,
    ) -> Result<Response<GetTurnContextResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let internal_caller = everruns_core::Caller::internal(req.org_id);

        // Get session via SessionService
        let session = self
            .session_service
            .get(&internal_caller, session_id, None)
            .await
            .map_err(|e| {
                tracing::error!("Failed to get session: {}", e);
                Status::internal("Failed to get session")
            })?
            .ok_or_else(|| Status::not_found("Session not found"))?;

        // Get agent with capabilities via AgentService (optional)
        let agent = if let Some(agent_id) = session.agent_id {
            self.agent_service
                .get(&internal_caller, agent_id.uuid())
                .await
                .map_err(|e| {
                    tracing::error!("Failed to get agent: {}", e);
                    Status::internal("Failed to get agent")
                })?
        } else {
            None
        };

        // Load harness
        let harness = self
            .harness_service
            .get(&internal_caller, session.harness_id.uuid())
            .await
            .map_err(|e| {
                tracing::error!("Failed to get harness: {}", e);
                Status::internal("Failed to get harness")
            })?;

        // Convert to proto types
        use everruns_internal_protocol::{datetime_to_proto_timestamp, uuid_to_proto_uuid};

        let proto_agent = agent.as_ref().map(schema_agent_to_proto);
        let proto_harness = harness.as_ref().map(schema_harness_to_proto);

        let proto_session = proto::Session {
            id: Some(uuid_to_proto_uuid(session.id.uuid())),
            agent_id: session.agent_id.map(|id| uuid_to_proto_uuid(id.uuid())),
            harness_id: Some(uuid_to_proto_uuid(session.harness_id.uuid())),
            title: session.title.clone().unwrap_or_default(),
            locale: session.locale.clone().unwrap_or_default(),
            status: session.status.to_string(),
            created_at: Some(datetime_to_proto_timestamp(session.created_at)),
            updated_at: Some(datetime_to_proto_timestamp(session.updated_at)),
            default_model_id: session.model_id.map(|id| uuid_to_proto_uuid(id.uuid())),
            organization_id: session.organization_id.clone(),
            capabilities: session
                .capabilities
                .iter()
                .filter_map(|c| serde_json::to_string(c).ok())
                .collect(),
            tags: session.tags.clone(),
        };

        // Load messages from events using EventService with limit
        let default_limit: i32 = std::env::var("TURN_CONTEXT_MESSAGE_LIMIT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(200);
        let message_limit = req
            .message_limit
            .filter(|&l| l > 0)
            .unwrap_or(default_limit);

        // Fetch one extra to detect truncation
        let events = self
            .event_service
            .list_message_events_limited(session_id, Some(message_limit + 1))
            .await
            .map_err(|e| {
                tracing::error!("Failed to list messages: {}", e);
                Status::internal("Failed to list messages")
            })?;

        let messages_truncated = events.len() > message_limit as usize;
        let events_iter: Box<dyn Iterator<Item = _>> = if messages_truncated {
            // Skip the extra oldest message we fetched for truncation detection
            Box::new(events.into_iter().skip(1))
        } else {
            Box::new(events.into_iter())
        };

        let mut proto_messages: Vec<proto::Message> = Vec::new();

        for event in events_iter {
            // Extract message from typed event data
            let message = match event_to_message(&event) {
                Some(m) => m,
                None => {
                    tracing::warn!(
                        "Failed to extract message from event {}: type={}",
                        event.id,
                        event.event_type
                    );
                    continue;
                }
            };

            // Convert to proto Message using prost types
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

            proto_messages.push(proto::Message {
                id: Some(uuid_to_proto_uuid(message.id.uuid())),
                role: message.role.to_string(),
                content,
                controls,
                metadata,
                created_at: Some(datetime_to_proto_timestamp(message.created_at)),
                thinking: message.thinking.clone(),
                thinking_signature: message.thinking_signature.clone(),
                external_actor,
            });
        }

        // Get model with provider (decrypted API key) via LlmResolverService
        // Priority: session model > agent model > harness model > default model
        let model_id = session
            .model_id
            .or(agent.as_ref().and_then(|a| a.default_model_id))
            .or(harness.as_ref().and_then(|h| h.default_model_id));

        let model: Option<proto::ModelWithProvider> = if let Some(mid) = model_id {
            self.llm_resolver_service
                .resolve_model(req.org_id, mid.uuid())
                .await
                .map_err(|e| {
                    tracing::error!("Failed to resolve model: {}", e);
                    Status::internal("Failed to resolve model")
                })?
                .map(Self::resolved_model_to_proto)
        } else {
            // Try to get the default model
            self.llm_resolver_service
                .resolve_default_model(req.org_id)
                .await
                .map_err(|e| {
                    tracing::error!("Failed to resolve default model: {}", e);
                    Status::internal("Failed to resolve default model")
                })?
                .map(Self::resolved_model_to_proto)
        };

        // Build MCP tool definitions from agent's MCP capabilities
        // This resolves MCP tools so the worker doesn't need to look them up
        let mcp_tool_definitions = if let Some(ref a) = agent {
            self.build_mcp_tool_definitions(req.org_id, a).await
        } else {
            vec![]
        };

        Ok(Response::new(GetTurnContextResponse {
            agent: proto_agent,
            session: Some(proto_session),
            messages: proto_messages,
            model,
            mcp_tool_definitions,
            harness: proto_harness,
            messages_truncated,
        }))
    }

    async fn emit_event_stream(
        &self,
        request: Request<Streaming<EmitEventRequest>>,
    ) -> Result<Response<EmitEventStreamResponse>, Status> {
        let mut stream = request.into_inner();
        let mut event_requests: Vec<everruns_core::EventRequest> = Vec::new();

        // Collect all event requests from the stream, converting proto to core types
        while let Some(req) = stream.message().await? {
            let proto_event_request = match req.event {
                Some(e) => e,
                None => {
                    tracing::warn!("Received emit_event_stream request without event");
                    continue;
                }
            };

            // Convert proto EventRequest to core EventRequest using typed conversions
            let core_event_request = match proto_event_request_to_schema(proto_event_request) {
                Ok(e) => e,
                Err(e) => {
                    tracing::warn!("Failed to convert proto event request to core: {}", e);
                    continue;
                }
            };

            event_requests.push(core_event_request);
        }

        // Emit all events through the EventService
        let events_processed = self
            .event_service
            .emit_batch(event_requests)
            .await
            .map_err(|e| {
                tracing::error!("Failed to emit event batch: {}", e);
                Status::internal("Failed to store events")
            })?;

        Ok(Response::new(EmitEventStreamResponse { events_processed }))
    }

    // ========================================================================
    // Individual operations
    // ========================================================================

    async fn get_agent(
        &self,
        request: Request<GetAgentRequest>,
    ) -> Result<Response<GetAgentResponse>, Status> {
        let req = request.into_inner();
        let agent_id = parse_uuid(req.agent_id.as_ref())?;

        // Get agent with capabilities via AgentService
        let internal_caller = everruns_core::Caller::internal(req.org_id);
        let agent = self
            .agent_service
            .get(&internal_caller, agent_id)
            .await
            .map_err(|e| Status::internal(format!("Failed to get agent: {}", e)))?;

        let proto_agent = agent.map(|a| schema_agent_to_proto(&a));

        Ok(Response::new(GetAgentResponse { agent: proto_agent }))
    }

    async fn get_harness(
        &self,
        request: Request<GetHarnessRequest>,
    ) -> Result<Response<GetHarnessResponse>, Status> {
        let req = request.into_inner();
        let harness_id = parse_uuid(req.harness_id.as_ref())?;

        let harness = self
            .harness_service
            .resolve_effective(req.org_id, everruns_core::HarnessId::from_uuid(harness_id))
            .await
            .map_err(|e| Status::internal(format!("Failed to get harness: {}", e)))?;

        let proto_harness = harness.map(|h| schema_harness_to_proto(&h));

        Ok(Response::new(GetHarnessResponse {
            harness: proto_harness,
        }))
    }

    async fn get_session(
        &self,
        request: Request<GetSessionRequest>,
    ) -> Result<Response<GetSessionResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let internal_caller = everruns_core::Caller::internal(req.org_id);

        // Get session via SessionService
        let session = self
            .session_service
            .get(&internal_caller, session_id, None)
            .await
            .map_err(|e| {
                tracing::error!("Failed to get session: {}", e);
                Status::internal("Failed to get session")
            })?;

        use everruns_internal_protocol::{datetime_to_proto_timestamp, uuid_to_proto_uuid};

        let proto_session = session.map(|s| proto::Session {
            id: Some(uuid_to_proto_uuid(s.id.uuid())),
            agent_id: s.agent_id.map(|id| uuid_to_proto_uuid(id.uuid())),
            harness_id: Some(uuid_to_proto_uuid(s.harness_id.uuid())),
            title: s.title.clone().unwrap_or_default(),
            locale: s.locale.clone().unwrap_or_default(),
            status: s.status.to_string(),
            created_at: Some(datetime_to_proto_timestamp(s.created_at)),
            updated_at: Some(datetime_to_proto_timestamp(s.updated_at)),
            default_model_id: s.model_id.map(|id| uuid_to_proto_uuid(id.uuid())),
            organization_id: s.organization_id.clone(),
            capabilities: s
                .capabilities
                .iter()
                .filter_map(|c| serde_json::to_string(c).ok())
                .collect(),
            tags: s.tags.clone(),
        });

        Ok(Response::new(GetSessionResponse {
            session: proto_session,
        }))
    }

    async fn set_session_status(
        &self,
        request: Request<SetSessionStatusRequest>,
    ) -> Result<Response<SetSessionStatusResponse>, Status> {
        use everruns_internal_protocol::{datetime_to_proto_timestamp, uuid_to_proto_uuid};

        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let internal_caller = everruns_core::Caller::internal(req.org_id);

        // Validate status value
        let valid_statuses = ["started", "active", "idle", "waiting_for_tool_results"];
        if !valid_statuses.contains(&req.status.as_str()) {
            return Err(Status::invalid_argument(format!(
                "Invalid status '{}'. Must be one of: started, active, idle, waiting_for_tool_results",
                req.status
            )));
        }

        let session = self
            .session_service
            .update_status(&internal_caller, session_id, req.status)
            .await
            .map_err(|e| {
                tracing::error!("Failed to update session status: {}", e);
                Status::internal("Failed to update session status")
            })?
            .ok_or_else(|| Status::not_found("Session not found"))?;

        let proto_session = proto::Session {
            id: Some(uuid_to_proto_uuid(session.id.uuid())),
            agent_id: session.agent_id.map(|id| uuid_to_proto_uuid(id.uuid())),
            harness_id: Some(uuid_to_proto_uuid(session.harness_id.uuid())),
            title: session.title.clone().unwrap_or_default(),
            locale: session.locale.clone().unwrap_or_default(),
            status: session.status.to_string(),
            created_at: Some(datetime_to_proto_timestamp(session.created_at)),
            updated_at: Some(datetime_to_proto_timestamp(session.updated_at)),
            default_model_id: session.model_id.map(|id| uuid_to_proto_uuid(id.uuid())),
            organization_id: session.organization_id.clone(),
            capabilities: session
                .capabilities
                .iter()
                .filter_map(|c| serde_json::to_string(c).ok())
                .collect(),
            tags: session.tags.clone(),
        };

        Ok(Response::new(SetSessionStatusResponse {
            session: Some(proto_session),
        }))
    }

    async fn set_session_title(
        &self,
        request: Request<SetSessionTitleRequest>,
    ) -> Result<Response<SetSessionTitleResponse>, Status> {
        use everruns_internal_protocol::{datetime_to_proto_timestamp, uuid_to_proto_uuid};

        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let internal_caller = everruns_core::Caller::internal(req.org_id);

        let session = self
            .session_service
            .update(
                &internal_caller,
                session_id,
                crate::api::sessions::UpdateSessionRequest {
                    title: Some(req.title),
                    locale: None,
                    tags: None,
                },
            )
            .await
            .map_err(|e| {
                tracing::error!("Failed to update session title: {}", e);
                Status::internal("Failed to update session title")
            })?
            .ok_or_else(|| Status::not_found("Session not found"))?;

        let proto_session = proto::Session {
            id: Some(uuid_to_proto_uuid(session.id.uuid())),
            agent_id: session.agent_id.map(|id| uuid_to_proto_uuid(id.uuid())),
            harness_id: Some(uuid_to_proto_uuid(session.harness_id.uuid())),
            title: session.title.clone().unwrap_or_default(),
            locale: session.locale.clone().unwrap_or_default(),
            status: session.status.to_string(),
            created_at: Some(datetime_to_proto_timestamp(session.created_at)),
            updated_at: Some(datetime_to_proto_timestamp(session.updated_at)),
            default_model_id: session.model_id.map(|id| uuid_to_proto_uuid(id.uuid())),
            organization_id: session.organization_id.clone(),
            capabilities: session
                .capabilities
                .iter()
                .filter_map(|c| serde_json::to_string(c).ok())
                .collect(),
            tags: session.tags.clone(),
        };

        Ok(Response::new(SetSessionTitleResponse {
            session: Some(proto_session),
        }))
    }

    async fn load_messages(
        &self,
        request: Request<LoadMessagesRequest>,
    ) -> Result<Response<LoadMessagesResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;

        // Query events for message-related event types using EventService
        let events = self
            .event_service
            .list_message_events(session_id)
            .await
            .map_err(|e| Status::internal(format!("Failed to list messages: {}", e)))?;

        use everruns_internal_protocol::{datetime_to_proto_timestamp, uuid_to_proto_uuid};

        let mut proto_messages: Vec<proto::Message> = Vec::with_capacity(events.len());

        for event in events {
            // Extract message from typed event data
            let message = match event_to_message(&event) {
                Some(m) => m,
                None => {
                    tracing::warn!(
                        "Failed to extract message from event {}: type={}",
                        event.id,
                        event.event_type
                    );
                    continue;
                }
            };

            // Convert to proto Message using prost types
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

            proto_messages.push(proto::Message {
                id: Some(uuid_to_proto_uuid(message.id.uuid())),
                role: message.role.to_string(),
                content,
                controls,
                metadata,
                created_at: Some(datetime_to_proto_timestamp(message.created_at)),
                thinking: message.thinking.clone(),
                thinking_signature: message.thinking_signature.clone(),
                external_actor,
            });
        }

        Ok(Response::new(LoadMessagesResponse {
            messages: proto_messages,
        }))
    }

    async fn add_message(
        &self,
        request: Request<AddMessageRequest>,
    ) -> Result<Response<AddMessageResponse>, Status> {
        use chrono::Utc;
        use everruns_core::{
            ContentPart, Controls, EventContext, EventRequest, Message, MessageRole,
            events::{InputMessageData, OutputMessageCompletedData},
        };
        use everruns_internal_protocol::{
            datetime_to_proto_timestamp, json_to_proto_list, json_to_proto_struct,
            proto_list_to_json, proto_struct_to_json, uuid_to_proto_uuid,
        };

        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;

        // Parse content from prost ListValue
        let content_json = req
            .content
            .as_ref()
            .map(proto_list_to_json)
            .unwrap_or_else(|| serde_json::Value::Array(vec![]));
        let content: Vec<ContentPart> = serde_json::from_value(content_json)
            .map_err(|e| Status::invalid_argument(format!("Invalid content: {}", e)))?;

        // Parse optional controls from prost Struct
        let controls: Option<Controls> = req
            .controls
            .as_ref()
            .map(|s| serde_json::from_value(proto_struct_to_json(s)))
            .transpose()
            .map_err(|e| Status::invalid_argument(format!("Invalid controls: {}", e)))?;

        // Parse optional metadata from prost Struct
        let metadata: Option<std::collections::HashMap<String, serde_json::Value>> = req
            .metadata
            .as_ref()
            .map(|s| serde_json::from_value(proto_struct_to_json(s)))
            .transpose()
            .map_err(|e| Status::invalid_argument(format!("Invalid metadata: {}", e)))?;

        // Parse role
        let role = MessageRole::from(req.role.as_str());

        // Create the message
        let message = Message {
            id: uuid::Uuid::now_v7().into(),
            role: role.clone(),
            content,
            phase: None,
            thinking: None,
            thinking_signature: None,
            controls,
            metadata,
            external_actor: None,
            created_at: Utc::now(),
        };

        // Create typed event request based on role
        let event_request = match role {
            MessageRole::User => EventRequest::new(
                session_id.into(),
                EventContext::empty(),
                InputMessageData::new(message.clone()),
            ),
            MessageRole::Agent => EventRequest::new(
                session_id.into(),
                EventContext::empty(),
                OutputMessageCompletedData::new(message.clone()),
            ),
            MessageRole::System | MessageRole::ToolResult => {
                // System and tool messages are typically stored via emit_event
                return Err(Status::invalid_argument(
                    "System and tool messages should be added via emit_event",
                ));
            }
        };

        // Emit through the EventService
        let _stored_event = self.event_service.emit(event_request).await.map_err(|e| {
            tracing::error!("Failed to create message event: {}", e);
            Status::internal("Failed to store message")
        })?;

        // Convert message to proto using prost types
        let content_json_val = serde_json::to_value(&message.content).unwrap_or_default();
        let content = Some(json_to_proto_list(&content_json_val));

        let controls = message.controls.as_ref().map(|c| {
            let json = serde_json::to_value(c).unwrap_or_default();
            json_to_proto_struct(&json)
        });

        let metadata = message.metadata.as_ref().map(|m| {
            let json = serde_json::to_value(m).unwrap_or_default();
            json_to_proto_struct(&json)
        });

        let external_actor = message.external_actor.as_ref().map(|ea| {
            let json = serde_json::to_value(ea).unwrap_or_default();
            json_to_proto_struct(&json)
        });

        let proto_message = proto::Message {
            id: Some(uuid_to_proto_uuid(message.id.uuid())),
            role: message.role.to_string(),
            content,
            controls,
            metadata,
            created_at: Some(datetime_to_proto_timestamp(message.created_at)),
            thinking: message.thinking.clone(),
            thinking_signature: message.thinking_signature.clone(),
            external_actor,
        };

        Ok(Response::new(AddMessageResponse {
            message: Some(proto_message),
        }))
    }

    async fn emit_event(
        &self,
        request: Request<EmitEventRequest>,
    ) -> Result<Response<EmitEventResponse>, Status> {
        let req = request.into_inner();
        let proto_event_request = req
            .event
            .ok_or_else(|| Status::invalid_argument("Missing event"))?;

        // Convert proto EventRequest to core EventRequest using typed conversions
        let core_event_request = proto_event_request_to_schema(proto_event_request)
            .map_err(|e| Status::invalid_argument(format!("Invalid event: {}", e)))?;

        // Emit through the EventService
        let stored_event = self
            .event_service
            .emit(core_event_request)
            .await
            .map_err(|e| {
                tracing::error!("Failed to emit event: {}", e);
                Status::internal("Failed to store event")
            })?;

        // Return the full stored event with id and sequence
        Ok(Response::new(EmitEventResponse {
            event: Some(schema_event_to_proto(&stored_event)),
        }))
    }

    async fn commit_exec(
        &self,
        _request: Request<CommitExecRequest>,
    ) -> Result<Response<CommitExecResponse>, Status> {
        // No-op for now - exec_id tracking for idempotency can be added later
        Ok(Response::new(CommitExecResponse { committed: true }))
    }

    async fn get_model_with_provider(
        &self,
        request: Request<GetModelWithProviderRequest>,
    ) -> Result<Response<GetModelWithProviderResponse>, Status> {
        let req = request.into_inner();
        let model_id = parse_uuid(req.model_id.as_ref())?;

        // Check if encryption service is available
        if !self.llm_resolver_service.has_encryption() {
            return Err(Status::unavailable(
                "Encryption service not configured - cannot decrypt API keys",
            ));
        }

        // Resolve model via LlmResolverService
        let resolved = self
            .llm_resolver_service
            .resolve_model(req.org_id, model_id)
            .await
            .map_err(|e| {
                tracing::error!("Failed to resolve model: {}", e);
                Status::internal("Failed to resolve model")
            })?;

        Ok(Response::new(GetModelWithProviderResponse {
            model: resolved.map(Self::resolved_model_to_proto),
        }))
    }

    async fn get_default_model(
        &self,
        request: Request<GetDefaultModelRequest>,
    ) -> Result<Response<GetDefaultModelResponse>, Status> {
        let req = request.into_inner();
        // Check if encryption service is available
        if !self.llm_resolver_service.has_encryption() {
            tracing::error!("gRPC get_default_model: encryption service not available");
            return Err(Status::unavailable(
                "Encryption service not configured - cannot decrypt API keys",
            ));
        }

        // Resolve default model via LlmResolverService
        let resolved = self
            .llm_resolver_service
            .resolve_default_model(req.org_id)
            .await
            .map_err(|e| {
                tracing::error!("Failed to resolve default model: {}", e);
                Status::internal("Failed to resolve default model")
            })?;

        // Log model resolution result (omit api_key length for security)
        if let Some(ref model) = resolved {
            tracing::debug!(
                model_id = %model.model_id,
                provider_type = %model.provider_type,
                has_api_key = model.api_key.is_some(),
                "gRPC get_default_model: resolved model"
            );
        } else {
            tracing::debug!("gRPC get_default_model: no default model configured");
        }

        Ok(Response::new(GetDefaultModelResponse {
            model: resolved.map(Self::resolved_model_to_proto),
        }))
    }

    // ========================================================================
    // Session file operations (via SessionFileService)
    // ========================================================================

    async fn session_read_file(
        &self,
        request: Request<SessionReadFileRequest>,
    ) -> Result<Response<SessionReadFileResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;

        // Read file via SessionFileService
        let file = self
            .session_file_service
            .read_file(session_id, &req.path)
            .await
            .map_err(|e| {
                tracing::error!("Failed to read file: {}", e);
                Status::internal("Failed to read file")
            })?;

        use everruns_internal_protocol::{datetime_to_proto_timestamp, uuid_to_proto_uuid};

        let proto_file = file.map(|f| proto::SessionFile {
            id: Some(uuid_to_proto_uuid(f.id)),
            session_id: Some(uuid_to_proto_uuid(f.session_id)),
            path: f.path.clone(),
            name: f.name.clone(),
            content: f.content,
            encoding: f.encoding,
            is_directory: f.is_directory,
            is_readonly: f.is_readonly,
            size_bytes: f.size_bytes,
            created_at: Some(datetime_to_proto_timestamp(f.created_at)),
            updated_at: Some(datetime_to_proto_timestamp(f.updated_at)),
        });

        Ok(Response::new(SessionReadFileResponse { file: proto_file }))
    }

    async fn session_write_file(
        &self,
        request: Request<SessionWriteFileRequest>,
    ) -> Result<Response<SessionWriteFileResponse>, Status> {
        use everruns_internal_protocol::{datetime_to_proto_timestamp, uuid_to_proto_uuid};

        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;

        // Check if file already exists
        let existing = self
            .session_file_service
            .read_file(session_id, &req.path)
            .await
            .map_err(|e| {
                tracing::error!("Failed to check file: {}", e);
                Status::internal("Failed to check file")
            })?;

        let file = if existing.is_some() {
            // Update existing file
            let update = UpdateFileInput {
                content: Some(req.content.clone()),
                encoding: None,
                is_readonly: None,
            };
            self.session_file_service
                .update_file(session_id, &req.path, update)
                .await
                .map_err(|e| {
                    tracing::error!("Failed to update file: {}", e);
                    Status::internal("Failed to update file")
                })?
                .ok_or_else(|| Status::internal("File disappeared during update"))?
        } else {
            // Create new file
            let create = CreateFileInput {
                path: req.path.clone(),
                content: Some(req.content.clone()),
                encoding: None,
                is_readonly: None,
            };
            self.session_file_service
                .create_file(session_id, create)
                .await
                .map_err(|e| {
                    let msg = e.to_string();
                    if msg.contains("already exists") {
                        Status::already_exists(msg)
                    } else {
                        tracing::error!("Failed to create file: {}", e);
                        Status::internal("Failed to create file")
                    }
                })?
        };

        let proto_file = proto::SessionFile {
            id: Some(uuid_to_proto_uuid(file.id)),
            session_id: Some(uuid_to_proto_uuid(file.session_id)),
            path: file.path.clone(),
            name: file.name.clone(),
            content: file.content,
            encoding: file.encoding,
            is_directory: file.is_directory,
            is_readonly: file.is_readonly,
            size_bytes: file.size_bytes,
            created_at: Some(datetime_to_proto_timestamp(file.created_at)),
            updated_at: Some(datetime_to_proto_timestamp(file.updated_at)),
        };

        Ok(Response::new(SessionWriteFileResponse {
            file: Some(proto_file),
        }))
    }

    async fn session_write_file_if_content_matches(
        &self,
        request: Request<SessionWriteFileIfContentMatchesRequest>,
    ) -> Result<Response<SessionWriteFileIfContentMatchesResponse>, Status> {
        use everruns_internal_protocol::{datetime_to_proto_timestamp, uuid_to_proto_uuid};

        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;

        let file = self
            .session_file_service
            .update_file_if_content_matches(
                session_id,
                &req.path,
                &req.expected_content,
                &req.expected_encoding,
                &req.content,
                &req.encoding,
            )
            .await
            .map_err(|e| {
                tracing::error!("Failed to conditionally update file: {}", e);
                Status::internal("Failed to update file")
            })?;

        let proto_file = file.map(|file| proto::SessionFile {
            id: Some(uuid_to_proto_uuid(file.id)),
            session_id: Some(uuid_to_proto_uuid(file.session_id)),
            path: file.path.clone(),
            name: file.name.clone(),
            content: file.content,
            encoding: file.encoding,
            is_directory: file.is_directory,
            is_readonly: file.is_readonly,
            size_bytes: file.size_bytes,
            created_at: Some(datetime_to_proto_timestamp(file.created_at)),
            updated_at: Some(datetime_to_proto_timestamp(file.updated_at)),
        });

        Ok(Response::new(SessionWriteFileIfContentMatchesResponse {
            file: proto_file,
        }))
    }

    async fn session_delete_file(
        &self,
        request: Request<SessionDeleteFileRequest>,
    ) -> Result<Response<SessionDeleteFileResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;

        // Delete via SessionFileService
        let deleted = self
            .session_file_service
            .delete(session_id, &req.path, req.recursive)
            .await
            .map_err(|e| {
                tracing::error!("Failed to delete file: {}", e);
                Status::internal("Failed to delete file")
            })?;

        Ok(Response::new(SessionDeleteFileResponse { deleted }))
    }

    async fn session_list_directory(
        &self,
        request: Request<SessionListDirectoryRequest>,
    ) -> Result<Response<SessionListDirectoryResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;

        // List directory via SessionFileService
        let files = self
            .session_file_service
            .list_directory(session_id, &req.path)
            .await
            .map_err(|e| {
                let msg = e.to_string();
                if msg.contains("not found") || msg.contains("not a directory") {
                    tracing::debug!("Directory not found: {}", req.path);
                    Status::not_found(msg)
                } else {
                    tracing::error!("Failed to list directory: {}", e);
                    Status::internal("Failed to list directory")
                }
            })?;

        use everruns_internal_protocol::{datetime_to_proto_timestamp, uuid_to_proto_uuid};

        let proto_files: Vec<proto::FileInfo> = files
            .iter()
            .map(|f| proto::FileInfo {
                id: Some(uuid_to_proto_uuid(f.id)),
                session_id: Some(uuid_to_proto_uuid(f.session_id)),
                path: f.path.clone(),
                name: f.name.clone(),
                is_directory: f.is_directory,
                is_readonly: f.is_readonly,
                size_bytes: f.size_bytes,
                created_at: Some(datetime_to_proto_timestamp(f.created_at)),
                updated_at: Some(datetime_to_proto_timestamp(f.updated_at)),
            })
            .collect();

        Ok(Response::new(SessionListDirectoryResponse {
            files: proto_files,
        }))
    }

    async fn session_stat_file(
        &self,
        request: Request<SessionStatFileRequest>,
    ) -> Result<Response<SessionStatFileResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;

        // Get file stat via SessionFileService
        let stat = self
            .session_file_service
            .stat(session_id, &req.path)
            .await
            .map_err(|e| {
                tracing::error!("Failed to stat file: {}", e);
                Status::internal("Failed to stat file")
            })?;

        use everruns_internal_protocol::datetime_to_proto_timestamp;

        let proto_stat = stat.map(|s| proto::FileStat {
            path: s.path.clone(),
            name: s.name.clone(),
            is_directory: s.is_directory,
            is_readonly: s.is_readonly,
            size_bytes: s.size_bytes,
            created_at: Some(datetime_to_proto_timestamp(s.created_at)),
            updated_at: Some(datetime_to_proto_timestamp(s.updated_at)),
        });

        Ok(Response::new(SessionStatFileResponse { stat: proto_stat }))
    }

    async fn session_grep_files(
        &self,
        request: Request<SessionGrepFilesRequest>,
    ) -> Result<Response<SessionGrepFilesResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;

        // Grep via SessionFileService
        let grep_input = GrepInput {
            pattern: req.pattern.clone(),
            path_pattern: req.path_pattern.clone(),
        };

        let grep_results = self
            .session_file_service
            .grep(session_id, grep_input)
            .await
            .map_err(|e| {
                // Check if it's a regex error
                if e.to_string().contains("regex") {
                    return Status::invalid_argument(format!("Invalid regex pattern: {}", e));
                }
                tracing::error!("Failed to grep files: {}", e);
                Status::internal("Failed to grep files")
            })?;

        // Convert GrepResult to proto GrepMatch (flatten)
        let matches: Vec<proto::GrepMatch> = grep_results
            .into_iter()
            .flat_map(|result| {
                result.matches.into_iter().map(|m| proto::GrepMatch {
                    path: m.path,
                    line_number: m.line_number as u64,
                    line: m.line,
                })
            })
            .collect();

        Ok(Response::new(SessionGrepFilesResponse { matches }))
    }

    async fn session_create_directory(
        &self,
        request: Request<SessionCreateDirectoryRequest>,
    ) -> Result<Response<SessionCreateDirectoryResponse>, Status> {
        use everruns_internal_protocol::{datetime_to_proto_timestamp, uuid_to_proto_uuid};

        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;

        // Create directory via SessionFileService
        let create = CreateDirectoryInput {
            path: req.path.clone(),
        };

        let file_info = self
            .session_file_service
            .create_directory(session_id, create)
            .await
            .map_err(|e| {
                // Check if it's a "file exists" error
                if e.to_string().contains("file exists") || e.to_string().contains("A file exists")
                {
                    return Status::already_exists("A file with this path already exists");
                }
                tracing::error!("Failed to create directory: {}", e);
                Status::internal("Failed to create directory")
            })?;

        let proto_file_info = proto::FileInfo {
            id: Some(uuid_to_proto_uuid(file_info.id)),
            session_id: Some(uuid_to_proto_uuid(file_info.session_id)),
            path: file_info.path.clone(),
            name: file_info.name.clone(),
            is_directory: file_info.is_directory,
            is_readonly: file_info.is_readonly,
            size_bytes: file_info.size_bytes,
            created_at: Some(datetime_to_proto_timestamp(file_info.created_at)),
            updated_at: Some(datetime_to_proto_timestamp(file_info.updated_at)),
        };

        Ok(Response::new(SessionCreateDirectoryResponse {
            directory: Some(proto_file_info),
        }))
    }

    // ========================================================================
    // Durable execution operations
    // ========================================================================

    async fn create_durable_workflow(
        &self,
        request: Request<CreateDurableWorkflowRequest>,
    ) -> Result<Response<CreateDurableWorkflowResponse>, Status> {
        use everruns_internal_protocol::uuid_to_proto_uuid;

        let req = request.into_inner();
        let store = self.durable_store()?;

        // Generate or use provided workflow ID
        let workflow_id = if let Some(proto_id) = req.workflow_id {
            parse_uuid(Some(&proto_id))?
        } else {
            uuid::Uuid::now_v7()
        };

        // Convert proto Struct to serde_json::Value
        let input = req
            .input
            .map(|s| everruns_internal_protocol::proto_struct_to_json(&s))
            .unwrap_or_else(|| serde_json::json!({}));

        // Create workflow instance
        store
            .create_workflow(workflow_id, &req.workflow_type, input, None)
            .await
            .map_err(|e| {
                tracing::error!("Failed to create durable workflow: {}", e);
                Status::internal("Failed to create workflow")
            })?;

        Ok(Response::new(CreateDurableWorkflowResponse {
            workflow_id: Some(uuid_to_proto_uuid(workflow_id)),
        }))
    }

    async fn get_durable_workflow_status(
        &self,
        request: Request<GetDurableWorkflowStatusRequest>,
    ) -> Result<Response<GetDurableWorkflowStatusResponse>, Status> {
        let req = request.into_inner();
        let store = self.durable_store()?;
        let workflow_id = parse_uuid(req.workflow_id.as_ref())?;

        let info = store.get_workflow_info(workflow_id).await.map_err(|e| {
            if matches!(e, StoreError::WorkflowNotFound(_)) {
                return Status::not_found("Workflow not found");
            }
            tracing::error!("Failed to get workflow status: {}", e);
            Status::internal("Failed to get workflow status")
        })?;

        let status = workflow_status_to_proto(info.status);
        let output = info
            .result
            .map(|o| everruns_internal_protocol::json_to_proto_struct(&o));
        let error = info.error.map(|e| e.message);

        Ok(Response::new(GetDurableWorkflowStatusResponse {
            status: status.into(),
            output,
            error,
        }))
    }

    async fn update_durable_workflow_status(
        &self,
        request: Request<UpdateDurableWorkflowStatusRequest>,
    ) -> Result<Response<UpdateDurableWorkflowStatusResponse>, Status> {
        let req = request.into_inner();
        let store = self.durable_store()?;
        let workflow_id = parse_uuid(req.workflow_id.as_ref())?;

        let status = proto_to_workflow_status(req.status());
        let output = req
            .output
            .map(|s| everruns_internal_protocol::proto_struct_to_json(&s));
        let error = req.error.clone().map(WorkflowError::new);

        store
            .update_workflow_status(workflow_id, status, output.clone(), error)
            .await
            .map_err(|e| {
                tracing::error!("Failed to update workflow status: {}", e);
                Status::internal("Failed to update workflow status")
            })?;

        // Record terminal workflow event based on new status
        match status {
            WorkflowStatus::Completed => {
                record_workflow_completed(
                    store.as_ref(),
                    workflow_id,
                    output.unwrap_or_else(|| serde_json::json!({})),
                )
                .await;
            }
            WorkflowStatus::Failed => {
                record_workflow_failed(
                    store.as_ref(),
                    workflow_id,
                    req.error.unwrap_or_else(|| "Unknown error".to_string()),
                )
                .await;
            }
            WorkflowStatus::Cancelled => {
                record_workflow_cancelled(
                    store.as_ref(),
                    workflow_id,
                    Some(req.error.unwrap_or_else(|| "Cancelled".to_string())),
                )
                .await;
            }
            _ => {
                // No event for Pending/Running status changes
            }
        }

        Ok(Response::new(UpdateDurableWorkflowStatusResponse {
            updated: true,
        }))
    }

    async fn enqueue_durable_task(
        &self,
        request: Request<EnqueueDurableTaskRequest>,
    ) -> Result<Response<EnqueueDurableTaskResponse>, Status> {
        let req = request.into_inner();
        let store = self.durable_store()?;

        let task_def = req
            .task
            .ok_or_else(|| Status::invalid_argument("Missing task definition"))?;
        let workflow_id = parse_uuid(task_def.workflow_id.as_ref())?;

        let input = task_def
            .input
            .map(|s| everruns_internal_protocol::proto_struct_to_json(&s))
            .unwrap_or_else(|| serde_json::json!({}));

        // For now, use default activity options
        // TODO: Map proto options to ActivityOptions when needed
        let options = ActivityOptions::default();

        let task = TaskDefinition {
            workflow_id: Some(workflow_id),
            activity_id: task_def.activity_id.clone(),
            activity_type: task_def.activity_type.clone(),
            input: input.clone(),
            options: options.clone(),
        };

        let task_id = store.enqueue_task(task).await.map_err(|e| {
            if matches!(
                e,
                everruns_durable::StoreError::TaskQueueLimitExceeded { .. }
            ) {
                tracing::warn!("Task queue limit exceeded: {}", e);
                Status::resource_exhausted(e.to_string())
            } else {
                tracing::error!("Failed to enqueue task: {}", e);
                Status::internal("Failed to enqueue task")
            }
        })?;

        // Record ActivityScheduled event
        let event = WorkflowEvent::ActivityScheduled {
            activity_id: task_def.activity_id,
            activity_type: task_def.activity_type,
            input,
            options,
        };
        let _ = append_event(store.as_ref(), workflow_id, event).await;

        use everruns_internal_protocol::uuid_to_proto_uuid;
        Ok(Response::new(EnqueueDurableTaskResponse {
            task_id: Some(uuid_to_proto_uuid(task_id)),
        }))
    }

    async fn claim_durable_tasks(
        &self,
        request: Request<ClaimDurableTasksRequest>,
    ) -> Result<Response<ClaimDurableTasksResponse>, Status> {
        use everruns_internal_protocol::uuid_to_proto_uuid;

        let req = request.into_inner();
        let store = self.durable_store()?;

        let tasks = store
            .claim_task(&req.worker_id, &req.activity_types, req.max_tasks as usize)
            .await
            .map_err(|e| {
                tracing::error!("Failed to claim tasks: {}", e);
                Status::internal("Failed to claim tasks")
            })?;

        // Record ActivityStarted events for each claimed task
        for task in &tasks {
            record_activity_started(
                store.as_ref(),
                task.workflow_id,
                task.activity_id.clone(),
                task.attempt,
                req.worker_id.clone(),
            )
            .await;
        }

        let proto_tasks: Vec<proto::DurableClaimedTask> = tasks
            .into_iter()
            .map(|t| proto::DurableClaimedTask {
                id: Some(uuid_to_proto_uuid(t.id)),
                workflow_id: t.workflow_id.map(uuid_to_proto_uuid),
                activity_id: t.activity_id,
                activity_type: t.activity_type,
                input: Some(everruns_internal_protocol::json_to_proto_struct(&t.input)),
                attempt: t.attempt as i32,
            })
            .collect();

        Ok(Response::new(ClaimDurableTasksResponse {
            tasks: proto_tasks,
        }))
    }

    async fn complete_durable_task(
        &self,
        request: Request<CompleteDurableTaskRequest>,
    ) -> Result<Response<CompleteDurableTaskResponse>, Status> {
        let req = request.into_inner();
        let store = self.durable_store()?;
        let task_id = parse_uuid(req.task_id.as_ref())?;
        let worker_id = &req.worker_id;

        let output = req
            .output
            .map(|s| everruns_internal_protocol::proto_struct_to_json(&s))
            .unwrap_or_else(|| serde_json::json!({}));

        // Get task info before completing (to get workflow_id and activity_id for event)
        let task_info = match store.get_task(task_id).await {
            Ok(info) => Some(info),
            Err(e) => {
                tracing::warn!(%task_id, error = %e, "Failed to get task info for event");
                None
            }
        };

        // complete_task now verifies worker ownership to prevent duplicate scheduling
        match store
            .complete_task(task_id, worker_id, output.clone())
            .await
        {
            Ok(()) => {
                // Record ActivityCompleted event
                if let Some(info) = task_info {
                    record_activity_completed(
                        store.as_ref(),
                        info.workflow_id,
                        info.activity_id,
                        output,
                    )
                    .await;
                }
                Ok(Response::new(CompleteDurableTaskResponse { success: true }))
            }
            Err(StoreError::TaskNotOwned(_)) => {
                // Task was reclaimed by another worker - not an error, just return false
                tracing::info!(
                    %task_id,
                    %worker_id,
                    "Task completion rejected: task was reclaimed or already completed"
                );
                Ok(Response::new(CompleteDurableTaskResponse {
                    success: false,
                }))
            }
            Err(e) => {
                tracing::error!("Failed to complete task: {}", e);
                Err(Status::internal("Failed to complete task"))
            }
        }
    }

    async fn fail_durable_task(
        &self,
        request: Request<FailDurableTaskRequest>,
    ) -> Result<Response<FailDurableTaskResponse>, Status> {
        let req = request.into_inner();
        let store = self.durable_store()?;
        let task_id = parse_uuid(req.task_id.as_ref())?;

        // Get task info before failing (to get workflow_id and activity_id for event)
        let task_info = match store.get_task(task_id).await {
            Ok(info) => Some(info),
            Err(e) => {
                tracing::warn!(%task_id, error = %e, "Failed to get task info for event");
                None
            }
        };

        let outcome = store.fail_task(task_id, &req.error).await.map_err(|e| {
            tracing::error!("Failed to fail task: {}", e);
            Status::internal("Failed to fail task")
        })?;

        // Check if task will be retried
        let will_retry = matches!(outcome, TaskFailureOutcome::WillRetry { .. });

        // Record ActivityFailed event
        if let Some(info) = task_info {
            record_activity_failed(
                store.as_ref(),
                info.workflow_id,
                info.activity_id,
                req.error.clone(),
                will_retry,
            )
            .await;
        }

        Ok(Response::new(FailDurableTaskResponse {
            failed: true,
            will_retry,
        }))
    }

    async fn heartbeat_durable_task(
        &self,
        request: Request<HeartbeatDurableTaskRequest>,
    ) -> Result<Response<HeartbeatDurableTaskResponse>, Status> {
        let req = request.into_inner();
        let store = self.durable_store()?;
        let task_id = parse_uuid(req.task_id.as_ref())?;

        let details = req
            .details
            .map(|s| everruns_internal_protocol::proto_struct_to_json(&s));

        let response = store
            .heartbeat_task(task_id, &req.worker_id, details)
            .await
            .map_err(|e| {
                tracing::error!("Failed to heartbeat task: {}", e);
                Status::internal("Failed to heartbeat task")
            })?;

        Ok(Response::new(HeartbeatDurableTaskResponse {
            acknowledged: response.accepted,
            should_cancel: response.should_cancel,
        }))
    }

    async fn count_active_durable_workflows(
        &self,
        _request: Request<CountActiveDurableWorkflowsRequest>,
    ) -> Result<Response<CountActiveDurableWorkflowsResponse>, Status> {
        let store = self.durable_store()?;

        let count = store.count_active_workflows().await.map_err(|e| {
            tracing::error!("Failed to count active workflows: {}", e);
            Status::internal("Failed to count active workflows")
        })?;

        Ok(Response::new(CountActiveDurableWorkflowsResponse { count }))
    }

    async fn register_durable_worker(
        &self,
        request: Request<RegisterDurableWorkerRequest>,
    ) -> Result<Response<RegisterDurableWorkerResponse>, Status> {
        let req = request.into_inner();
        let store = self.durable_store()?;

        let worker_info = WorkerInfo {
            id: req.worker_id,
            worker_group: req.worker_group,
            activity_types: req.activity_types,
            max_concurrency: req.max_concurrency as u32,
            current_load: 0,
            status: "active".to_string(),
            accepting_tasks: true,
            backpressure_reason: None,
            started_at: chrono::Utc::now(),
            last_heartbeat_at: chrono::Utc::now(),
            hostname: None,
            version: None,
            metadata: None,
            tasks_completed: 0,
            tasks_failed: 0,
            avg_task_duration_ms: None,
        };

        store.register_worker(worker_info).await.map_err(|e| {
            tracing::error!("Failed to register worker: {}", e);
            Status::internal("Failed to register worker")
        })?;

        Ok(Response::new(RegisterDurableWorkerResponse {
            registered: true,
        }))
    }

    async fn heartbeat_durable_worker(
        &self,
        request: Request<HeartbeatDurableWorkerRequest>,
    ) -> Result<Response<HeartbeatDurableWorkerResponse>, Status> {
        let req = request.into_inner();
        let store = self.durable_store()?;

        store
            .worker_heartbeat(
                &req.worker_id,
                req.current_load as usize,
                req.accepting_tasks,
            )
            .await
            .map_err(|e| {
                tracing::error!("Failed to heartbeat worker: {}", e);
                Status::internal("Failed to heartbeat worker")
            })?;

        Ok(Response::new(HeartbeatDurableWorkerResponse {
            acknowledged: true,
        }))
    }

    async fn deregister_durable_worker(
        &self,
        request: Request<DeregisterDurableWorkerRequest>,
    ) -> Result<Response<DeregisterDurableWorkerResponse>, Status> {
        let req = request.into_inner();
        let store = self.durable_store()?;

        let tasks_reclaimed = store.deregister_worker(&req.worker_id).await.map_err(|e| {
            tracing::error!("Failed to deregister worker: {}", e);
            Status::internal("Failed to deregister worker")
        })?;

        if tasks_reclaimed > 0 {
            tracing::info!(
                worker_id = %req.worker_id,
                tasks_reclaimed,
                "Worker deregistered with task reclamation"
            );
        }

        Ok(Response::new(DeregisterDurableWorkerResponse {
            deregistered: true,
            tasks_reclaimed: tasks_reclaimed as i32,
        }))
    }

    // ========================================================================
    // Circuit breaker operations
    // ========================================================================

    async fn check_circuit_breaker(
        &self,
        request: Request<CheckCircuitBreakerRequest>,
    ) -> Result<Response<CheckCircuitBreakerResponse>, Status> {
        let req = request.into_inner();
        let store = self.durable_store()?;

        // Create a circuit breaker instance for this key
        let config = CircuitBreakerConfig::default();
        let store_dyn: Arc<dyn WorkflowEventStore> = store.clone();
        let breaker = DistributedCircuitBreaker::new(req.key.clone(), config, store_dyn);

        // Try to get a permit (this checks if the circuit allows the call)
        match breaker.allow().await {
            Ok(_permit) => {
                // Permit granted - get current state
                let state = breaker.state().await.unwrap_or(CircuitState::Closed);
                Ok(Response::new(CheckCircuitBreakerResponse {
                    allowed: true,
                    state: circuit_state_to_proto(state).into(),
                }))
            }
            Err(e) => {
                // Circuit is open or half-open
                let state = match e {
                    everruns_durable::CircuitBreakerError::Open => CircuitState::Open,
                    _ => {
                        tracing::error!("Circuit breaker error: {}", e);
                        CircuitState::Closed
                    }
                };
                Ok(Response::new(CheckCircuitBreakerResponse {
                    allowed: false,
                    state: circuit_state_to_proto(state).into(),
                }))
            }
        }
    }

    async fn record_circuit_breaker_success(
        &self,
        request: Request<RecordCircuitBreakerSuccessRequest>,
    ) -> Result<Response<RecordCircuitBreakerSuccessResponse>, Status> {
        let req = request.into_inner();
        let store = self.durable_store()?;

        // Create a circuit breaker instance and record success
        let config = CircuitBreakerConfig::default();
        let store_dyn: Arc<dyn WorkflowEventStore> = store.clone();
        let breaker = DistributedCircuitBreaker::new(req.key.clone(), config, store_dyn);

        // Get permit and record success
        if let Ok(permit) = breaker.allow().await {
            permit.success().await.map_err(|e| {
                tracing::error!("Failed to record circuit breaker success: {}", e);
                Status::internal("Failed to record success")
            })?;
        }

        let state = breaker.state().await.unwrap_or(CircuitState::Closed);
        Ok(Response::new(RecordCircuitBreakerSuccessResponse {
            state: circuit_state_to_proto(state).into(),
        }))
    }

    async fn record_circuit_breaker_failure(
        &self,
        request: Request<RecordCircuitBreakerFailureRequest>,
    ) -> Result<Response<RecordCircuitBreakerFailureResponse>, Status> {
        let req = request.into_inner();
        let store = self.durable_store()?;

        // Create a circuit breaker instance and record failure
        let config = CircuitBreakerConfig::default();
        let store_dyn: Arc<dyn WorkflowEventStore> = store.clone();
        let breaker = DistributedCircuitBreaker::new(req.key.clone(), config, store_dyn);

        // Get the state before recording failure
        let state_before = breaker.state().await.unwrap_or(CircuitState::Closed);

        // Get permit and record failure
        if let Ok(permit) = breaker.allow().await {
            permit.failure().await.map_err(|e| {
                tracing::error!("Failed to record circuit breaker failure: {}", e);
                Status::internal("Failed to record failure")
            })?;
        }

        // Get the state after recording failure
        let state_after = breaker.state().await.unwrap_or(CircuitState::Closed);
        let circuit_opened =
            state_before != CircuitState::Open && state_after == CircuitState::Open;

        if circuit_opened {
            tracing::warn!(key = %req.key, "Circuit breaker opened");
        }

        Ok(Response::new(RecordCircuitBreakerFailureResponse {
            state: circuit_state_to_proto(state_after).into(),
            circuit_opened,
        }))
    }

    // ========================================================================
    // Push-based task notifications
    // ========================================================================

    /// Stream type for task notifications
    type SubscribeTaskNotificationsStream =
        Pin<Box<dyn futures::Stream<Item = Result<TaskNotification, Status>> + Send>>;

    async fn subscribe_task_notifications(
        &self,
        request: Request<SubscribeTaskNotificationsRequest>,
    ) -> Result<Response<Self::SubscribeTaskNotificationsStream>, Status> {
        let req = request.into_inner();
        let worker_id = req.worker_id.clone();
        let activity_types = req.activity_types.clone();

        // Get the broadcaster
        let broadcaster = self
            .task_broadcaster
            .as_ref()
            .ok_or_else(|| Status::unavailable("Task notifications not enabled"))?
            .clone();

        tracing::info!(
            worker_id = %worker_id,
            activity_types = ?activity_types,
            "Worker subscribing to task notifications"
        );

        // Subscribe to notifications
        let subscription = broadcaster
            .subscribe(worker_id.clone(), activity_types.clone())
            .await;

        // Create a channel for the stream
        let (tx, rx) = tokio::sync::mpsc::channel(128);
        let activity_types_set: std::collections::HashSet<String> =
            activity_types.into_iter().collect();

        // Spawn a task to forward notifications to the stream
        let broadcaster_for_cleanup = broadcaster.clone();
        let worker_id_for_cleanup = worker_id.clone();
        tokio::spawn(async move {
            let mut receiver = subscription.receiver;
            let mut heartbeat_interval = tokio::time::interval(std::time::Duration::from_secs(30));

            loop {
                tokio::select! {
                    // Handle incoming task notifications
                    notification = receiver.recv() => {
                        match notification {
                            Ok(payload) => {
                                // Filter by activity type
                                if activity_types_set.contains(&payload.activity_type) {
                                    let proto_notification = TaskNotification {
                                        notification_type: TaskNotificationType::TaskAvailable.into(),
                                        activity_type: payload.activity_type,
                                        pending_count: payload.pending_count,
                                        timestamp: Some(everruns_internal_protocol::datetime_to_proto_timestamp(
                                            chrono::Utc::now(),
                                        )),
                                    };

                                    if tx.send(Ok(proto_notification)).await.is_err() {
                                        // Client disconnected
                                        tracing::debug!(
                                            worker_id = %worker_id,
                                            "Client disconnected from task notification stream"
                                        );
                                        break;
                                    }
                                }
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(count)) => {
                                // Missed some notifications, but that's OK - worker will poll
                                tracing::warn!(
                                    worker_id = %worker_id,
                                    missed_count = count,
                                    "Task notification stream lagged"
                                );
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                                // Broadcaster shut down
                                tracing::info!(
                                    worker_id = %worker_id,
                                    "Task notification broadcaster shut down"
                                );
                                break;
                            }
                        }
                    }

                    // Send periodic heartbeats
                    _ = heartbeat_interval.tick() => {
                        let heartbeat = TaskNotification {
                            notification_type: TaskNotificationType::Heartbeat.into(),
                            activity_type: String::new(),
                            pending_count: 0,
                            timestamp: Some(everruns_internal_protocol::datetime_to_proto_timestamp(
                                chrono::Utc::now(),
                            )),
                        };

                        if tx.send(Ok(heartbeat)).await.is_err() {
                            // Client disconnected
                            tracing::debug!(
                                worker_id = %worker_id,
                                "Client disconnected during heartbeat"
                            );
                            break;
                        }
                    }
                }
            }

            // Clean up subscription
            broadcaster_for_cleanup
                .unsubscribe(&worker_id_for_cleanup)
                .await;
        });

        // Return the receiver as a stream
        let stream = ReceiverStream::new(rx);
        Ok(Response::new(Box::pin(stream)))
    }

    // =========================================================================
    // Image Resolution Operations
    // =========================================================================

    /// Resolve a single image by ID
    ///
    /// Returns base64-encoded image data and media type for LLM consumption.
    /// This is used by workers to resolve `image_file` content parts before
    /// sending messages to LLM providers.
    async fn resolve_image(
        &self,
        request: Request<ResolveImageRequest>,
    ) -> Result<Response<ResolveImageResponse>, Status> {
        let req = request.into_inner();
        let image_id = parse_uuid(req.image_id.as_ref())?;

        // Get image from storage (org-scoped)
        let image_row = match self.db.get_image(req.org_id, image_id).await {
            Ok(Some(row)) => row,
            Ok(None) => {
                return Ok(Response::new(ResolveImageResponse {
                    found: false,
                    base64: String::new(),
                    media_type: String::new(),
                }));
            }
            Err(e) => {
                tracing::error!(%image_id, error = %e, "Failed to get image");
                return Err(Status::internal("Failed to get image"));
            }
        };

        // Encode to base64
        let base64_data = base64::engine::general_purpose::STANDARD.encode(&image_row.data);

        Ok(Response::new(ResolveImageResponse {
            found: true,
            base64: base64_data,
            media_type: image_row.content_type,
        }))
    }

    /// Resolve multiple images in a batch
    ///
    /// More efficient than multiple single image calls for multimodal messages
    /// with multiple images.
    async fn resolve_images(
        &self,
        request: Request<ResolveImagesRequest>,
    ) -> Result<Response<ResolveImagesResponse>, Status> {
        let req = request.into_inner();

        let mut images = std::collections::HashMap::new();

        for proto_id in req.image_ids {
            let image_id = parse_uuid(Some(&proto_id))?;

            // Get image from storage (org-scoped)
            match self.db.get_image(req.org_id, image_id).await {
                Ok(Some(row)) => {
                    let base64_data = base64::engine::general_purpose::STANDARD.encode(&row.data);
                    images.insert(
                        image_id.to_string(),
                        ResolvedImageData {
                            base64: base64_data,
                            media_type: row.content_type,
                        },
                    );
                }
                Ok(None) => {
                    // Image not found - skip it
                    tracing::debug!(%image_id, "Image not found during batch resolution");
                }
                Err(e) => {
                    tracing::warn!(%image_id, error = %e, "Failed to get image during batch resolution");
                }
            }
        }

        Ok(Response::new(ResolveImagesResponse { images }))
    }

    // ========================================================================
    // MCP server operations
    // ========================================================================

    async fn get_mcp_server_by_prefix(
        &self,
        request: Request<GetMcpServerByPrefixRequest>,
    ) -> Result<Response<GetMcpServerByPrefixResponse>, Status> {
        let req = request.into_inner();

        let internal_caller = everruns_core::Caller::internal(req.org_id);
        let resolved = self
            .mcp_server_service
            .resolve_by_prefix(&internal_caller, &req.server_prefix)
            .await
            .map_err(|e| {
                tracing::error!("Failed to resolve MCP server: {}", e);
                Status::internal("Failed to resolve MCP server")
            })?;

        let server_info = resolved.map(|r| McpServerInfo {
            id: Some(proto::Uuid {
                value: r.id.to_string(),
            }),
            name: r.name,
            url: r.url,
            api_key: r.api_key,
            headers: r.headers,
        });

        Ok(Response::new(GetMcpServerByPrefixResponse {
            server: server_info,
        }))
    }

    // =========================================================================
    // Session Storage Operations
    // =========================================================================

    async fn session_storage_set_value(
        &self,
        request: Request<SessionStorageSetValueRequest>,
    ) -> Result<Response<SessionStorageSetValueResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let store = self.storage_store()?;

        store
            .set_value(session_id.into(), &req.key, &req.value)
            .await
            .map_err(|e| {
                tracing::error!("Failed to set storage value: {}", e);
                Status::internal("Failed to set storage value")
            })?;

        Ok(Response::new(SessionStorageSetValueResponse {}))
    }

    async fn session_storage_get_value(
        &self,
        request: Request<SessionStorageGetValueRequest>,
    ) -> Result<Response<SessionStorageGetValueResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let store = self.storage_store()?;

        let value = store
            .get_value(session_id.into(), &req.key)
            .await
            .map_err(|e| {
                tracing::error!("Failed to get storage value: {}", e);
                Status::internal("Failed to get storage value")
            })?;

        Ok(Response::new(SessionStorageGetValueResponse { value }))
    }

    async fn session_storage_delete_value(
        &self,
        request: Request<SessionStorageDeleteValueRequest>,
    ) -> Result<Response<SessionStorageDeleteValueResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let store = self.storage_store()?;

        let deleted = store
            .delete_value(session_id.into(), &req.key)
            .await
            .map_err(|e| {
                tracing::error!("Failed to delete storage value: {}", e);
                Status::internal("Failed to delete storage value")
            })?;

        Ok(Response::new(SessionStorageDeleteValueResponse { deleted }))
    }

    async fn session_storage_list_keys(
        &self,
        request: Request<SessionStorageListKeysRequest>,
    ) -> Result<Response<SessionStorageListKeysResponse>, Status> {
        use everruns_internal_protocol::datetime_to_proto_timestamp;

        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let store = self.storage_store()?;

        let keys = store.list_keys(session_id.into()).await.map_err(|e| {
            tracing::error!("Failed to list storage keys: {}", e);
            Status::internal("Failed to list storage keys")
        })?;

        let proto_keys = keys
            .into_iter()
            .map(|k| proto::StorageKeyInfo {
                key: k.key,
                created_at: Some(datetime_to_proto_timestamp(k.created_at)),
                updated_at: Some(datetime_to_proto_timestamp(k.updated_at)),
            })
            .collect();

        Ok(Response::new(SessionStorageListKeysResponse {
            keys: proto_keys,
        }))
    }

    async fn session_storage_set_secret(
        &self,
        request: Request<SessionStorageSetSecretRequest>,
    ) -> Result<Response<SessionStorageSetSecretResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let store = self.storage_store()?;

        store
            .set_secret(session_id.into(), &req.name, &req.value)
            .await
            .map_err(|e| {
                tracing::error!("Failed to set secret: {}", e);
                Status::internal("Failed to set secret")
            })?;

        Ok(Response::new(SessionStorageSetSecretResponse {}))
    }

    async fn session_storage_get_secret(
        &self,
        request: Request<SessionStorageGetSecretRequest>,
    ) -> Result<Response<SessionStorageGetSecretResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let store = self.storage_store()?;

        let value = store
            .get_secret(session_id.into(), &req.name)
            .await
            .map_err(|e| {
                tracing::error!("Failed to get secret: {}", e);
                Status::internal("Failed to get secret")
            })?;

        Ok(Response::new(SessionStorageGetSecretResponse { value }))
    }

    async fn session_storage_delete_secret(
        &self,
        request: Request<SessionStorageDeleteSecretRequest>,
    ) -> Result<Response<SessionStorageDeleteSecretResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let store = self.storage_store()?;

        let deleted = store
            .delete_secret(session_id.into(), &req.name)
            .await
            .map_err(|e| {
                tracing::error!("Failed to delete secret: {}", e);
                Status::internal("Failed to delete secret")
            })?;

        Ok(Response::new(SessionStorageDeleteSecretResponse {
            deleted,
        }))
    }

    async fn session_storage_list_secrets(
        &self,
        request: Request<SessionStorageListSecretsRequest>,
    ) -> Result<Response<SessionStorageListSecretsResponse>, Status> {
        use everruns_internal_protocol::datetime_to_proto_timestamp;

        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let store = self.storage_store()?;

        let secrets = store.list_secrets(session_id.into()).await.map_err(|e| {
            tracing::error!("Failed to list secrets: {}", e);
            Status::internal("Failed to list secrets")
        })?;

        let proto_secrets = secrets
            .into_iter()
            .map(|s| proto::StorageSecretInfo {
                name: s.name,
                created_at: Some(datetime_to_proto_timestamp(s.created_at)),
                updated_at: Some(datetime_to_proto_timestamp(s.updated_at)),
            })
            .collect();

        Ok(Response::new(SessionStorageListSecretsResponse {
            secrets: proto_secrets,
        }))
    }

    // ========================================================================
    // Connection token resolution
    // ========================================================================

    async fn get_connection_token(
        &self,
        request: Request<GetConnectionTokenRequest>,
    ) -> Result<Response<GetConnectionTokenResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let resolver = self.connection_resolver()?;

        let token = resolver
            .get_connection_token(session_id.into(), &req.provider)
            .await
            .map_err(|e| {
                tracing::error!("Failed to resolve connection token: {}", e);
                Status::internal("Failed to resolve connection token")
            })?;

        Ok(Response::new(GetConnectionTokenResponse { token }))
    }

    async fn get_connection_user(
        &self,
        request: Request<GetConnectionUserRequest>,
    ) -> Result<Response<GetConnectionUserResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let resolver = self.connection_resolver()?;

        let user_id = resolver
            .get_connection_user(session_id.into(), &req.provider)
            .await
            .map_err(|e| {
                tracing::error!("Failed to resolve connection owner: {}", e);
                Status::internal("Failed to resolve connection owner")
            })?;

        Ok(Response::new(GetConnectionUserResponse {
            user_id: user_id.map(|user_id| proto::Uuid {
                value: user_id.to_string(),
            }),
        }))
    }

    async fn get_connection_token_for_user(
        &self,
        request: Request<GetConnectionTokenForUserRequest>,
    ) -> Result<Response<GetConnectionTokenForUserResponse>, Status> {
        let req = request.into_inner();
        let user_id = parse_uuid(req.user_id.as_ref())?;
        let resolver = self.connection_resolver()?;

        let token = resolver
            .get_connection_token_for_user(user_id, &req.provider)
            .await
            .map_err(|e| {
                tracing::error!("Failed to resolve connection token for user: {}", e);
                Status::internal("Failed to resolve connection token for user")
            })?;

        Ok(Response::new(GetConnectionTokenForUserResponse { token }))
    }

    // ========================================================================
    // Leased resource operations
    // ========================================================================

    async fn upsert_leased_resource(
        &self,
        request: Request<UpsertLeasedResourceRequest>,
    ) -> Result<Response<UpsertLeasedResourceResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let store = self.leased_resource_store();

        let resource = store
            .upsert_resource(everruns_core::UpsertLeasedResource {
                session_id: session_id.into(),
                provider: req.provider,
                resource_type: req.resource_type,
                external_id: req.external_id,
                display_name: req.display_name,
                owner_user_id: req
                    .owner_user_id
                    .as_ref()
                    .map(|id| parse_uuid(Some(id)))
                    .transpose()?,
                lease_duration_seconds: req.lease_duration_seconds,
                metadata: req
                    .metadata
                    .as_ref()
                    .map(everruns_internal_protocol::proto_struct_to_json)
                    .unwrap_or_else(|| serde_json::json!({})),
            })
            .await
            .map_err(|e| {
                tracing::error!("Failed to upsert leased resource: {}", e);
                Status::internal("Failed to upsert leased resource")
            })?;

        Ok(Response::new(UpsertLeasedResourceResponse {
            resource: Some(leased_resource_to_proto(&resource)),
        }))
    }

    async fn release_leased_resource(
        &self,
        request: Request<ReleaseLeasedResourceRequest>,
    ) -> Result<Response<ReleaseLeasedResourceResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let store = self.leased_resource_store();

        let resource = store
            .release_resource(
                session_id.into(),
                &req.provider,
                &req.resource_type,
                &req.external_id,
            )
            .await
            .map_err(|e| {
                tracing::error!("Failed to release leased resource: {}", e);
                Status::internal("Failed to release leased resource")
            })?;

        Ok(Response::new(ReleaseLeasedResourceResponse {
            resource: resource.as_ref().map(leased_resource_to_proto),
        }))
    }

    async fn list_session_leased_resources(
        &self,
        request: Request<ListSessionLeasedResourcesRequest>,
    ) -> Result<Response<ListSessionLeasedResourcesResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let store = self.leased_resource_store();

        let resources = store.list_resources(session_id.into()).await.map_err(|e| {
            tracing::error!("Failed to list leased resources: {}", e);
            Status::internal("Failed to list leased resources")
        })?;

        Ok(Response::new(ListSessionLeasedResourcesResponse {
            resources: resources.iter().map(leased_resource_to_proto).collect(),
        }))
    }

    async fn claim_due_leased_resources(
        &self,
        request: Request<ClaimDueLeasedResourcesRequest>,
    ) -> Result<Response<ClaimDueLeasedResourcesResponse>, Status> {
        let req = request.into_inner();
        let rows = self
            .db
            .claim_due_leased_resources(req.limit as i32, req.stale_after_seconds as i32)
            .await
            .map_err(|e| {
                tracing::error!("Failed to claim leased resources: {}", e);
                Status::internal("Failed to claim leased resources")
            })?;

        let resources = rows
            .iter()
            .map(crate::storage::leased_resource_row_to_domain)
            .collect::<everruns_core::Result<Vec<_>>>()
            .map_err(|e| {
                tracing::error!("Failed to map leased resources: {}", e);
                Status::internal("Failed to map leased resources")
            })?;

        Ok(Response::new(ClaimDueLeasedResourcesResponse {
            resources: resources.iter().map(leased_resource_to_proto).collect(),
        }))
    }

    async fn mark_leased_resource_released(
        &self,
        request: Request<MarkLeasedResourceReleasedRequest>,
    ) -> Result<Response<MarkLeasedResourceReleasedResponse>, Status> {
        let req = request.into_inner();
        let resource_id = parse_uuid(req.resource_id.as_ref())?;
        let expected_cleanup_started_at = req
            .expected_cleanup_started_at
            .as_ref()
            .map(everruns_internal_protocol::proto_timestamp_to_datetime)
            .ok_or_else(|| Status::invalid_argument("Missing expected_cleanup_started_at"))?;

        let updated = self
            .db
            .mark_leased_resource_released(
                everruns_core::LeasedResourceId::from_uuid(resource_id),
                expected_cleanup_started_at,
            )
            .await
            .map_err(|e| {
                tracing::error!("Failed to mark leased resource released: {}", e);
                Status::internal("Failed to mark leased resource released")
            })?
            .is_some();

        Ok(Response::new(MarkLeasedResourceReleasedResponse {
            updated,
        }))
    }

    async fn mark_leased_resource_cleanup_failed(
        &self,
        request: Request<MarkLeasedResourceCleanupFailedRequest>,
    ) -> Result<Response<MarkLeasedResourceCleanupFailedResponse>, Status> {
        let req = request.into_inner();
        let resource_id = parse_uuid(req.resource_id.as_ref())?;
        let expected_cleanup_started_at = req
            .expected_cleanup_started_at
            .as_ref()
            .map(everruns_internal_protocol::proto_timestamp_to_datetime)
            .ok_or_else(|| Status::invalid_argument("Missing expected_cleanup_started_at"))?;

        let updated = self
            .db
            .mark_leased_resource_cleanup_failed(
                everruns_core::LeasedResourceId::from_uuid(resource_id),
                expected_cleanup_started_at,
                req.retry_after_seconds as i32,
                &req.error,
            )
            .await
            .map_err(|e| {
                tracing::error!("Failed to mark leased resource cleanup failed: {}", e);
                Status::internal("Failed to mark leased resource cleanup failed")
            })?
            .is_some();

        Ok(Response::new(MarkLeasedResourceCleanupFailedResponse {
            updated,
        }))
    }

    // ========================================================================
    // Session schedule operations
    // ========================================================================

    async fn create_session_schedule(
        &self,
        request: Request<CreateSessionScheduleRequest>,
    ) -> Result<Response<CreateSessionScheduleResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let store = self.schedule_store(req.org_id)?;

        let scheduled_at = req
            .scheduled_at
            .as_ref()
            .map(everruns_internal_protocol::proto_timestamp_to_datetime);

        let schedule = store
            .create_schedule(
                session_id.into(),
                req.description,
                req.cron_expression,
                scheduled_at,
                req.timezone,
            )
            .await
            .map_err(|e| {
                tracing::error!("Failed to create schedule: {}", e);
                Status::internal(format!("Failed to create schedule: {}", e))
            })?;

        Ok(Response::new(CreateSessionScheduleResponse {
            schedule: Some(session_schedule_to_proto(&schedule)),
        }))
    }

    async fn cancel_session_schedule(
        &self,
        request: Request<CancelSessionScheduleRequest>,
    ) -> Result<Response<CancelSessionScheduleResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let schedule_id = parse_uuid(req.schedule_id.as_ref())?;
        let store = self.schedule_store(req.org_id)?;

        let schedule = store
            .cancel_schedule(
                session_id.into(),
                everruns_core::typed_id::ScheduleId::from_uuid(schedule_id),
            )
            .await
            .map_err(|e| {
                tracing::error!("Failed to cancel schedule: {}", e);
                Status::internal(format!("Failed to cancel schedule: {}", e))
            })?;

        Ok(Response::new(CancelSessionScheduleResponse {
            schedule: Some(session_schedule_to_proto(&schedule)),
        }))
    }

    async fn list_session_schedules(
        &self,
        request: Request<ListSessionSchedulesRequest>,
    ) -> Result<Response<ListSessionSchedulesResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let store = self.schedule_store(req.org_id)?;

        let schedules = store.list_schedules(session_id.into()).await.map_err(|e| {
            tracing::error!("Failed to list schedules: {}", e);
            Status::internal(format!("Failed to list schedules: {}", e))
        })?;

        let proto_schedules = schedules.iter().map(session_schedule_to_proto).collect();

        Ok(Response::new(ListSessionSchedulesResponse {
            schedules: proto_schedules,
        }))
    }

    async fn count_active_session_schedules(
        &self,
        request: Request<CountActiveSessionSchedulesRequest>,
    ) -> Result<Response<CountActiveSessionSchedulesResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let store = self.schedule_store(req.org_id)?;

        let count = store
            .count_active_schedules(session_id.into())
            .await
            .map_err(|e| {
                tracing::error!("Failed to count active schedules: {}", e);
                Status::internal(format!("Failed to count active schedules: {}", e))
            })?;

        Ok(Response::new(CountActiveSessionSchedulesResponse { count }))
    }

    // ========================================================================
    // Session SQL database operations
    // ========================================================================

    async fn session_sql_db_create_database(
        &self,
        request: Request<SessionSqlDbCreateDatabaseRequest>,
    ) -> Result<Response<SessionSqlDbCreateDatabaseResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let store = self.sqldb_store()?;

        let db = store
            .create_database(session_id.into(), &req.name)
            .await
            .map_err(sqldb_error_to_status)?;

        Ok(Response::new(SessionSqlDbCreateDatabaseResponse {
            database: Some(db_info_to_proto(db)),
        }))
    }

    async fn session_sql_db_list_databases(
        &self,
        request: Request<SessionSqlDbListDatabasesRequest>,
    ) -> Result<Response<SessionSqlDbListDatabasesResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let store = self.sqldb_store()?;

        let databases = store
            .list_databases(session_id.into())
            .await
            .map_err(sqldb_error_to_status)?;

        Ok(Response::new(SessionSqlDbListDatabasesResponse {
            databases: databases.into_iter().map(db_info_to_proto).collect(),
        }))
    }

    async fn session_sql_db_get_database(
        &self,
        request: Request<SessionSqlDbGetDatabaseRequest>,
    ) -> Result<Response<SessionSqlDbGetDatabaseResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let store = self.sqldb_store()?;

        let db = store
            .get_database(session_id.into(), &req.name)
            .await
            .map_err(sqldb_error_to_status)?;

        Ok(Response::new(SessionSqlDbGetDatabaseResponse {
            database: db.map(db_info_to_proto),
        }))
    }

    async fn session_sql_db_delete_database(
        &self,
        request: Request<SessionSqlDbDeleteDatabaseRequest>,
    ) -> Result<Response<SessionSqlDbDeleteDatabaseResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let store = self.sqldb_store()?;

        let deleted = store
            .delete_database(session_id.into(), &req.name)
            .await
            .map_err(sqldb_error_to_status)?;

        Ok(Response::new(SessionSqlDbDeleteDatabaseResponse {
            deleted,
        }))
    }

    async fn session_sql_db_execute(
        &self,
        request: Request<SessionSqlDbExecuteRequest>,
    ) -> Result<Response<SessionSqlDbExecuteResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let store = self.sqldb_store()?;

        let result = store
            .sql_execute(session_id.into(), &req.db_name, &req.sql)
            .await
            .map_err(sqldb_error_to_status)?;

        Ok(Response::new(SessionSqlDbExecuteResponse {
            rows_affected: result.rows_affected,
        }))
    }

    async fn session_sql_db_query(
        &self,
        request: Request<SessionSqlDbQueryRequest>,
    ) -> Result<Response<SessionSqlDbQueryResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let store = self.sqldb_store()?;

        let result = store
            .sql_query(session_id.into(), &req.db_name, &req.sql)
            .await
            .map_err(sqldb_error_to_status)?;

        let proto_rows: Vec<prost_types::ListValue> = result
            .rows
            .into_iter()
            .map(|row| prost_types::ListValue {
                values: row.into_iter().map(json_value_to_proto).collect(),
            })
            .collect();

        Ok(Response::new(SessionSqlDbQueryResponse {
            columns: result.columns,
            rows: proto_rows,
            row_count: result.row_count as u64,
            truncated: result.truncated,
        }))
    }

    async fn session_sql_db_schema(
        &self,
        request: Request<SessionSqlDbSchemaRequest>,
    ) -> Result<Response<SessionSqlDbSchemaResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let store = self.sqldb_store()?;

        let tables = store
            .sql_schema(session_id.into(), &req.db_name, req.table.as_deref())
            .await
            .map_err(sqldb_error_to_status)?;

        let proto_tables: Vec<proto::SessionSqlDbTableSchema> = tables
            .into_iter()
            .map(|t| proto::SessionSqlDbTableSchema {
                name: t.name,
                columns: t
                    .columns
                    .into_iter()
                    .map(|c| proto::SessionSqlDbColumnSchema {
                        name: c.name,
                        column_type: c.column_type,
                        notnull: c.notnull,
                        pk: c.pk,
                        default_value: c.default_value,
                    })
                    .collect(),
                row_count: t.row_count,
            })
            .collect();

        Ok(Response::new(SessionSqlDbSchemaResponse {
            tables: proto_tables,
        }))
    }

    // ========================================================================
    // Platform management operations
    // ========================================================================

    async fn platform_list_harnesses(
        &self,
        request: Request<PlatformListHarnessesRequest>,
    ) -> Result<Response<PlatformListHarnessesResponse>, Status> {
        let req = request.into_inner();
        let internal_caller = everruns_core::Caller::internal(req.org_id);
        let harnesses = self
            .harness_service
            .list(&internal_caller, None, false)
            .await
            .map_err(|e| Status::internal(format!("Failed to list harnesses: {}", e)))?;

        let proto_harnesses = harnesses.iter().map(schema_harness_to_proto).collect();
        Ok(Response::new(PlatformListHarnessesResponse {
            harnesses: proto_harnesses,
        }))
    }

    async fn platform_create_harness(
        &self,
        request: Request<PlatformCreateHarnessRequest>,
    ) -> Result<Response<PlatformCreateHarnessResponse>, Status> {
        let req = request.into_inner();
        let capabilities: Vec<everruns_core::AgentCapabilityConfig> = req
            .capabilities
            .iter()
            .map(|id| everruns_core::AgentCapabilityConfig::new(id.as_str()))
            .collect();

        let create_req = crate::api::harnesses::CreateHarnessRequest {
            name: req.name,
            description: req.description,
            system_prompt: req.system_prompt,
            parent_harness_id: req
                .parent_harness_id
                .as_ref()
                .map(|parent_id| parse_uuid(Some(parent_id)))
                .transpose()?
                .map(everruns_core::HarnessId::from_uuid),
            default_model_id: None,
            tags: vec![],
            capabilities,
            initial_files: vec![],
        };

        let internal_caller = everruns_core::Caller::internal(req.org_id);
        let harness = self
            .harness_service
            .create(&internal_caller, create_req)
            .await
            .map_err(|e| Status::internal(format!("Failed to create harness: {}", e)))?;

        Ok(Response::new(PlatformCreateHarnessResponse {
            harness: Some(schema_harness_to_proto(&harness)),
        }))
    }

    async fn platform_update_harness(
        &self,
        request: Request<PlatformUpdateHarnessRequest>,
    ) -> Result<Response<PlatformUpdateHarnessResponse>, Status> {
        let req = request.into_inner();
        let harness_id = parse_uuid(req.harness_id.as_ref())?;

        let update_req = crate::api::harnesses::UpdateHarnessRequest {
            name: req.name,
            description: req.description,
            system_prompt: req.system_prompt,
            parent_harness_id: if req.clear_parent_harness_id.unwrap_or(false) {
                Some(None)
            } else {
                req.parent_harness_id
                    .as_ref()
                    .map(|parent_id| parse_uuid(Some(parent_id)))
                    .transpose()?
                    .map(everruns_core::HarnessId::from_uuid)
                    .map(Some)
            },
            default_model_id: None,
            tags: None,
            capabilities: None,
            initial_files: None,
            status: None,
        };

        let internal_caller = everruns_core::Caller::internal(req.org_id);
        let harness = self
            .harness_service
            .update(&internal_caller, harness_id, update_req)
            .await
            .map_err(|e| Status::internal(format!("Failed to update harness: {}", e)))?
            .ok_or_else(|| Status::not_found("Harness not found"))?;

        Ok(Response::new(PlatformUpdateHarnessResponse {
            harness: Some(schema_harness_to_proto(&harness)),
        }))
    }

    async fn platform_delete_harness(
        &self,
        request: Request<PlatformDeleteHarnessRequest>,
    ) -> Result<Response<PlatformDeleteHarnessResponse>, Status> {
        let req = request.into_inner();
        let harness_id = parse_uuid(req.harness_id.as_ref())?;

        let internal_caller = everruns_core::Caller::internal(req.org_id);
        self.harness_service
            .delete(&internal_caller, harness_id)
            .await
            .map_err(|e| Status::internal(format!("Failed to delete harness: {}", e)))?;

        Ok(Response::new(PlatformDeleteHarnessResponse {}))
    }

    async fn platform_copy_harness(
        &self,
        request: Request<PlatformCopyHarnessRequest>,
    ) -> Result<Response<PlatformCopyHarnessResponse>, Status> {
        let req = request.into_inner();
        let harness_id = parse_uuid(req.harness_id.as_ref())?;

        let internal_caller = everruns_core::Caller::internal(req.org_id);
        let harness = self
            .harness_service
            .copy(&internal_caller, harness_id)
            .await
            .map_err(|e| Status::internal(format!("Failed to copy harness: {}", e)))?
            .ok_or_else(|| Status::not_found("Harness not found"))?;

        // If a new_name was provided, update the copy with the new name
        let harness = if let Some(new_name) = req.new_name {
            let update_req = crate::api::harnesses::UpdateHarnessRequest {
                name: Some(new_name),
                description: None,
                system_prompt: None,
                parent_harness_id: None,
                default_model_id: None,
                tags: None,
                capabilities: None,
                initial_files: None,
                status: None,
            };
            self.harness_service
                .update(&internal_caller, harness.id.uuid(), update_req)
                .await
                .map_err(|e| Status::internal(format!("Failed to rename copied harness: {}", e)))?
                .unwrap_or(harness)
        } else {
            harness
        };

        Ok(Response::new(PlatformCopyHarnessResponse {
            harness: Some(schema_harness_to_proto(&harness)),
        }))
    }

    async fn platform_list_agents(
        &self,
        request: Request<PlatformListAgentsRequest>,
    ) -> Result<Response<PlatformListAgentsResponse>, Status> {
        let req = request.into_inner();
        let internal_caller = everruns_core::Caller::internal(req.org_id);
        let pagination = crate::api::common::Pagination::new(0, 1000);
        let (agents, _total) = self
            .agent_service
            .list(&internal_caller, None, false, pagination)
            .await
            .map_err(|e| Status::internal(format!("Failed to list agents: {}", e)))?;

        let proto_agents = agents.iter().map(schema_agent_to_proto).collect();
        Ok(Response::new(PlatformListAgentsResponse {
            agents: proto_agents,
        }))
    }

    async fn platform_create_agent(
        &self,
        request: Request<PlatformCreateAgentRequest>,
    ) -> Result<Response<PlatformCreateAgentResponse>, Status> {
        let req = request.into_inner();
        let capabilities: Vec<everruns_core::AgentCapabilityConfig> = req
            .capabilities
            .iter()
            .map(|id| everruns_core::AgentCapabilityConfig::new(id.as_str()))
            .collect();

        let create_req = crate::api::agents::CreateAgentRequest {
            id: None,
            name: req.name,
            description: req.description,
            system_prompt: req.system_prompt,
            default_model_id: None,
            tags: vec![],
            capabilities,
            initial_files: vec![],
            tools: vec![],
        };

        let internal_caller = everruns_core::Caller::internal(req.org_id);
        let agent = self
            .agent_service
            .create(&internal_caller, None, create_req)
            .await
            .map_err(|e| Status::internal(format!("Failed to create agent: {}", e)))?;

        Ok(Response::new(PlatformCreateAgentResponse {
            agent: Some(schema_agent_to_proto(&agent)),
        }))
    }

    async fn platform_update_agent(
        &self,
        request: Request<PlatformUpdateAgentRequest>,
    ) -> Result<Response<PlatformUpdateAgentResponse>, Status> {
        let req = request.into_inner();
        let agent_id = parse_uuid(req.agent_id.as_ref())?;

        // Resolve UUID to public_id for the service
        let internal_caller = everruns_core::Caller::internal(req.org_id);
        let agent = self
            .agent_service
            .get(&internal_caller, agent_id)
            .await
            .map_err(|e| Status::internal(format!("Failed to get agent: {}", e)))?
            .ok_or_else(|| Status::not_found("Agent not found"))?;

        let update_req = crate::api::agents::UpdateAgentRequest {
            name: req.name,
            description: req.description,
            system_prompt: req.system_prompt,
            default_model_id: None,
            tags: None,
            capabilities: None,
            initial_files: None,
            tools: None,
            status: None,
        };

        let updated = self
            .agent_service
            .update(&internal_caller, &agent.public_id.to_string(), update_req)
            .await
            .map_err(|e| Status::internal(format!("Failed to update agent: {}", e)))?
            .ok_or_else(|| Status::not_found("Agent not found"))?;

        Ok(Response::new(PlatformUpdateAgentResponse {
            agent: Some(schema_agent_to_proto(&updated)),
        }))
    }

    async fn platform_delete_agent(
        &self,
        request: Request<PlatformDeleteAgentRequest>,
    ) -> Result<Response<PlatformDeleteAgentResponse>, Status> {
        let req = request.into_inner();
        let agent_id = parse_uuid(req.agent_id.as_ref())?;

        // Resolve UUID to public_id for the service
        let internal_caller = everruns_core::Caller::internal(req.org_id);
        let agent = self
            .agent_service
            .get(&internal_caller, agent_id)
            .await
            .map_err(|e| Status::internal(format!("Failed to get agent: {}", e)))?
            .ok_or_else(|| Status::not_found("Agent not found"))?;

        self.agent_service
            .delete(&internal_caller, &agent.public_id.to_string())
            .await
            .map_err(|e| Status::internal(format!("Failed to delete agent: {}", e)))?;

        Ok(Response::new(PlatformDeleteAgentResponse {}))
    }

    async fn platform_list_sessions(
        &self,
        request: Request<PlatformListSessionsRequest>,
    ) -> Result<Response<PlatformListSessionsResponse>, Status> {
        let req = request.into_inner();
        let internal_caller = everruns_core::Caller::internal(req.org_id);

        let limit = req.limit.unwrap_or(20);
        let agent_id = match req.agent_id {
            Some(ref uuid_proto) => Some(parse_uuid(Some(uuid_proto))?),
            None => None,
        };

        let pagination = crate::api::common::Pagination { offset: 0, limit };

        let (sessions, _total) = self
            .session_service
            .list(&internal_caller, agent_id, None, None, pagination)
            .await
            .map_err(|e| Status::internal(format!("Failed to list sessions: {}", e)))?;

        let proto_sessions = sessions.iter().map(schema_session_to_proto).collect();
        Ok(Response::new(PlatformListSessionsResponse {
            sessions: proto_sessions,
        }))
    }

    async fn platform_create_session(
        &self,
        request: Request<PlatformCreateSessionRequest>,
    ) -> Result<Response<PlatformCreateSessionResponse>, Status> {
        let req = request.into_inner();
        let internal_caller = everruns_core::Caller::internal(req.org_id);
        let harness_id = parse_uuid(req.harness_id.as_ref())?;

        let agent_uuid = match req.agent_id {
            Some(ref uuid_proto) => Some(parse_uuid(Some(uuid_proto))?),
            None => None,
        };

        // Look up agent public_id if agent_uuid is provided
        let agent_public_id = if let Some(aid) = agent_uuid {
            let agent = self
                .agent_service
                .get(&internal_caller, aid)
                .await
                .map_err(|e| Status::internal(format!("Failed to get agent: {}", e)))?
                .ok_or_else(|| Status::not_found("Agent not found"))?;
            Some(agent.public_id)
        } else {
            None
        };

        let create_req = crate::api::sessions::CreateSessionRequest {
            harness_id: Some(everruns_core::HarnessId::from_uuid(harness_id)),
            agent_id: agent_public_id,
            title: req.title,
            locale: req.locale,
            tags: vec![],
            model_id: None,
            capabilities: vec![],
            tools: vec![],
        };

        let session = self
            .session_service
            .create(
                &internal_caller,
                harness_id,
                agent_uuid,
                create_req.agent_id,
                create_req,
            )
            .await
            .map_err(|e| Status::internal(format!("Failed to create session: {}", e)))?;

        Ok(Response::new(PlatformCreateSessionResponse {
            session: Some(schema_session_to_proto(&session)),
        }))
    }

    async fn platform_delete_session(
        &self,
        request: Request<PlatformDeleteSessionRequest>,
    ) -> Result<Response<PlatformDeleteSessionResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let internal_caller = everruns_core::Caller::internal(req.org_id);

        self.session_service
            .delete(&internal_caller, session_id)
            .await
            .map_err(|e| Status::internal(format!("Failed to delete session: {}", e)))?;

        Ok(Response::new(PlatformDeleteSessionResponse {}))
    }

    async fn platform_send_message(
        &self,
        request: Request<PlatformSendMessageRequest>,
    ) -> Result<Response<PlatformSendMessageResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let internal_caller = everruns_core::Caller::internal(req.org_id);

        // Get session to find harness_id and agent_id
        let session = self
            .session_service
            .get(&internal_caller, session_id, None)
            .await
            .map_err(|e| Status::internal(format!("Failed to get session: {}", e)))?
            .ok_or_else(|| Status::not_found("Session not found"))?;

        // Create message event
        let message_id = uuid::Uuid::now_v7();
        let now = chrono::Utc::now();

        let core_message = everruns_core::Message {
            id: everruns_core::MessageId::from_uuid(message_id),
            role: everruns_core::MessageRole::User,
            content: vec![everruns_core::ContentPart::text(&req.content)],
            phase: None,
            thinking: None,
            thinking_signature: None,
            controls: None,
            metadata: None,
            external_actor: None,
            created_at: now,
        };

        let session_id_typed = everruns_core::SessionId::from_uuid(session_id);
        let message_id_typed = everruns_core::MessageId::from_uuid(message_id);

        // Emit input message event
        self.event_service
            .emit(everruns_core::EventRequest::new(
                session_id_typed,
                everruns_core::events::EventContext::empty(),
                everruns_core::events::InputMessageData::new(core_message),
            ))
            .await
            .map_err(|e| Status::internal(format!("Failed to emit message event: {}", e)))?;

        // Start turn workflow if runner is available
        if let Some(ref runner) = self.runner {
            let runner = runner.clone();
            let harness_id = session.harness_id;
            let agent_id = session.agent_id;
            tokio::spawn(async move {
                if let Err(e) = runner
                    .start_run(
                        req.org_id,
                        session_id_typed,
                        harness_id,
                        agent_id,
                        message_id_typed,
                    )
                    .await
                {
                    tracing::error!(
                        session_id = %session_id,
                        error = %e,
                        "Platform send_message: failed to start turn workflow"
                    );
                }
            });
        }

        Ok(Response::new(PlatformSendMessageResponse {}))
    }

    async fn platform_get_messages(
        &self,
        request: Request<PlatformGetMessagesRequest>,
    ) -> Result<Response<PlatformGetMessagesResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let limit = req.limit.unwrap_or(10) as i32;

        let events = self
            .event_service
            .list_message_events_limited(session_id, Some(limit))
            .await
            .map_err(|e| Status::internal(format!("Failed to list messages: {}", e)))?;

        let proto_messages: Vec<proto::PlatformMessage> = events
            .iter()
            .filter_map(|ev| {
                let (role, content) = match &ev.data {
                    everruns_core::EventData::InputMessage(d) => {
                        let text: String = d
                            .message
                            .content
                            .iter()
                            .filter_map(|p| match p {
                                everruns_core::ContentPart::Text(t) => Some(t.text.as_str()),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        ("user".to_string(), text)
                    }
                    everruns_core::EventData::OutputMessageCompleted(d) => {
                        let text: String = d
                            .message
                            .content
                            .iter()
                            .filter_map(|p| match p {
                                everruns_core::ContentPart::Text(t) => Some(t.text.as_str()),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        ("assistant".to_string(), text)
                    }
                    _ => return None,
                };

                if content.is_empty() {
                    return None;
                }

                Some(proto::PlatformMessage {
                    role,
                    content,
                    created_at: Some(ip_datetime_to_proto_timestamp(ev.ts)),
                })
            })
            .collect();

        Ok(Response::new(PlatformGetMessagesResponse {
            messages: proto_messages,
        }))
    }

    async fn platform_wait_for_idle(
        &self,
        request: Request<PlatformWaitForIdleRequest>,
    ) -> Result<Response<PlatformWaitForIdleResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let internal_caller = everruns_core::Caller::internal(req.org_id);
        let timeout_secs = req.timeout_secs.unwrap_or(120);

        let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(timeout_secs);

        loop {
            let session = self
                .session_service
                .get(&internal_caller, session_id, None)
                .await
                .map_err(|e| Status::internal(format!("Failed to get session: {}", e)))?
                .ok_or_else(|| Status::not_found("Session not found"))?;

            let status_str = session.status.to_string();
            if status_str == "idle"
                || status_str == "waiting_for_tool_results"
                || status_str == "error"
            {
                return Ok(Response::new(PlatformWaitForIdleResponse {
                    status: status_str,
                }));
            }

            if tokio::time::Instant::now() >= deadline {
                return Ok(Response::new(PlatformWaitForIdleResponse {
                    status: "timeout".to_string(),
                }));
            }

            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        }
    }

    async fn platform_list_capabilities(
        &self,
        request: Request<PlatformListCapabilitiesRequest>,
    ) -> Result<Response<PlatformListCapabilitiesResponse>, Status> {
        let req = request.into_inner();
        let capabilities = self
            .capability_service
            .list_all(req.org_id)
            .await
            .map_err(|e| Status::internal(format!("Failed to list capabilities: {}", e)))?;

        // Apply search filter if provided
        let filtered: Vec<_> = if let Some(ref q) = req.search {
            capabilities
                .into_iter()
                .filter(|c| c.matches_search(q))
                .collect()
        } else {
            capabilities
        };

        let proto_caps = filtered
            .iter()
            .map(|c| PlatformCapabilityInfo {
                id: c.id.as_str().to_string(),
                name: c.name.clone(),
                description: c.description.clone(),
                status: c.status.to_string(),
                category: c.category.clone(),
                icon: c.icon.clone(),
                is_mcp: c.is_mcp,
                is_skill: c.is_skill,
                tool_count: c.tool_definitions.len() as u32,
                tool_names: c
                    .tool_definitions
                    .iter()
                    .map(|t| t.name().to_string())
                    .collect(),
                dependencies: c.dependencies.clone(),
            })
            .collect();

        Ok(Response::new(PlatformListCapabilitiesResponse {
            capabilities: proto_caps,
        }))
    }

    async fn platform_get_base_url(
        &self,
        _request: Request<PlatformGetBaseUrlRequest>,
    ) -> Result<Response<PlatformGetBaseUrlResponse>, Status> {
        let base_url = std::env::var("PUBLIC_APP_URL")
            .or_else(|_| std::env::var("APP_URL"))
            .unwrap_or_else(|_| "http://localhost:9300".to_string());

        Ok(Response::new(PlatformGetBaseUrlResponse { base_url }))
    }
}

// Helper functions for status conversion

fn circuit_state_to_proto(state: CircuitState) -> ProtoCircuitBreakerState {
    match state {
        CircuitState::Closed => ProtoCircuitBreakerState::Closed,
        CircuitState::Open => ProtoCircuitBreakerState::Open,
        CircuitState::HalfOpen => ProtoCircuitBreakerState::HalfOpen,
    }
}

fn workflow_status_to_proto(status: WorkflowStatus) -> DurableWorkflowStatus {
    match status {
        WorkflowStatus::Pending => DurableWorkflowStatus::Pending,
        WorkflowStatus::Running => DurableWorkflowStatus::Running,
        WorkflowStatus::Completed => DurableWorkflowStatus::Completed,
        WorkflowStatus::Failed => DurableWorkflowStatus::Failed,
        WorkflowStatus::Cancelled => DurableWorkflowStatus::Cancelled,
        WorkflowStatus::ContinuedAsNew => DurableWorkflowStatus::ContinuedAsNew,
    }
}

fn proto_to_workflow_status(status: DurableWorkflowStatus) -> WorkflowStatus {
    match status {
        DurableWorkflowStatus::Pending => WorkflowStatus::Pending,
        DurableWorkflowStatus::Running => WorkflowStatus::Running,
        DurableWorkflowStatus::Completed => WorkflowStatus::Completed,
        DurableWorkflowStatus::Failed => WorkflowStatus::Failed,
        DurableWorkflowStatus::Cancelled => WorkflowStatus::Cancelled,
        DurableWorkflowStatus::ContinuedAsNew => WorkflowStatus::ContinuedAsNew,
        DurableWorkflowStatus::Unspecified => WorkflowStatus::Pending,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tonic::service::Interceptor;

    // Env-var-mutating tests must not run in parallel.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn test_interceptor_allows_when_no_token_configured() {
        let mut interceptor = GrpcAuthInterceptor::new(None);
        let request = Request::new(());
        assert!(interceptor.call(request).is_ok());
    }

    #[test]
    fn test_interceptor_allows_valid_bearer_token() {
        let mut interceptor = GrpcAuthInterceptor::new(Some("secret123".to_string()));
        let mut request = Request::new(());
        request
            .metadata_mut()
            .insert("authorization", "Bearer secret123".parse().unwrap());
        assert!(interceptor.call(request).is_ok());
    }

    #[test]
    fn test_interceptor_rejects_missing_token() {
        let mut interceptor = GrpcAuthInterceptor::new(Some("secret123".to_string()));
        let request = Request::new(());
        let err = interceptor.call(request).unwrap_err();
        assert_eq!(err.code(), tonic::Code::Unauthenticated);
        assert!(err.message().contains("Missing"));
    }

    #[test]
    fn test_interceptor_rejects_wrong_token() {
        let mut interceptor = GrpcAuthInterceptor::new(Some("secret123".to_string()));
        let mut request = Request::new(());
        request
            .metadata_mut()
            .insert("authorization", "Bearer wrong_token".parse().unwrap());
        let err = interceptor.call(request).unwrap_err();
        assert_eq!(err.code(), tonic::Code::Unauthenticated);
        assert!(err.message().contains("Invalid"));
    }

    #[test]
    fn test_interceptor_rejects_non_bearer_scheme() {
        let mut interceptor = GrpcAuthInterceptor::new(Some("secret123".to_string()));
        let mut request = Request::new(());
        request
            .metadata_mut()
            .insert("authorization", "Basic secret123".parse().unwrap());
        let err = interceptor.call(request).unwrap_err();
        assert_eq!(err.code(), tonic::Code::Unauthenticated);
    }

    /// Acquire env lock, tolerating poison from #[should_panic] tests.
    fn lock_env() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    // TM-DURABLE-002: require_grpc_auth_token panics when WORKER_GRPC_AUTH_TOKEN is unset
    #[test]
    #[should_panic(expected = "WORKER_GRPC_AUTH_TOKEN must be set")]
    fn test_require_grpc_auth_token_panics_without_env() {
        let _lock = lock_env();
        unsafe { std::env::remove_var("WORKER_GRPC_AUTH_TOKEN") };
        require_grpc_auth_token();
    }

    #[test]
    fn test_require_grpc_auth_token_returns_value() {
        let _lock = lock_env();
        unsafe { std::env::set_var("WORKER_GRPC_AUTH_TOKEN", "test-token-123") };
        let token = require_grpc_auth_token();
        assert_eq!(token, "test-token-123");
        unsafe { std::env::remove_var("WORKER_GRPC_AUTH_TOKEN") };
    }

    #[test]
    fn test_grpc_server_tls_returns_none_when_no_env_vars() {
        let _lock = lock_env();
        unsafe {
            std::env::remove_var("WORKER_GRPC_TLS_CERT");
            std::env::remove_var("WORKER_GRPC_TLS_KEY");
            std::env::remove_var("WORKER_GRPC_TLS_CA_CERT");
        }
        let config = grpc_server_tls_from_env();
        assert!(
            config.is_none(),
            "Should return None when TLS not configured"
        );
    }

    #[test]
    fn test_grpc_server_tls_returns_none_when_cert_empty() {
        let _lock = lock_env();
        unsafe {
            std::env::set_var("WORKER_GRPC_TLS_CERT", "");
            std::env::set_var("WORKER_GRPC_TLS_KEY", "");
        }
        let config = grpc_server_tls_from_env();
        assert!(config.is_none());
        unsafe {
            std::env::remove_var("WORKER_GRPC_TLS_CERT");
            std::env::remove_var("WORKER_GRPC_TLS_KEY");
        }
    }

    #[test]
    fn test_grpc_server_tls_returns_config_with_valid_certs() {
        let _lock = lock_env();
        let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        let cert_path = format!("{}/tests/fixtures/test-server-cert.pem", manifest);
        let key_path = format!("{}/tests/fixtures/test-server-key.pem", manifest);

        unsafe {
            std::env::set_var("WORKER_GRPC_TLS_CERT", &cert_path);
            std::env::set_var("WORKER_GRPC_TLS_KEY", &key_path);
            std::env::remove_var("WORKER_GRPC_TLS_CA_CERT");
        }

        let config = grpc_server_tls_from_env();
        assert!(
            config.is_some(),
            "Should return Some when cert+key are configured"
        );

        unsafe {
            std::env::remove_var("WORKER_GRPC_TLS_CERT");
            std::env::remove_var("WORKER_GRPC_TLS_KEY");
        }
    }

    #[test]
    fn test_grpc_server_tls_with_client_ca() {
        let _lock = lock_env();
        let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        let cert_path = format!("{}/tests/fixtures/test-server-cert.pem", manifest);
        let key_path = format!("{}/tests/fixtures/test-server-key.pem", manifest);
        let ca_path = format!("{}/tests/fixtures/test-ca.pem", manifest);

        unsafe {
            std::env::set_var("WORKER_GRPC_TLS_CERT", &cert_path);
            std::env::set_var("WORKER_GRPC_TLS_KEY", &key_path);
            std::env::set_var("WORKER_GRPC_TLS_CA_CERT", &ca_path);
        }

        let config = grpc_server_tls_from_env();
        assert!(
            config.is_some(),
            "Should return Some when cert+key+ca are configured"
        );

        unsafe {
            std::env::remove_var("WORKER_GRPC_TLS_CERT");
            std::env::remove_var("WORKER_GRPC_TLS_KEY");
            std::env::remove_var("WORKER_GRPC_TLS_CA_CERT");
        }
    }

    #[test]
    #[should_panic(expected = "Failed to read WORKER_GRPC_TLS_CERT")]
    fn test_grpc_server_tls_panics_on_missing_cert_file() {
        let _lock = lock_env();
        unsafe {
            std::env::set_var("WORKER_GRPC_TLS_CERT", "/nonexistent/cert.pem");
            std::env::set_var("WORKER_GRPC_TLS_KEY", "/nonexistent/key.pem");
        }
        let _config = grpc_server_tls_from_env();
        // cleanup won't run due to panic, but that's fine for test
    }

    // ========================================================================
    // Session SQL database helper tests
    // ========================================================================

    #[test]
    fn test_sqldb_error_to_status_maps_not_found() {
        use everruns_core::session_sqldb::SessionSqlDbError;
        let err = SessionSqlDbError::DatabaseNotFound("test_db".into());
        let status = sqldb_error_to_status(err);
        assert_eq!(status.code(), tonic::Code::NotFound);
    }

    #[test]
    fn test_sqldb_error_to_status_maps_already_exists() {
        use everruns_core::session_sqldb::SessionSqlDbError;
        let err = SessionSqlDbError::DatabaseAlreadyExists("test_db".into());
        let status = sqldb_error_to_status(err);
        assert_eq!(status.code(), tonic::Code::AlreadyExists);
    }

    #[test]
    fn test_sqldb_error_to_status_maps_invalid_name() {
        use everruns_core::session_sqldb::SessionSqlDbError;
        let err = SessionSqlDbError::InvalidDatabaseName("bad!name".into());
        let status = sqldb_error_to_status(err);
        assert_eq!(status.code(), tonic::Code::InvalidArgument);
    }

    #[test]
    fn test_sqldb_error_to_status_maps_limit_exceeded() {
        use everruns_core::session_sqldb::SessionSqlDbError;
        let err = SessionSqlDbError::LimitExceeded("max 10".into());
        let status = sqldb_error_to_status(err);
        assert_eq!(status.code(), tonic::Code::ResourceExhausted);
    }

    #[test]
    fn test_sqldb_error_to_status_maps_query_error() {
        use everruns_core::session_sqldb::SessionSqlDbError;
        let err = SessionSqlDbError::QueryError("syntax error".into());
        let status = sqldb_error_to_status(err);
        assert_eq!(status.code(), tonic::Code::FailedPrecondition);
    }

    #[test]
    fn test_sqldb_error_to_status_maps_timeout() {
        use everruns_core::session_sqldb::SessionSqlDbError;
        let err = SessionSqlDbError::QueryTimeout(30);
        let status = sqldb_error_to_status(err);
        assert_eq!(status.code(), tonic::Code::DeadlineExceeded);
    }

    #[test]
    fn test_sqldb_error_to_status_maps_authorizer_blocked() {
        use everruns_core::session_sqldb::SessionSqlDbError;
        let err = SessionSqlDbError::AuthorizerBlocked("DROP TABLE".into());
        let status = sqldb_error_to_status(err);
        assert_eq!(status.code(), tonic::Code::PermissionDenied);
    }

    #[test]
    fn test_sqldb_error_to_status_maps_result_too_large() {
        use everruns_core::session_sqldb::SessionSqlDbError;
        let err = SessionSqlDbError::ResultTooLarge("1MB limit".into());
        let status = sqldb_error_to_status(err);
        assert_eq!(status.code(), tonic::Code::FailedPrecondition);
    }

    #[test]
    fn test_sqldb_error_to_status_maps_internal() {
        use everruns_core::session_sqldb::SessionSqlDbError;
        let err = SessionSqlDbError::Internal("unexpected".into());
        let status = sqldb_error_to_status(err);
        assert_eq!(status.code(), tonic::Code::Internal);
    }

    #[test]
    fn test_json_value_to_proto_null() {
        let val = json_value_to_proto(serde_json::Value::Null);
        assert!(matches!(
            val.kind,
            Some(prost_types::value::Kind::NullValue(0))
        ));
    }

    #[test]
    fn test_json_value_to_proto_string() {
        let val = json_value_to_proto(serde_json::Value::String("hello".into()));
        assert!(
            matches!(val.kind, Some(prost_types::value::Kind::StringValue(ref s)) if s == "hello")
        );
    }

    #[test]
    fn test_json_value_to_proto_number() {
        let val = json_value_to_proto(serde_json::json!(42.0));
        assert!(
            matches!(val.kind, Some(prost_types::value::Kind::NumberValue(n)) if (n - 42.0).abs() < f64::EPSILON)
        );
    }

    #[test]
    fn test_json_value_to_proto_bool() {
        let val = json_value_to_proto(serde_json::Value::Bool(true));
        assert!(matches!(
            val.kind,
            Some(prost_types::value::Kind::BoolValue(true))
        ));
    }

    #[test]
    fn test_json_value_to_proto_array() {
        let val = json_value_to_proto(serde_json::json!([1, "two", null]));
        match val.kind {
            Some(prost_types::value::Kind::ListValue(list)) => {
                assert_eq!(list.values.len(), 3);
                assert!(matches!(
                    list.values[0].kind,
                    Some(prost_types::value::Kind::NumberValue(_))
                ));
                assert!(matches!(
                    list.values[1].kind,
                    Some(prost_types::value::Kind::StringValue(_))
                ));
                assert!(matches!(
                    list.values[2].kind,
                    Some(prost_types::value::Kind::NullValue(_))
                ));
            }
            _ => panic!("Expected ListValue"),
        }
    }

    #[test]
    fn test_json_value_to_proto_object() {
        let val = json_value_to_proto(serde_json::json!({"key": "value", "num": 42}));
        match val.kind {
            Some(prost_types::value::Kind::StructValue(s)) => {
                assert_eq!(s.fields.len(), 2);
                assert!(s.fields.contains_key("key"));
                assert!(s.fields.contains_key("num"));
            }
            _ => panic!("Expected StructValue"),
        }
    }

    #[test]
    fn test_db_info_to_proto_roundtrip() {
        use chrono::Utc;
        let now = Utc::now();
        let info = everruns_core::session_sqldb::DatabaseInfo {
            name: "test_db".into(),
            size_bytes: 4096,
            page_count: 1,
            created_at: now,
            updated_at: now,
        };
        let proto = db_info_to_proto(info);
        assert_eq!(proto.name, "test_db");
        assert_eq!(proto.size_bytes, 4096);
        assert_eq!(proto.page_count, 1);
        assert!(proto.created_at.is_some());
        assert!(proto.updated_at.is_some());
    }
}

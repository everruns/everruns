// Internal gRPC Service for Worker Communication
//
// Decision: Workers communicate with control plane via gRPC for all database operations
// Decision: This provides a clean boundary and simplifies worker deployment
// Decision: gRPC service uses the same services layer as HTTP API for consistency
// Decision: No direct database access - all operations go through services layer

use crate::services::{
    AgentService, EventService, HarnessService, LlmResolverService, McpServerService,
    SessionFileService, SessionService,
    session_file::{CreateDirectoryInput, CreateFileInput, GrepInput, UpdateFileInput},
};
use crate::storage::{EncryptionService, StorageBackend};
use crate::task_notifications::TaskNotificationBroadcaster;
use base64::Engine;
use everruns_durable::{
    ActivityOptions, CircuitBreakerConfig, CircuitState, DistributedCircuitBreaker,
    PostgresWorkflowEventStore, StoreError, TaskDefinition, TaskFailureOutcome, WorkerInfo,
    WorkflowError, WorkflowEvent, WorkflowEventStore, WorkflowStatus, append_event,
    record_activity_completed, record_activity_failed, record_activity_started,
    record_workflow_cancelled, record_workflow_completed, record_workflow_failed,
};
use everruns_internal_protocol::proto::{
    self, AddMessageRequest, AddMessageResponse, CheckCircuitBreakerRequest,
    CheckCircuitBreakerResponse, CircuitBreakerState as ProtoCircuitBreakerState,
    ClaimDurableTasksRequest, ClaimDurableTasksResponse, CommitExecRequest, CommitExecResponse,
    CompleteDurableTaskRequest, CompleteDurableTaskResponse, CountActiveDurableWorkflowsRequest,
    CountActiveDurableWorkflowsResponse, CreateDurableWorkflowRequest,
    CreateDurableWorkflowResponse, DeregisterDurableWorkerRequest, DeregisterDurableWorkerResponse,
    DurableWorkflowStatus, EmitEventRequest, EmitEventResponse, EmitEventStreamResponse,
    EnqueueDurableTaskRequest, EnqueueDurableTaskResponse, FailDurableTaskRequest,
    FailDurableTaskResponse, GetAgentRequest, GetAgentResponse, GetDefaultModelRequest,
    GetDefaultModelResponse, GetDurableWorkflowStatusRequest, GetDurableWorkflowStatusResponse,
    GetHarnessRequest, GetHarnessResponse, GetMcpServerByPrefixRequest,
    GetMcpServerByPrefixResponse, GetModelWithProviderRequest, GetModelWithProviderResponse,
    GetSessionRequest, GetSessionResponse, GetTurnContextRequest, GetTurnContextResponse,
    HeartbeatDurableTaskRequest, HeartbeatDurableTaskResponse, HeartbeatDurableWorkerRequest,
    HeartbeatDurableWorkerResponse, LoadMessagesRequest, LoadMessagesResponse, McpServerInfo,
    McpToolDef, RecordCircuitBreakerFailureRequest, RecordCircuitBreakerFailureResponse,
    RecordCircuitBreakerSuccessRequest, RecordCircuitBreakerSuccessResponse,
    RegisterDurableWorkerRequest, RegisterDurableWorkerResponse, ResolveImageRequest,
    ResolveImageResponse, ResolveImagesRequest, ResolveImagesResponse, ResolvedImageData,
    SessionCreateDirectoryRequest, SessionCreateDirectoryResponse, SessionDeleteFileRequest,
    SessionDeleteFileResponse, SessionGrepFilesRequest, SessionGrepFilesResponse,
    SessionListDirectoryRequest, SessionListDirectoryResponse, SessionReadFileRequest,
    SessionReadFileResponse, SessionStatFileRequest, SessionStatFileResponse,
    SessionWriteFileRequest, SessionWriteFileResponse, SetSessionStatusRequest,
    SetSessionStatusResponse, SubscribeTaskNotificationsRequest, TaskNotification,
    TaskNotificationType, UpdateDurableWorkflowStatusRequest, UpdateDurableWorkflowStatusResponse,
};
use everruns_internal_protocol::{
    WorkerService, WorkerServiceServer, proto_event_request_to_schema, schema_agent_to_proto,
    schema_event_to_proto, schema_harness_to_proto,
};
use std::pin::Pin;
use std::sync::Arc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status, Streaming};

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
    durable_store: Option<Arc<PostgresWorkflowEventStore>>,
    /// Task notification broadcaster for push-based notifications
    task_broadcaster: Option<Arc<TaskNotificationBroadcaster>>,
    /// Storage backend for image resolution
    db: Arc<StorageBackend>,
}

impl WorkerServiceImpl {
    pub fn new(
        event_service: EventService,
        db: Arc<StorageBackend>,
        encryption: Option<Arc<EncryptionService>>,
    ) -> Self {
        let agent_service = AgentService::new(db.clone());
        let harness_service = HarnessService::new(db.clone());
        let session_service = SessionService::new(db.clone());
        let session_file_service = SessionFileService::new(db.clone());
        let llm_resolver_service = LlmResolverService::new(db.clone(), encryption.clone());
        let mcp_server_service = McpServerService::new(db.clone(), encryption);

        // Create durable store using the pool if available (PostgreSQL mode only)
        // In dev mode (in-memory), durable execution is handled differently
        let durable_store = db
            .pool()
            .map(|pool| Arc::new(PostgresWorkflowEventStore::new(pool.clone())));

        Self {
            event_service,
            agent_service,
            harness_service,
            session_service,
            session_file_service,
            llm_resolver_service,
            mcp_server_service,
            durable_store,
            task_broadcaster: None, // Set via set_task_broadcaster() after async initialization
            db,
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

    /// Get organization public_id from org_id
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
    /// fetches cached tools for each server, and converts to proto McpToolDef.
    async fn build_mcp_tool_definitions(&self, agent: &everruns_core::Agent) -> Vec<McpToolDef> {
        use everruns_core::capabilities::mcp::parse_mcp_capability_id;
        use everruns_core::mcp_server::mcp_tool_name;

        let mut mcp_tools = Vec::new();

        for cap in &agent.capabilities {
            let cap_id = cap.capability_id();

            // Parse MCP server UUID from capability ID
            let server_id = match parse_mcp_capability_id(cap_id) {
                Some(id) => id,
                None => continue, // Not an MCP capability
            };

            // Fetch MCP server tools (using cache)
            let tools = match self.mcp_server_service.get_tools(server_id, false).await {
                Ok(t) => t,
                Err(e) => {
                    tracing::warn!(
                        server_id = %server_id,
                        error = %e,
                        "Failed to get MCP server tools, skipping"
                    );
                    continue;
                }
            };

            // Get server name for tool prefixing
            let server_name = match self.mcp_server_service.get(server_id).await {
                Ok(Some(s)) => s.name,
                _ => {
                    tracing::warn!(server_id = %server_id, "MCP server not found, skipping");
                    continue;
                }
            };

            // Convert each MCP tool to proto McpToolDef
            for tool in tools {
                let prefixed_name = mcp_tool_name(&server_name, &tool.name);
                let description = tool
                    .description
                    .unwrap_or_else(|| format!("Tool from MCP server: {}", server_name));
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
        let org_public_id = self.get_org_public_id(req.org_id).await?;

        // Get session via SessionService
        let session = self
            .session_service
            .get(req.org_id, &org_public_id, session_id)
            .await
            .map_err(|e| {
                tracing::error!("Failed to get session: {}", e);
                Status::internal("Failed to get session")
            })?
            .ok_or_else(|| Status::not_found("Session not found"))?;

        // Get agent with capabilities via AgentService (optional)
        let agent = if let Some(agent_id) = session.agent_id {
            self.agent_service
                .get(req.org_id, agent_id.uuid())
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
            .get(req.org_id, session.harness_id.uuid())
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
        };

        // Load messages from events using EventService
        let events = self
            .event_service
            .list_message_events(session_id)
            .await
            .map_err(|e| {
                tracing::error!("Failed to list messages: {}", e);
                Status::internal("Failed to list messages")
            })?;

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

            proto_messages.push(proto::Message {
                id: Some(uuid_to_proto_uuid(message.id.uuid())),
                role: message.role.to_string(),
                content,
                controls,
                metadata,
                created_at: Some(datetime_to_proto_timestamp(message.created_at)),
                thinking: message.thinking.clone(),
                thinking_signature: message.thinking_signature.clone(),
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
                .resolve_model(mid.uuid())
                .await
                .map_err(|e| {
                    tracing::error!("Failed to resolve model: {}", e);
                    Status::internal("Failed to resolve model")
                })?
                .map(Self::resolved_model_to_proto)
        } else {
            // Try to get the default model
            self.llm_resolver_service
                .resolve_default_model()
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
            self.build_mcp_tool_definitions(a).await
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
        let agent = self
            .agent_service
            .get(req.org_id, agent_id)
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
            .get(req.org_id, harness_id)
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
        let org_public_id = self.get_org_public_id(req.org_id).await?;

        // Get session via SessionService
        let session = self
            .session_service
            .get(req.org_id, &org_public_id, session_id)
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
        let org_public_id = self.get_org_public_id(req.org_id).await?;

        // Validate status value
        let valid_statuses = ["started", "active", "idle"];
        if !valid_statuses.contains(&req.status.as_str()) {
            return Err(Status::invalid_argument(format!(
                "Invalid status '{}'. Must be one of: started, active, idle",
                req.status
            )));
        }

        let session = self
            .session_service
            .update_status(req.org_id, &org_public_id, session_id, req.status)
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
        };

        Ok(Response::new(SetSessionStatusResponse {
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

            proto_messages.push(proto::Message {
                id: Some(uuid_to_proto_uuid(message.id.uuid())),
                role: message.role.to_string(),
                content,
                controls,
                metadata,
                created_at: Some(datetime_to_proto_timestamp(message.created_at)),
                thinking: message.thinking.clone(),
                thinking_signature: message.thinking_signature.clone(),
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
            thinking: None,
            thinking_signature: None,
            controls,
            metadata,
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

        let proto_message = proto::Message {
            id: Some(uuid_to_proto_uuid(message.id.uuid())),
            role: message.role.to_string(),
            content,
            controls,
            metadata,
            created_at: Some(datetime_to_proto_timestamp(message.created_at)),
            thinking: message.thinking.clone(),
            thinking_signature: message.thinking_signature.clone(),
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
            .resolve_model(model_id)
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
        _request: Request<GetDefaultModelRequest>,
    ) -> Result<Response<GetDefaultModelResponse>, Status> {
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
            .resolve_default_model()
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
                    tracing::error!("Failed to create file: {}", e);
                    Status::internal("Failed to create file")
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
                tracing::error!("Failed to list directory: {}", e);
                Status::internal("Failed to list directory")
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
            workflow_id,
            activity_id: task_def.activity_id.clone(),
            activity_type: task_def.activity_type.clone(),
            input: input.clone(),
            options: options.clone(),
        };

        let task_id = store.enqueue_task(task).await.map_err(|e| {
            tracing::error!("Failed to enqueue task: {}", e);
            Status::internal("Failed to enqueue task")
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
                workflow_id: Some(uuid_to_proto_uuid(t.workflow_id)),
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

        // Get image from storage
        let image_row = match self.db.get_image(image_id).await {
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

            // Get image from storage
            match self.db.get_image(image_id).await {
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

        // List active MCP servers and find one matching the prefix
        let servers = self
            .mcp_server_service
            .list_active_with_tools()
            .await
            .map_err(|e| {
                tracing::error!("Failed to list MCP servers: {}", e);
                Status::internal("Failed to list MCP servers")
            })?;

        // Find server matching the prefix (sanitized server name)
        let server_prefix_lower = req.server_prefix.to_lowercase();
        let matching_server = servers.into_iter().find(|s| {
            let sanitized_name = s
                .server
                .name
                .to_lowercase()
                .chars()
                .map(|c| if c.is_alphanumeric() { c } else { '_' })
                .collect::<String>();
            sanitized_name == server_prefix_lower
        });

        let server_info = if let Some(server_with_tools) = matching_server {
            // Get API key if set (already decrypted in the service)
            let api_key = if server_with_tools.server.api_key_set {
                // We need to fetch the full server info with decrypted API key
                // For now, we don't have direct access to the decrypted key in McpServerWithTools
                // This would need enhancement in the MCP service to include decrypted key
                None // TODO: Return decrypted API key when available
            } else {
                None
            };

            Some(McpServerInfo {
                id: Some(proto::Uuid {
                    value: server_with_tools.server.id.to_string(),
                }),
                name: server_with_tools.server.name,
                url: server_with_tools.server.url,
                api_key,
                headers: server_with_tools.server.headers,
            })
        } else {
            None
        };

        Ok(Response::new(GetMcpServerByPrefixResponse {
            server: server_info,
        }))
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
    }
}

fn proto_to_workflow_status(status: DurableWorkflowStatus) -> WorkflowStatus {
    match status {
        DurableWorkflowStatus::Pending => WorkflowStatus::Pending,
        DurableWorkflowStatus::Running => WorkflowStatus::Running,
        DurableWorkflowStatus::Completed => WorkflowStatus::Completed,
        DurableWorkflowStatus::Failed => WorkflowStatus::Failed,
        DurableWorkflowStatus::Cancelled => WorkflowStatus::Cancelled,
        DurableWorkflowStatus::Unspecified => WorkflowStatus::Pending,
    }
}

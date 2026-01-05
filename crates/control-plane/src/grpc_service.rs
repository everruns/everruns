// Internal gRPC Service for Worker Communication
//
// Decision: Workers communicate with control plane via gRPC for all database operations
// Decision: This provides a clean boundary and simplifies worker deployment
// Decision: gRPC service uses the same services layer as HTTP API for consistency
// Decision: No direct database access - all operations go through services layer

use crate::services::{
    session_file::{CreateDirectoryInput, CreateFileInput, GrepInput, UpdateFileInput},
    AgentService, EventService, LlmResolverService, SessionFileService, SessionService,
};
use crate::storage::{Database, EncryptionService};
use everruns_internal_protocol::proto::{
    self, AddMessageRequest, AddMessageResponse, CommitExecRequest, CommitExecResponse,
    EmitEventRequest, EmitEventResponse, EmitEventStreamResponse, GetAgentRequest,
    GetAgentResponse, GetDefaultModelRequest, GetDefaultModelResponse, GetModelWithProviderRequest,
    GetModelWithProviderResponse, GetSessionRequest, GetSessionResponse, GetTurnContextRequest,
    GetTurnContextResponse, LoadMessagesRequest, LoadMessagesResponse,
    SessionCreateDirectoryRequest, SessionCreateDirectoryResponse, SessionDeleteFileRequest,
    SessionDeleteFileResponse, SessionGrepFilesRequest, SessionGrepFilesResponse,
    SessionListDirectoryRequest, SessionListDirectoryResponse, SessionReadFileRequest,
    SessionReadFileResponse, SessionStatFileRequest, SessionStatFileResponse,
    SessionWriteFileRequest, SessionWriteFileResponse,
};
use everruns_internal_protocol::{
    proto_event_request_to_schema, schema_agent_to_proto, schema_event_to_proto, WorkerService,
    WorkerServiceServer,
};
use std::sync::Arc;
use tonic::{Request, Response, Status, Streaming};

/// gRPC service implementation for worker communication
///
/// This service follows the layered architecture: gRPC -> Services -> Storage
/// No direct database access is allowed - all operations go through the services layer.
pub struct WorkerServiceImpl {
    event_service: EventService,
    agent_service: AgentService,
    session_service: SessionService,
    session_file_service: SessionFileService,
    llm_resolver_service: LlmResolverService,
}

impl WorkerServiceImpl {
    pub fn new(db: Arc<Database>, encryption: Option<Arc<EncryptionService>>) -> Self {
        let event_service = EventService::new(db.clone());
        let agent_service = AgentService::new(db.clone());
        let session_service = SessionService::new(db.clone());
        let session_file_service = SessionFileService::new(db.clone());
        let llm_resolver_service = LlmResolverService::new(db, encryption);
        Self {
            event_service,
            agent_service,
            session_service,
            session_file_service,
            llm_resolver_service,
        }
    }

    /// Create a tonic server for this service
    pub fn into_server(self) -> WorkerServiceServer<Self> {
        WorkerServiceServer::new(self)
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
        EventData::MessageUser(d) => Some(d.message.clone()),
        EventData::MessageAgent(d) => Some(d.message.clone()),
        EventData::ToolCallCompleted(d) => {
            let result: Option<serde_json::Value> = d.result.as_ref().map(|parts| {
                if parts.len() == 1 {
                    if let ContentPart::Text(t) = &parts[0] {
                        return serde_json::Value::String(t.text.clone());
                    }
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

        // Get session via SessionService
        let session = self
            .session_service
            .get(session_id)
            .await
            .map_err(|e| {
                tracing::error!("Failed to get session: {}", e);
                Status::internal("Failed to get session")
            })?
            .ok_or_else(|| Status::not_found("Session not found"))?;

        // Get agent with capabilities via AgentService
        let agent = self
            .agent_service
            .get(session.agent_id)
            .await
            .map_err(|e| {
                tracing::error!("Failed to get agent: {}", e);
                Status::internal("Failed to get agent")
            })?
            .ok_or_else(|| Status::not_found("Agent not found"))?;

        // Convert to proto types
        use everruns_internal_protocol::{datetime_to_proto_timestamp, uuid_to_proto_uuid};

        let proto_agent = schema_agent_to_proto(&agent);

        let proto_session = proto::Session {
            id: Some(uuid_to_proto_uuid(session.id)),
            agent_id: Some(uuid_to_proto_uuid(session.agent_id)),
            title: session.title.clone().unwrap_or_default(),
            status: session.status.to_string(),
            created_at: Some(datetime_to_proto_timestamp(session.created_at)),
            updated_at: Some(datetime_to_proto_timestamp(session.created_at)),
            default_model_id: session.model_id.map(uuid_to_proto_uuid),
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
                id: Some(uuid_to_proto_uuid(message.id)),
                role: message.role.to_string(),
                content,
                controls,
                metadata,
                created_at: Some(datetime_to_proto_timestamp(message.created_at)),
            });
        }

        // Get model with provider (decrypted API key) via LlmResolverService
        // Priority: session model > agent model > default model
        let model_id = session.model_id.or(agent.default_model_id);

        let model: Option<proto::ModelWithProvider> = if let Some(mid) = model_id {
            self.llm_resolver_service
                .resolve_model(mid)
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

        Ok(Response::new(GetTurnContextResponse {
            agent: Some(proto_agent),
            session: Some(proto_session),
            messages: proto_messages,
            model,
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
            .get(agent_id)
            .await
            .map_err(|e| Status::internal(format!("Failed to get agent: {}", e)))?;

        let proto_agent = agent.map(|a| schema_agent_to_proto(&a));

        Ok(Response::new(GetAgentResponse { agent: proto_agent }))
    }

    async fn get_session(
        &self,
        request: Request<GetSessionRequest>,
    ) -> Result<Response<GetSessionResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;

        // Get session via SessionService
        let session = self.session_service.get(session_id).await.map_err(|e| {
            tracing::error!("Failed to get session: {}", e);
            Status::internal("Failed to get session")
        })?;

        use everruns_internal_protocol::{datetime_to_proto_timestamp, uuid_to_proto_uuid};

        let proto_session = session.map(|s| proto::Session {
            id: Some(uuid_to_proto_uuid(s.id)),
            agent_id: Some(uuid_to_proto_uuid(s.agent_id)),
            title: s.title.clone().unwrap_or_default(),
            status: s.status.to_string(),
            created_at: Some(datetime_to_proto_timestamp(s.created_at)),
            updated_at: Some(datetime_to_proto_timestamp(s.created_at)),
            default_model_id: s.model_id.map(uuid_to_proto_uuid),
        });

        Ok(Response::new(GetSessionResponse {
            session: proto_session,
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
                id: Some(uuid_to_proto_uuid(message.id)),
                role: message.role.to_string(),
                content,
                controls,
                metadata,
                created_at: Some(datetime_to_proto_timestamp(message.created_at)),
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
            ContentPart, Controls, EventContext, EventRequest, Message, MessageAgentData,
            MessageRole, MessageUserData,
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
            id: uuid::Uuid::now_v7(),
            role: role.clone(),
            content,
            controls,
            metadata,
            created_at: Utc::now(),
        };

        // Create typed event request based on role
        let event_request = match role {
            MessageRole::User => EventRequest::new(
                session_id,
                EventContext::empty(),
                MessageUserData::new(message.clone()),
            ),
            MessageRole::Assistant => EventRequest::new(
                session_id,
                EventContext::empty(),
                MessageAgentData::new(message.clone()),
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
            id: Some(uuid_to_proto_uuid(message.id)),
            role: message.role.to_string(),
            content,
            controls,
            metadata,
            created_at: Some(datetime_to_proto_timestamp(message.created_at)),
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
}

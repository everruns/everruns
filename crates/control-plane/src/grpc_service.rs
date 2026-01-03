// Internal gRPC Service for Worker Communication
//
// Decision: Workers communicate with control plane via gRPC for all database operations
// Decision: This provides a clean boundary and simplifies worker deployment
// Decision: gRPC service uses the same services layer as HTTP API for consistency

use crate::services::EventService;
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
    proto_event_request_to_schema, schema_event_to_proto, WorkerService, WorkerServiceServer,
};
use std::sync::Arc;
use tonic::{Request, Response, Status, Streaming};

/// gRPC service implementation for worker communication
pub struct WorkerServiceImpl {
    db: Arc<Database>,
    encryption: Option<Arc<EncryptionService>>,
    event_service: EventService,
}

impl WorkerServiceImpl {
    pub fn new(db: Arc<Database>, encryption: Option<Arc<EncryptionService>>) -> Self {
        let event_service = EventService::new(db.clone());
        Self {
            db,
            encryption,
            event_service,
        }
    }

    /// Create a tonic server for this service
    pub fn into_server(self) -> WorkerServiceServer<Self> {
        WorkerServiceServer::new(self)
    }

    /// Resolve a model by ID and return with decrypted provider API key
    async fn resolve_model_with_provider(
        &self,
        model_id: uuid::Uuid,
    ) -> Result<Option<proto::ModelWithProvider>, Status> {
        let encryption = match &self.encryption {
            Some(enc) => enc.as_ref().clone(),
            None => return Ok(None), // No encryption service, can't decrypt API keys
        };

        let model_row = self
            .db
            .get_llm_model(model_id)
            .await
            .map_err(|e| Status::internal(format!("Failed to get model: {}", e)))?;

        let model_row = match model_row {
            Some(row) => row,
            None => return Ok(None),
        };

        let provider_row = self
            .db
            .get_llm_provider(model_row.provider_id)
            .await
            .map_err(|e| Status::internal(format!("Failed to get provider: {}", e)))?;

        let provider_row = match provider_row {
            Some(row) => row,
            None => return Ok(None),
        };

        let provider_with_key = self
            .db
            .get_provider_with_api_key(&provider_row, &encryption)
            .map_err(|e| Status::internal(format!("Failed to decrypt API key: {}", e)))?;

        Ok(Some(proto::ModelWithProvider {
            model: model_row.model_id,
            provider_type: provider_with_key.provider_type,
            api_key: provider_with_key.api_key,
            base_url: provider_with_key.base_url,
        }))
    }

    /// Resolve the default model and return with decrypted provider API key
    async fn resolve_default_model(&self) -> Result<Option<proto::ModelWithProvider>, Status> {
        let encryption = match &self.encryption {
            Some(enc) => enc.as_ref().clone(),
            None => return Ok(None), // No encryption service, can't decrypt API keys
        };

        let model_row = self
            .db
            .get_default_llm_model()
            .await
            .map_err(|e| Status::internal(format!("Failed to get default model: {}", e)))?;

        let model_row = match model_row {
            Some(row) => row,
            None => return Ok(None),
        };

        let provider_row = self
            .db
            .get_llm_provider(model_row.provider_id)
            .await
            .map_err(|e| Status::internal(format!("Failed to get provider: {}", e)))?;

        let provider_row = match provider_row {
            Some(row) => row,
            None => return Ok(None),
        };

        let provider_with_key = self
            .db
            .get_provider_with_api_key(&provider_row, &encryption)
            .map_err(|e| Status::internal(format!("Failed to decrypt API key: {}", e)))?;

        Ok(Some(proto::ModelWithProvider {
            model: model_row.model_id,
            provider_type: provider_with_key.provider_type,
            api_key: provider_with_key.api_key,
            base_url: provider_with_key.base_url,
        }))
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

// Helper to convert proto timestamp to chrono DateTime
fn proto_to_datetime(ts: &proto::Timestamp) -> chrono::DateTime<chrono::Utc> {
    use chrono::TimeZone;
    chrono::Utc
        .timestamp_opt(ts.seconds, ts.nanos as u32)
        .single()
        .unwrap_or_else(chrono::Utc::now)
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

        // Get session
        let session_row = self
            .db
            .get_session(session_id)
            .await
            .map_err(|e| Status::internal(format!("Failed to get session: {}", e)))?
            .ok_or_else(|| Status::not_found("Session not found"))?;

        // Get agent
        let agent_row = self
            .db
            .get_agent(session_row.agent_id)
            .await
            .map_err(|e| Status::internal(format!("Failed to get agent: {}", e)))?
            .ok_or_else(|| Status::not_found("Agent not found"))?;

        // Convert rows to proto types
        use everruns_internal_protocol::{datetime_to_proto_timestamp, uuid_to_proto_uuid};

        let proto_agent = proto::Agent {
            id: Some(uuid_to_proto_uuid(agent_row.id)),
            name: agent_row.name.clone(),
            description: agent_row.description.clone().unwrap_or_default(),
            system_prompt: agent_row.system_prompt.clone(),
            default_model_id: agent_row.default_model_id.map(uuid_to_proto_uuid),
            temperature: None, // Not stored in AgentRow
            max_tokens: None,  // Not stored in AgentRow
            status: agent_row.status.clone(),
            created_at: Some(datetime_to_proto_timestamp(agent_row.created_at)),
            updated_at: Some(datetime_to_proto_timestamp(agent_row.updated_at)),
            capability_ids: vec![], // Capabilities stored separately, not in AgentRow
        };

        let proto_session = proto::Session {
            id: Some(uuid_to_proto_uuid(session_row.id)),
            agent_id: Some(uuid_to_proto_uuid(session_row.agent_id)),
            title: session_row.title.clone().unwrap_or_default(),
            status: session_row.status.clone(),
            created_at: Some(datetime_to_proto_timestamp(session_row.created_at)),
            // SessionRow doesn't have updated_at, use created_at as fallback
            updated_at: Some(datetime_to_proto_timestamp(session_row.created_at)),
            default_model_id: session_row.model_id.map(uuid_to_proto_uuid),
        };

        // Load messages from events table
        let events = self
            .db
            .list_message_events(session_id)
            .await
            .map_err(|e| Status::internal(format!("Failed to list messages: {}", e)))?;

        use everruns_core::{ContentPart, Event, EventData, Message};

        let mut proto_messages: Vec<proto::Message> = Vec::with_capacity(events.len());

        for event_row in events {
            // Parse the event data to get the message
            let event: Event = match serde_json::from_value(event_row.data.clone()) {
                Ok(e) => e,
                Err(e) => {
                    tracing::warn!("Failed to parse event {}: {}", event_row.id, e);
                    continue;
                }
            };

            // Extract message from typed event data
            let message = match event.data {
                EventData::MessageUser(data) => data.message,
                EventData::MessageAgent(data) => data.message,
                EventData::ToolCallCompleted(data) => {
                    // Convert tool call completion to tool result message
                    let result: Option<serde_json::Value> = data.result.map(|parts| {
                        if parts.len() == 1 {
                            if let ContentPart::Text(t) = &parts[0] {
                                return serde_json::Value::String(t.text.clone());
                            }
                        }
                        serde_json::to_value(&parts).unwrap_or_default()
                    });
                    Message::tool_result(&data.tool_call_id, result, data.error)
                }
                _ => continue,
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

        // Get model with provider (decrypted API key)
        // Priority: session model > agent model > default model
        let model_id = session_row.model_id.or(agent_row.default_model_id);

        let model: Option<proto::ModelWithProvider> = if let Some(mid) = model_id {
            self.resolve_model_with_provider(mid).await?
        } else {
            // Try to get the default model
            self.resolve_default_model().await?
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

        let agent_row = self
            .db
            .get_agent(agent_id)
            .await
            .map_err(|e| Status::internal(format!("Failed to get agent: {}", e)))?;

        use everruns_internal_protocol::{datetime_to_proto_timestamp, uuid_to_proto_uuid};

        let proto_agent = agent_row.map(|a| proto::Agent {
            id: Some(uuid_to_proto_uuid(a.id)),
            name: a.name.clone(),
            description: a.description.clone().unwrap_or_default(),
            system_prompt: a.system_prompt.clone(),
            default_model_id: a.default_model_id.map(uuid_to_proto_uuid),
            temperature: None, // Not stored in AgentRow
            max_tokens: None,  // Not stored in AgentRow
            status: a.status.clone(),
            created_at: Some(datetime_to_proto_timestamp(a.created_at)),
            updated_at: Some(datetime_to_proto_timestamp(a.updated_at)),
            capability_ids: vec![], // Capabilities stored separately
        });

        Ok(Response::new(GetAgentResponse { agent: proto_agent }))
    }

    async fn get_session(
        &self,
        request: Request<GetSessionRequest>,
    ) -> Result<Response<GetSessionResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;

        let session_row = self
            .db
            .get_session(session_id)
            .await
            .map_err(|e| Status::internal(format!("Failed to get session: {}", e)))?;

        use everruns_internal_protocol::{datetime_to_proto_timestamp, uuid_to_proto_uuid};

        let proto_session = session_row.map(|s| proto::Session {
            id: Some(uuid_to_proto_uuid(s.id)),
            agent_id: Some(uuid_to_proto_uuid(s.agent_id)),
            title: s.title.clone().unwrap_or_default(),
            status: s.status.clone(),
            created_at: Some(datetime_to_proto_timestamp(s.created_at)),
            // SessionRow doesn't have updated_at, use created_at as fallback
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

        // Query events for message-related event types
        let events = self
            .db
            .list_message_events(session_id)
            .await
            .map_err(|e| Status::internal(format!("Failed to list messages: {}", e)))?;

        use everruns_core::{ContentPart, Event, EventData, Message};
        use everruns_internal_protocol::{datetime_to_proto_timestamp, uuid_to_proto_uuid};

        let mut proto_messages: Vec<proto::Message> = Vec::with_capacity(events.len());

        for event_row in events {
            // Parse the event data to get the message
            let event: Event = match serde_json::from_value(event_row.data.clone()) {
                Ok(e) => e,
                Err(e) => {
                    tracing::warn!("Failed to parse event {}: {}", event_row.id, e);
                    continue;
                }
            };

            // Extract message from typed event data
            let message = match event.data {
                EventData::MessageUser(data) => data.message,
                EventData::MessageAgent(data) => data.message,
                EventData::ToolCallCompleted(data) => {
                    // Convert tool call completion to tool result message
                    let result: Option<serde_json::Value> = data.result.map(|parts| {
                        if parts.len() == 1 {
                            if let ContentPart::Text(t) = &parts[0] {
                                return serde_json::Value::String(t.text.clone());
                            }
                        }
                        serde_json::to_value(&parts).unwrap_or_default()
                    });
                    Message::tool_result(&data.tool_call_id, result, data.error)
                }
                _ => {
                    // Not a message event type we care about
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
        let encryption = match &self.encryption {
            Some(enc) => enc.as_ref().clone(),
            None => {
                return Err(Status::unavailable(
                    "Encryption service not configured - cannot decrypt API keys",
                ));
            }
        };

        // Look up the model
        let model_row = self
            .db
            .get_llm_model(model_id)
            .await
            .map_err(|e| Status::internal(format!("Failed to get model: {}", e)))?;

        let model_row = match model_row {
            Some(row) => row,
            None => return Ok(Response::new(GetModelWithProviderResponse { model: None })),
        };

        // Look up the provider
        let provider_row = self
            .db
            .get_llm_provider(model_row.provider_id)
            .await
            .map_err(|e| Status::internal(format!("Failed to get provider: {}", e)))?;

        let provider_row = match provider_row {
            Some(row) => row,
            None => return Ok(Response::new(GetModelWithProviderResponse { model: None })),
        };

        // Decrypt the API key
        let provider_with_key = self
            .db
            .get_provider_with_api_key(&provider_row, &encryption)
            .map_err(|e| Status::internal(format!("Failed to decrypt API key: {}", e)))?;

        Ok(Response::new(GetModelWithProviderResponse {
            model: Some(proto::ModelWithProvider {
                model: model_row.model_id,
                provider_type: provider_with_key.provider_type,
                api_key: provider_with_key.api_key,
                base_url: provider_with_key.base_url,
            }),
        }))
    }

    async fn get_default_model(
        &self,
        _request: Request<GetDefaultModelRequest>,
    ) -> Result<Response<GetDefaultModelResponse>, Status> {
        // Check if encryption service is available
        let encryption = match &self.encryption {
            Some(enc) => enc.as_ref().clone(),
            None => {
                return Err(Status::unavailable(
                    "Encryption service not configured - cannot decrypt API keys",
                ));
            }
        };

        // Look up the default model
        let model_row = self
            .db
            .get_default_llm_model()
            .await
            .map_err(|e| Status::internal(format!("Failed to get default model: {}", e)))?;

        let model_row = match model_row {
            Some(row) => row,
            None => return Ok(Response::new(GetDefaultModelResponse { model: None })),
        };

        // Look up the provider
        let provider_row = self
            .db
            .get_llm_provider(model_row.provider_id)
            .await
            .map_err(|e| Status::internal(format!("Failed to get provider: {}", e)))?;

        let provider_row = match provider_row {
            Some(row) => row,
            None => return Ok(Response::new(GetDefaultModelResponse { model: None })),
        };

        // Decrypt the API key
        let provider_with_key = self
            .db
            .get_provider_with_api_key(&provider_row, &encryption)
            .map_err(|e| Status::internal(format!("Failed to decrypt API key: {}", e)))?;

        Ok(Response::new(GetDefaultModelResponse {
            model: Some(proto::ModelWithProvider {
                model: model_row.model_id,
                provider_type: provider_with_key.provider_type,
                api_key: provider_with_key.api_key,
                base_url: provider_with_key.base_url,
            }),
        }))
    }

    // ========================================================================
    // Session file operations
    // ========================================================================

    async fn session_read_file(
        &self,
        request: Request<SessionReadFileRequest>,
    ) -> Result<Response<SessionReadFileResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;

        let file_row = self
            .db
            .get_session_file(session_id, &req.path)
            .await
            .map_err(|e| Status::internal(format!("Failed to read file: {}", e)))?;

        use everruns_internal_protocol::{datetime_to_proto_timestamp, uuid_to_proto_uuid};

        // Derive name from path
        fn name_from_path(path: &str) -> String {
            if path == "/" {
                "/".to_string()
            } else {
                path.rsplit('/').next().unwrap_or(path).to_string()
            }
        }

        let proto_file = file_row.map(|f| {
            // Convert bytes content to string
            let content = f
                .content
                .as_ref()
                .map(|bytes| String::from_utf8_lossy(bytes).to_string());

            proto::SessionFile {
                id: Some(uuid_to_proto_uuid(f.id)),
                session_id: Some(uuid_to_proto_uuid(f.session_id)),
                path: f.path.clone(),
                name: name_from_path(&f.path),
                content,
                encoding: "text".to_string(), // Default encoding
                is_directory: f.is_directory,
                is_readonly: f.is_readonly,
                size_bytes: f.size_bytes,
                created_at: Some(datetime_to_proto_timestamp(f.created_at)),
                updated_at: Some(datetime_to_proto_timestamp(f.updated_at)),
            }
        });

        Ok(Response::new(SessionReadFileResponse { file: proto_file }))
    }

    async fn session_write_file(
        &self,
        request: Request<SessionWriteFileRequest>,
    ) -> Result<Response<SessionWriteFileResponse>, Status> {
        use crate::storage::{CreateSessionFileRow, UpdateSessionFile};
        use everruns_internal_protocol::{datetime_to_proto_timestamp, uuid_to_proto_uuid};

        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;

        // Convert content to bytes
        let content = req.content.into_bytes();

        // Check if file already exists
        let existing = self
            .db
            .get_session_file(session_id, &req.path)
            .await
            .map_err(|e| Status::internal(format!("Failed to check file: {}", e)))?;

        let file_row = if let Some(_existing_file) = existing {
            // Update existing file
            let update = UpdateSessionFile {
                content: Some(content),
                is_readonly: None,
            };
            self.db
                .update_session_file(session_id, &req.path, update)
                .await
                .map_err(|e| Status::internal(format!("Failed to update file: {}", e)))?
                .ok_or_else(|| Status::internal("File disappeared during update"))?
        } else {
            // Create new file
            let create = CreateSessionFileRow {
                session_id,
                path: req.path.clone(),
                content: Some(content),
                is_directory: false,
                is_readonly: false,
            };
            self.db
                .create_session_file(create)
                .await
                .map_err(|e| Status::internal(format!("Failed to create file: {}", e)))?
        };

        // Derive name from path
        fn name_from_path(path: &str) -> String {
            if path == "/" {
                "/".to_string()
            } else {
                path.rsplit('/').next().unwrap_or(path).to_string()
            }
        }

        let content_str = file_row
            .content
            .as_ref()
            .map(|bytes| String::from_utf8_lossy(bytes).to_string());

        let proto_file = proto::SessionFile {
            id: Some(uuid_to_proto_uuid(file_row.id)),
            session_id: Some(uuid_to_proto_uuid(file_row.session_id)),
            path: file_row.path.clone(),
            name: name_from_path(&file_row.path),
            content: content_str,
            encoding: "text".to_string(),
            is_directory: file_row.is_directory,
            is_readonly: file_row.is_readonly,
            size_bytes: file_row.size_bytes,
            created_at: Some(datetime_to_proto_timestamp(file_row.created_at)),
            updated_at: Some(datetime_to_proto_timestamp(file_row.updated_at)),
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

        let deleted = if req.recursive {
            let count = self
                .db
                .delete_session_file_recursive(session_id, &req.path)
                .await
                .map_err(|e| Status::internal(format!("Failed to delete file: {}", e)))?;
            count > 0
        } else {
            self.db
                .delete_session_file(session_id, &req.path)
                .await
                .map_err(|e| Status::internal(format!("Failed to delete file: {}", e)))?
        };

        Ok(Response::new(SessionDeleteFileResponse { deleted }))
    }

    async fn session_list_directory(
        &self,
        request: Request<SessionListDirectoryRequest>,
    ) -> Result<Response<SessionListDirectoryResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;

        let files = self
            .db
            .list_session_files(session_id, &req.path)
            .await
            .map_err(|e| Status::internal(format!("Failed to list directory: {}", e)))?;

        use everruns_internal_protocol::{datetime_to_proto_timestamp, uuid_to_proto_uuid};

        // Derive name from path
        fn name_from_path(path: &str) -> String {
            if path == "/" {
                "/".to_string()
            } else {
                path.rsplit('/').next().unwrap_or(path).to_string()
            }
        }

        let proto_files: Vec<proto::FileInfo> = files
            .iter()
            .map(|f| proto::FileInfo {
                id: Some(uuid_to_proto_uuid(f.id)),
                session_id: Some(uuid_to_proto_uuid(f.session_id)),
                path: f.path.clone(),
                name: name_from_path(&f.path),
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

        // Get file info and convert to stat
        let file = self
            .db
            .get_session_file(session_id, &req.path)
            .await
            .map_err(|e| Status::internal(format!("Failed to stat file: {}", e)))?;

        use everruns_internal_protocol::datetime_to_proto_timestamp;

        // Derive name from path
        fn name_from_path(path: &str) -> String {
            if path == "/" {
                "/".to_string()
            } else {
                path.rsplit('/').next().unwrap_or(path).to_string()
            }
        }

        let proto_stat = file.map(|f| proto::FileStat {
            path: f.path.clone(),
            name: name_from_path(&f.path),
            is_directory: f.is_directory,
            is_readonly: f.is_readonly,
            size_bytes: f.size_bytes,
            created_at: Some(datetime_to_proto_timestamp(f.created_at)),
            updated_at: Some(datetime_to_proto_timestamp(f.updated_at)),
        });

        Ok(Response::new(SessionStatFileResponse { stat: proto_stat }))
    }

    async fn session_grep_files(
        &self,
        request: Request<SessionGrepFilesRequest>,
    ) -> Result<Response<SessionGrepFilesResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;

        // Compile the regex pattern
        let regex = regex::Regex::new(&req.pattern)
            .map_err(|e| Status::invalid_argument(format!("Invalid regex pattern: {}", e)))?;

        // Get all files in the session
        let files = self
            .db
            .list_all_session_files(session_id)
            .await
            .map_err(|e| Status::internal(format!("Failed to list files: {}", e)))?;

        let mut matches = Vec::new();

        for file_info in files {
            // Skip directories
            if file_info.is_directory {
                continue;
            }

            // If path_pattern is specified, filter by it
            if let Some(ref path_pattern) = req.path_pattern {
                if !file_info.path.contains(path_pattern) {
                    continue;
                }
            }

            // Read the file content
            let file = match self.db.get_session_file(session_id, &file_info.path).await {
                Ok(Some(f)) => f,
                Ok(None) => continue,
                Err(e) => {
                    tracing::warn!("Failed to read file {}: {}", file_info.path, e);
                    continue;
                }
            };

            // Get content as string
            let content = match &file.content {
                Some(bytes) => match std::str::from_utf8(bytes) {
                    Ok(s) => s,
                    Err(_) => continue, // Skip binary files
                },
                None => continue,
            };

            // Search line by line
            for (line_idx, line) in content.lines().enumerate() {
                if regex.is_match(line) {
                    matches.push(proto::GrepMatch {
                        path: file_info.path.clone(),
                        line_number: (line_idx + 1) as u64,
                        line: line.to_string(),
                    });
                }
            }
        }

        Ok(Response::new(SessionGrepFilesResponse { matches }))
    }

    async fn session_create_directory(
        &self,
        request: Request<SessionCreateDirectoryRequest>,
    ) -> Result<Response<SessionCreateDirectoryResponse>, Status> {
        use crate::storage::CreateSessionFileRow;
        use everruns_internal_protocol::{datetime_to_proto_timestamp, uuid_to_proto_uuid};

        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;

        // Check if directory already exists
        let existing = self
            .db
            .get_session_file(session_id, &req.path)
            .await
            .map_err(|e| Status::internal(format!("Failed to check directory: {}", e)))?;

        let file_row = if let Some(existing_file) = existing {
            // Directory already exists, return it
            if !existing_file.is_directory {
                return Err(Status::already_exists(
                    "A file with this path already exists",
                ));
            }
            existing_file
        } else {
            // Create new directory
            let create = CreateSessionFileRow {
                session_id,
                path: req.path.clone(),
                content: None,
                is_directory: true,
                is_readonly: false,
            };
            self.db
                .create_session_file(create)
                .await
                .map_err(|e| Status::internal(format!("Failed to create directory: {}", e)))?
        };

        // Derive name from path
        fn name_from_path(path: &str) -> String {
            if path == "/" {
                "/".to_string()
            } else {
                path.rsplit('/').next().unwrap_or(path).to_string()
            }
        }

        let proto_file_info = proto::FileInfo {
            id: Some(uuid_to_proto_uuid(file_row.id)),
            session_id: Some(uuid_to_proto_uuid(file_row.session_id)),
            path: file_row.path.clone(),
            name: name_from_path(&file_row.path),
            is_directory: file_row.is_directory,
            is_readonly: file_row.is_readonly,
            size_bytes: file_row.size_bytes,
            created_at: Some(datetime_to_proto_timestamp(file_row.created_at)),
            updated_at: Some(datetime_to_proto_timestamp(file_row.updated_at)),
        };

        Ok(Response::new(SessionCreateDirectoryResponse {
            directory: Some(proto_file_info),
        }))
    }
}

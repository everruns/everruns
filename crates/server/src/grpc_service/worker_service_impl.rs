// WorkerService trait implementation for gRPC service
// Split from grpc_service.rs for maintainability (EVE-101).

use super::*;
use crate::domains::common::CommandErrorKind;

const COMMAND_API_VERSION_V1: &str = "v1";
const MAX_EXECUTE_COMMAND_PARAMS_BYTES: usize = 1024 * 1024;
const DEFAULT_TURN_CONTEXT_MESSAGE_LIMIT: i32 = 200;
const MAX_TURN_CONTEXT_MESSAGE_LIMIT: i32 = crate::storage::repository::MESSAGE_SAFETY_LIMIT as i32;

fn flatten_secret_bindings(
    bindings: std::collections::HashMap<String, Vec<everruns_mcp::McpSecretBinding>>,
) -> Vec<proto::McpSecretBinding> {
    bindings
        .into_iter()
        .flat_map(|(tool_name, bindings)| {
            bindings
                .into_iter()
                .map(move |binding| proto::McpSecretBinding {
                    tool_name: tool_name.clone(),
                    parameter_name: binding.parameter_name,
                    value: binding.value,
                    setup_url: binding.setup_url,
                    label: binding.label,
                })
        })
        .collect()
}

fn apply_proto_secret_binding_schemas(
    definitions: &mut [McpToolDef],
    bindings: &[everruns_core::McpSecretBindingMetadata],
) {
    for binding in bindings {
        let tool_name = everruns_core::mcp_tool_name(&binding.server_name, &binding.tool_name);
        let Some(definition) = definitions
            .iter_mut()
            .find(|definition| definition.name == tool_name)
        else {
            continue;
        };
        if let Some(parameters) = definition.parameters.as_mut() {
            let mut json = everruns_internal_protocol::proto_struct_to_json(parameters);
            if let Some(object) = json.as_object_mut() {
                if let Some(properties) = object
                    .get_mut("properties")
                    .and_then(serde_json::Value::as_object_mut)
                {
                    properties.remove(&binding.parameter_name);
                }
                if let Some(required) = object
                    .get_mut("required")
                    .and_then(serde_json::Value::as_array_mut)
                {
                    required.retain(|value| value.as_str() != Some(&binding.parameter_name));
                }
            }
            *parameters = everruns_internal_protocol::json_to_proto_struct(&json);
        }
        let status = if binding.configured {
            "configured"
        } else {
            "setup required"
        };
        definition.description.push_str(&format!(
            "\n\nCredential '{}' is securely bound ({status}); do not request or supply it. Setup: {}",
            binding.parameter_name, binding.setup_url
        ));
    }
}

fn normalize_turn_context_message_limit(requested_limit: Option<i32>, default_limit: i32) -> i32 {
    let normalized_default = default_limit.clamp(1, MAX_TURN_CONTEXT_MESSAGE_LIMIT);
    requested_limit
        .filter(|&l| l > 0)
        .unwrap_or(normalized_default)
        .clamp(1, MAX_TURN_CONTEXT_MESSAGE_LIMIT)
}

fn command_error_kind(error: &crate::domains::common::CommandError) -> i32 {
    match &error.kind {
        CommandErrorKind::BadRequest(_) => 1,
        CommandErrorKind::Unprocessable(_) => 1,
        CommandErrorKind::Forbidden(_) => 2,
        CommandErrorKind::NotFound(message) if message.starts_with("Unknown command:") => 1,
        CommandErrorKind::NotFound(_) => 3,
        CommandErrorKind::Conflict(_) => 4,
        CommandErrorKind::RateLimited(_) => 1,
        CommandErrorKind::Internal(_) => 5,
    }
}

fn command_error_to_proto(error: crate::domains::common::CommandError) -> ProtoCommandError {
    let message = match &error.kind {
        CommandErrorKind::Internal(inner) => inner.to_string(),
        _ => error.to_string(),
    };

    ProtoCommandError {
        kind: command_error_kind(&error),
        message,
    }
}

fn command_error_to_status(error: crate::domains::common::CommandError) -> Status {
    match error.kind {
        CommandErrorKind::BadRequest(message) => Status::invalid_argument(message),
        CommandErrorKind::Unprocessable(message) => Status::failed_precondition(message),
        CommandErrorKind::Forbidden(message) => Status::permission_denied(message),
        CommandErrorKind::NotFound(message) => Status::not_found(message),
        CommandErrorKind::Conflict(message) => Status::failed_precondition(message),
        CommandErrorKind::RateLimited(message) => Status::resource_exhausted(message),
        CommandErrorKind::Internal(inner) => Status::internal(inner.to_string()),
    }
}

fn command_schema_hash(
    meta: &crate::domains::common::CommandMeta,
    positional_arg: Option<&str>,
) -> String {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(meta.name.as_bytes());
    hasher.update([0]);
    hasher.update(meta.category.as_bytes());
    hasher.update([0]);
    hasher.update(meta.description.as_bytes());
    hasher.update([0]);
    hasher.update(meta.method.as_bytes());
    hasher.update([0]);
    hasher.update(meta.path.as_bytes());
    hasher.update([0]);
    if let Some(positional_arg) = positional_arg {
        hasher.update(positional_arg.as_bytes());
    }
    hex::encode(hasher.finalize())
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
        let mut session = self
            .session_service
            .get(&internal_caller, session_id, None)
            .await
            .map_err(|e| {
                tracing::error!("Failed to get session: {}", e);
                Status::internal("Failed to get session")
            })?
            .ok_or_else(|| Status::not_found("Session not found"))?;

        // Fold any runtime ARD attachments (knowledge/integrations/integrations.md, resource_discovery)
        // into the session config layer before scoped MCP servers / capabilities
        // are resolved, so attached MCP servers and A2A agents become usable on
        // the next turn with no change to the agent loop.
        if let Ok(store) = self.storage_store() {
            // Attachments merge into the portable config layer (EVE-882);
            // fold the merged fields back onto the stored record so proto
            // conversion and scoped-MCP resolution below see them.
            let mut execution_session = session.execution_session();
            everruns_core::ard_attachment::apply_session_attachments(
                store.as_ref(),
                &mut execution_session,
            )
            .await;
            session.mcp_servers = execution_session.mcp_servers;
            session.capabilities = execution_session.capabilities;
        }

        // Get agent with capabilities via domain query (optional)
        let mut agent = if let Some(agent_id) = session.agent_id {
            crate::domains::agents::queries::get_by_public_id(
                &self.db,
                req.org_id,
                &agent_id.to_string(),
            )
            .await
            .map_err(|e| {
                tracing::error!("Failed to get agent: {}", e);
                Status::internal("Failed to get agent")
            })?
        } else {
            None
        };
        if let (Some(agent), Some(version_id)) = (agent.as_mut(), session.agent_version_id)
            && let Some(version_row) = self
                .db
                .get_agent_version(req.org_id, version_id)
                .await
                .map_err(|e| {
                    tracing::error!("Failed to get agent version: {}", e);
                    Status::internal("Failed to get agent version")
                })?
        {
            let version = crate::domains::agents::queries::row_to_agent_version(version_row);
            *agent = crate::domains::agents::queries::version_to_agent(agent, &version);
        }

        // Load effective harness, including inherited parent config.
        let mut harness = crate::domains::harnesses::queries::resolve_effective(
            &self.db,
            req.org_id,
            session.harness_id,
        )
        .await
        .map_err(|e| {
            tracing::error!("Failed to get harness: {}", e);
            Status::internal("Failed to get harness")
        })?;

        // Persisted configs may predate an org disabling a feature. Strip
        // feature-gated capabilities at the server/worker boundary so they
        // cannot reappear in the runtime tool surface.
        let feature_flags = crate::services::org_feature_flags::resolve_org_feature_flags(
            &self.db,
            req.org_id,
            &everruns_platform::FeatureFlags::current(),
        )
        .await
        .map_err(|error| {
            tracing::error!(%error, org_id = req.org_id, "Failed to resolve runtime feature flags");
            Status::internal("Failed to resolve organization feature flags")
        })?;
        session
            .capabilities
            .retain(|capability| feature_flags.is_capability_enabled(capability.capability_id()));
        if let Some(agent) = agent.as_mut() {
            agent.capabilities.retain(|capability| {
                feature_flags.is_capability_enabled(capability.capability_id())
            });
        }
        if let Some(harness) = harness.as_mut() {
            harness.capabilities.retain(|capability| {
                feature_flags.is_capability_enabled(capability.capability_id())
            });
        }

        // Convert to proto types
        use everruns_internal_protocol::schema_session_to_proto;

        let proto_agent = agent.as_ref().map(schema_agent_to_proto);
        let proto_harness = harness.as_ref().map(schema_harness_to_proto);

        let proto_session = schema_session_to_proto(&session);

        // Load messages from events using EventService with limit
        let default_limit: i32 = std::env::var("TURN_CONTEXT_MESSAGE_LIMIT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_TURN_CONTEXT_MESSAGE_LIMIT);
        let message_limit = normalize_turn_context_message_limit(req.message_limit, default_limit);
        let fetch_limit = message_limit.saturating_add(1);

        // Fetch one extra to detect truncation
        let events = self
            .event_service
            .list_message_events_limited(session_id, Some(fetch_limit))
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

            proto_messages.push(message_to_proto(&message));
        }

        // Get model with provider (decrypted API key) via ProviderResolverService
        // Priority: session model > agent model > harness model > default model
        let model_id = session
            .model_id
            .or(agent.as_ref().and_then(|a| a.default_model_id))
            .or(harness.as_ref().and_then(|h| h.default_model_id));

        let model: Option<proto::ResolvedModel> = if let Some(mid) = model_id {
            self.provider_resolver_service
                .resolve_model(req.org_id, mid.uuid())
                .await
                .map_err(|e| {
                    tracing::error!("Failed to resolve model: {}", e);
                    Status::internal("Failed to resolve model")
                })?
                .map(Self::resolved_model_to_proto)
        } else {
            // Try to get the default model
            self.provider_resolver_service
                .resolve_default_model(req.org_id)
                .await
                .map_err(|e| {
                    tracing::error!("Failed to resolve default model: {}", e);
                    Status::internal("Failed to resolve default model")
                })?
                .map(Self::resolved_model_to_proto)
        };

        // Append org-scoped tool definitions first so scoped definitions win on
        // name collisions when RuntimeAgentBuilder deduplicates with last-wins.
        let local_mcp_tool_definitions = if let Some(ref harness) = harness {
            let effective = crate::domains::mcp_servers::scoped_mcp::merge_effective_scoped_mcp_servers_with_capabilities(
                harness,
                agent.as_ref(),
                &session,
                self.capability_service.registry(),
            );

            if let Err(error) =
                crate::domains::mcp_servers::scoped_mcp::validate_scoped_mcp_servers(&effective)
            {
                tracing::warn!(error = %error, "Invalid scoped MCP server config, skipping");
                vec![]
            } else {
                crate::domains::mcp_servers::scoped_mcp::build_scoped_mcp_tool_definitions(
                    &effective,
                    Some(session.id),
                    self.connection_resolver.as_ref(),
                    self.mcp_server_service.egress_service().as_ref(),
                )
                .await
                .map(|defs| {
                    defs.into_iter()
                        .map(|tool| McpToolDef {
                            name: tool.name().to_string(),
                            description: tool.description().to_string(),
                            parameters: Some(everruns_internal_protocol::json_to_proto_struct(
                                tool.parameters(),
                            )),
                            capability_id: tool
                                .capability_attribution()
                                .map(|(id, _)| id.to_string())
                                .unwrap_or_default(),
                            capability_name: tool
                                .capability_attribution()
                                .and_then(|(_, name)| name)
                                .unwrap_or_default()
                                .to_string(),
                        })
                        .collect()
                })
                .unwrap_or_else(|error| {
                    tracing::warn!(error = %error, "Failed to build scoped MCP tool definitions");
                    vec![]
                })
            }
        } else {
            vec![]
        };

        let mut mcp_tool_definitions = Vec::new();
        if let Some(ref a) = agent {
            mcp_tool_definitions.extend(self.build_mcp_tool_definitions(req.org_id, a).await);
        }
        mcp_tool_definitions.extend(local_mcp_tool_definitions);
        let binding_metadata = crate::domains::agents::credentials::list_secret_binding_metadata(
            self.db.as_ref(),
            req.org_id,
            session.agent_id,
        )
        .await
        .map_err(|error| {
            tracing::error!(%error, "Failed to load MCP credential metadata");
            Status::internal("Failed to load MCP credential metadata")
        })?;
        apply_proto_secret_binding_schemas(&mut mcp_tool_definitions, &binding_metadata);

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

        // Get agent with capabilities via domain query
        let public_id = everruns_provider::typed_id::AgentId::from_uuid(agent_id).to_string();
        let agent =
            crate::domains::agents::queries::get_by_public_id(&self.db, req.org_id, &public_id)
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

        let harness = crate::domains::harnesses::queries::resolve_effective(
            &self.db,
            req.org_id,
            everruns_provider::typed_id::HarnessId::from_uuid(harness_id),
        )
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

        use everruns_internal_protocol::schema_session_to_proto;

        let proto_session = session.map(|s| schema_session_to_proto(&s));

        Ok(Response::new(GetSessionResponse {
            session: proto_session,
        }))
    }

    async fn set_session_status(
        &self,
        request: Request<SetSessionStatusRequest>,
    ) -> Result<Response<SetSessionStatusResponse>, Status> {
        use everruns_internal_protocol::schema_session_to_proto;

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

        let proto_session = schema_session_to_proto(&session);

        Ok(Response::new(SetSessionStatusResponse {
            session: Some(proto_session),
        }))
    }

    async fn set_session_title(
        &self,
        request: Request<SetSessionTitleRequest>,
    ) -> Result<Response<SetSessionTitleResponse>, Status> {
        use everruns_internal_protocol::schema_session_to_proto;

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
                    goal: None,
                    agent_identity_id: everruns_durable::UpdateField::Unchanged,
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

        let proto_session = schema_session_to_proto(&session);

        Ok(Response::new(SetSessionTitleResponse {
            session: Some(proto_session),
        }))
    }

    async fn get_message(
        &self,
        request: Request<GetMessageRequest>,
    ) -> Result<Response<GetMessageResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let message_id = parse_uuid(req.message_id.as_ref())?;

        // Load message events and find the one matching message_id
        let events = self
            .event_service
            .list_message_events(session_id)
            .await
            .map_err(|e| Status::internal(format!("Failed to list messages: {}", e)))?;

        for event in events {
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

            if message.id.uuid() != message_id {
                continue;
            }

            return Ok(Response::new(GetMessageResponse {
                message: Some(message_to_proto(&message)),
            }));
        }

        // Message not found
        Ok(Response::new(GetMessageResponse { message: None }))
    }

    async fn load_messages(
        &self,
        request: Request<LoadMessagesRequest>,
    ) -> Result<Response<LoadMessagesResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let message_limit = req.message_limit.map(|limit| limit.max(0));

        let mut query = everruns_core::MessageQuery::new(session_id.into());
        query.limit = message_limit.map(i64::from);
        query.after_sequence = req.after_sequence;
        let events = self
            .db
            .list_message_events_filtered(&query)
            .await
            .map_err(|e| Status::internal(format!("Failed to list messages: {}", e)))?;
        let source_sequence = events
            .iter()
            .map(|event| i64::from(event.sequence))
            .max()
            .or(req.after_sequence);
        let total_count = self
            .db
            .count_message_events(everruns_provider::typed_id::SessionId::from_uuid(
                session_id,
            ))
            .await
            .map_err(|e| Status::internal(format!("Failed to count messages: {}", e)))?
            .min(i32::MAX as i64) as i32;

        let mut proto_messages: Vec<proto::Message> = Vec::with_capacity(events.len());

        for event in events {
            let event = crate::EventService::row_to_event(event);
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

            proto_messages.push(message_to_proto(&message));
        }

        Ok(Response::new(LoadMessagesResponse {
            messages: proto_messages,
            total_count,
            source_sequence,
        }))
    }

    async fn get_compaction_checkpoint(
        &self,
        request: Request<proto::GetCompactionCheckpointRequest>,
    ) -> Result<Response<proto::GetCompactionCheckpointResponse>, Status> {
        use everruns_core::CompactionCheckpointStore;

        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?.into();
        let encryption = self.encryption.clone().ok_or_else(|| {
            Status::failed_precondition("checkpoint encryption is not configured")
        })?;
        let store = crate::storage::DbCompactionCheckpointStore::new(self.db.clone(), encryption);
        let checkpoint = store
            .get_latest(session_id, &req.provider_type, &req.model)
            .await
            .map_err(|error| Status::internal(error.to_string()))?
            .map(|checkpoint| {
                let payload_json = serde_json::to_vec(&checkpoint.payload)
                    .map_err(|error| Status::internal(error.to_string()))?;
                Ok::<_, Status>(proto::CompactionCheckpoint {
                    id: Some(proto::Uuid {
                        value: checkpoint.id.to_string(),
                    }),
                    session_id: Some(proto::Uuid {
                        value: checkpoint.session_id.uuid().to_string(),
                    }),
                    source_sequence: checkpoint.source_sequence,
                    provider_type: checkpoint.provider_type,
                    model: checkpoint.model,
                    format_version: checkpoint.format_version,
                    payload_json,
                })
            })
            .transpose()?;
        Ok(Response::new(proto::GetCompactionCheckpointResponse {
            checkpoint,
        }))
    }

    async fn install_compaction_checkpoint(
        &self,
        request: Request<proto::InstallCompactionCheckpointRequest>,
    ) -> Result<Response<proto::InstallCompactionCheckpointResponse>, Status> {
        use everruns_core::CompactionCheckpointStore;

        let checkpoint = request
            .into_inner()
            .checkpoint
            .ok_or_else(|| Status::invalid_argument("missing checkpoint"))?;
        let payload = serde_json::from_slice(&checkpoint.payload_json)
            .map_err(|error| Status::invalid_argument(error.to_string()))?;
        let encryption = self.encryption.clone().ok_or_else(|| {
            Status::failed_precondition("checkpoint encryption is not configured")
        })?;
        let store = crate::storage::DbCompactionCheckpointStore::new(self.db.clone(), encryption);
        let installed = store
            .install(everruns_core::CompactionCheckpoint {
                id: parse_uuid(checkpoint.id.as_ref())?,
                session_id: parse_uuid(checkpoint.session_id.as_ref())?.into(),
                source_sequence: checkpoint.source_sequence,
                provider_type: checkpoint.provider_type,
                model: checkpoint.model,
                format_version: checkpoint.format_version,
                payload,
            })
            .await
            .map_err(|error| Status::internal(error.to_string()))?;
        Ok(Response::new(proto::InstallCompactionCheckpointResponse {
            installed,
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
            phase: message.phase.map(|phase| phase.as_provider_str().to_string()),
            phase_source: message
                .phase_source
                .map(|source| source.as_str().to_string()),
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
            .map_err(|e| Status::invalid_argument(format!("Invalid event: {e}")))?;

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

    async fn get_resolved_model(
        &self,
        request: Request<GetResolvedModelRequest>,
    ) -> Result<Response<GetResolvedModelResponse>, Status> {
        let req = request.into_inner();
        let model_id = parse_uuid(req.model_id.as_ref())?;

        // Resolve model via ProviderResolverService
        let resolved = self
            .provider_resolver_service
            .resolve_model(req.org_id, model_id)
            .await
            .map_err(|e| {
                tracing::error!("Failed to resolve model: {}", e);
                Status::internal("Failed to resolve model")
            })?;

        Ok(Response::new(GetResolvedModelResponse {
            model: resolved.map(Self::resolved_model_to_proto),
        }))
    }

    async fn get_default_model(
        &self,
        request: Request<GetDefaultModelRequest>,
    ) -> Result<Response<GetDefaultModelResponse>, Status> {
        let req = request.into_inner();
        // Resolve default model via ProviderResolverService
        let resolved = self
            .provider_resolver_service
            .resolve_default_model(req.org_id)
            .await
            .map_err(|e| {
                tracing::error!("Failed to resolve default model: {}", e);
                Status::internal("Failed to resolve default model")
            })?;

        // Model resolution is credential-free; provider config resolves later.
        if let Some(ref model) = resolved {
            tracing::debug!(
                model_id = %model.model_id,
                provider_type = %model.provider_type,
                provider_id = %model.provider_id,
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
    // Session file operations (via WorkspaceFileService)
    // ========================================================================

    async fn session_read_file(
        &self,
        request: Request<SessionReadFileRequest>,
    ) -> Result<Response<SessionReadFileResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;

        // Read file via WorkspaceFileService
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

        // Delete via WorkspaceFileService
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

        // List directory via WorkspaceFileService
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

        // Get file stat via WorkspaceFileService
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

        let options = everruns_core::GrepOptions {
            path_pattern: req.path_pattern,
            before_context: req.before_context as usize,
            after_context: req.after_context as usize,
            offset: req.offset as usize,
            limit: req.limit as usize,
            max_bytes: req.max_bytes as usize,
        };
        let grep_result = everruns_core::session_files::SessionFileSystem::grep_files_with_options(
            &self.session_file_service,
            everruns_provider::typed_id::SessionId::from_uuid(session_id),
            &req.pattern,
            &options,
        )
        .await
        .map_err(|e| {
            // Check if it's a regex error
            if e.to_string().contains("regex") {
                return Status::invalid_argument(format!("Invalid regex pattern: {}", e));
            }
            tracing::error!("Failed to grep files: {}", e);
            Status::internal("Failed to grep files")
        })?;

        let matches = grep_result
            .matches
            .into_iter()
            .map(|item| proto::GrepMatch {
                path: item.path,
                line_number: item.line_number as u64,
                line: item.line,
            })
            .collect();
        let blocks = grep_result
            .blocks
            .into_iter()
            .map(|block| proto::GrepContextBlock {
                path: block.path,
                start_line: block.start_line as u64,
                end_line: block.end_line as u64,
                match_line_numbers: block
                    .match_line_numbers
                    .into_iter()
                    .map(|line| line as u64)
                    .collect(),
                lines: block
                    .lines
                    .into_iter()
                    .map(|line| proto::GrepContextLine {
                        line_number: line.line_number as u64,
                        line: line.line,
                        is_match: line.is_match,
                    })
                    .collect(),
            })
            .collect();

        Ok(Response::new(SessionGrepFilesResponse {
            matches,
            blocks,
            total_matches: grep_result.total_matches as u64,
            returned_matches: grep_result.returned_matches as u64,
            bytes_returned: grep_result.bytes_returned as u64,
            bytes_total: grep_result.bytes_total as u64,
            next_offset: grep_result.next_offset.map(|offset| offset as u64),
            byte_truncated: grep_result.byte_truncated,
        }))
    }

    async fn session_create_directory(
        &self,
        request: Request<SessionCreateDirectoryRequest>,
    ) -> Result<Response<SessionCreateDirectoryResponse>, Status> {
        use everruns_internal_protocol::{datetime_to_proto_timestamp, uuid_to_proto_uuid};

        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;

        // Create directory via WorkspaceFileService
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

        let event = WorkflowEvent::ActivityScheduled {
            activity_id: task_def.activity_id.clone(),
            activity_type: task_def.activity_type.clone(),
            input: input.clone(),
            options: options.clone(),
        };
        append_event(store.as_ref(), workflow_id, event)
            .await
            .map_err(|e| {
                tracing::error!("Failed to append ActivityScheduled event: {}", e);
                Status::internal("Failed to append ActivityScheduled event")
            })?;

        let task = TaskDefinition {
            workflow_id: Some(workflow_id),
            activity_id: task_def.activity_id.clone(),
            activity_type: task_def.activity_type.clone(),
            input,
            options,
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

        // Notify NATS subscribers (no-op for PG backend — PG uses DB triggers)
        if let Some(broadcaster) = &self.task_broadcaster {
            broadcaster
                .notify_task_available(&task_def.activity_type)
                .await;
        }

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

        let proto_tasks: Vec<proto::DurableClaimedTask> = tasks
            .into_iter()
            .map(|t| proto::DurableClaimedTask {
                id: Some(uuid_to_proto_uuid(t.id)),
                workflow_id: t.workflow_id.map(uuid_to_proto_uuid),
                activity_id: t.activity_id,
                activity_type: t.activity_type,
                input: Some(everruns_internal_protocol::json_to_proto_struct(&t.input)),
                attempt: t.attempt as i32,
                max_attempts: t.max_attempts as i32,
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

        let outcome = match store
            .fail_task_with_retry(task_id, &req.error, req.retryable.unwrap_or(true))
            .await
        {
            Ok(outcome) => outcome,
            Err(StoreError::TaskNotOwned(_)) => {
                // EVE-639: fail_task now no-ops if the task is no longer 'claimed'
                // (a concurrent stale-reclaim already requeued or sealed it). The
                // failure is absorbed by the reclaimer; report it as a retry
                // (the task is back in the queue) rather than an internal error.
                tracing::info!(
                    %task_id,
                    "Task failure absorbed: task was already reclaimed"
                );
                return Ok(Response::new(FailDurableTaskResponse {
                    failed: true,
                    will_retry: true,
                    terminal_failure_owner: false,
                }));
            }
            Err(e) => {
                tracing::error!("Failed to fail task: {}", e);
                return Err(Status::internal("Failed to fail task"));
            }
        };

        // Check if task will be retried
        let will_retry = matches!(outcome, TaskFailureOutcome::WillRetry { .. });
        let terminal_failure_owner = if will_retry {
            false
        } else if let Some(workflow_id) = task_info.as_ref().and_then(|info| info.workflow_id) {
            let error = WorkflowError::new(req.error.clone());
            let won = store
                .try_fail_workflow(workflow_id, error)
                .await
                .map_err(|error| {
                    tracing::error!(%workflow_id, %error, "Failed to terminalize workflow");
                    Status::internal("Failed to terminalize workflow")
                })?;
            if won {
                record_workflow_failed(store.as_ref(), workflow_id, req.error.clone()).await;
            }
            won
        } else {
            false
        };

        // Notify NATS when task goes back to pending for retry
        if let (true, Some(broadcaster), Some(info)) =
            (will_retry, &self.task_broadcaster, &task_info)
        {
            broadcaster.notify_task_available(&info.activity_type).await;
        }

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
            terminal_failure_owner,
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

    async fn send_durable_workflow_signal(
        &self,
        request: Request<SendDurableWorkflowSignalRequest>,
    ) -> Result<Response<SendDurableWorkflowSignalResponse>, Status> {
        let req = request.into_inner();
        let workflow_id = uuid::Uuid::parse_str(&req.workflow_id)
            .map_err(|e| Status::invalid_argument(format!("Invalid workflow_id: {}", e)))?;
        let proto_signal = req
            .signal
            .ok_or_else(|| Status::invalid_argument("signal is required"))?;

        let payload = proto_signal
            .payload
            .as_ref()
            .map(everruns_internal_protocol::proto_value_to_json)
            .unwrap_or(serde_json::json!({}));
        let signal = everruns_durable::WorkflowSignal::new(proto_signal.signal_type, payload);

        let store = self.durable_store()?;
        store.send_signal(workflow_id, signal).await.map_err(|e| {
            tracing::error!("Failed to send workflow signal: {}", e);
            Status::internal("Failed to send workflow signal")
        })?;

        Ok(Response::new(SendDurableWorkflowSignalResponse {}))
    }

    async fn get_and_consume_durable_workflow_signals(
        &self,
        request: Request<GetAndConsumeDurableWorkflowSignalsRequest>,
    ) -> Result<Response<GetAndConsumeDurableWorkflowSignalsResponse>, Status> {
        let req = request.into_inner();
        let workflow_id = uuid::Uuid::parse_str(&req.workflow_id)
            .map_err(|e| Status::invalid_argument(format!("Invalid workflow_id: {}", e)))?;

        let store = self.durable_store()?;
        let signals = if req.signal_type.is_empty() {
            store.consume_pending_signals(workflow_id).await
        } else {
            store
                .consume_pending_signals_by_type(workflow_id, &req.signal_type)
                .await
        }
        .map_err(|e| {
            tracing::error!("Failed to consume pending signals: {}", e);
            Status::internal("Failed to consume pending signals")
        })?;

        let proto_signals = signals
            .into_iter()
            .map(|s| ProtoDurableWorkflowSignal {
                signal_type: s.signal_type,
                payload: Some(json_value_to_proto(s.payload)),
                sent_at: s.sent_at.to_rfc3339(),
            })
            .collect();

        Ok(Response::new(GetAndConsumeDurableWorkflowSignalsResponse {
            signals: proto_signals,
        }))
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

        // Prefer presigned URL to avoid sending image data over gRPC
        if let Some(url) = self.presigned_image_url(image_id, req.org_id) {
            // Metadata-only lookup — avoids loading the full image blob
            let media_type = match self.db.get_image_info(req.org_id, image_id).await {
                Ok(Some(info)) => info.content_type,
                Ok(None) => {
                    return Ok(Response::new(ResolveImageResponse {
                        found: false,
                        base64: String::new(),
                        media_type: String::new(),
                        url: String::new(),
                    }));
                }
                Err(e) => {
                    tracing::error!(%image_id, error = %e, "Failed to get image");
                    return Err(Status::internal("Failed to get image"));
                }
            };

            return Ok(Response::new(ResolveImageResponse {
                found: true,
                base64: String::new(),
                media_type,
                url,
            }));
        }

        // Fallback: send base64 over gRPC (when API_BASE_URL is not configured)
        let image_row = match self.db.get_image(req.org_id, image_id).await {
            Ok(Some(row)) => row,
            Ok(None) => {
                return Ok(Response::new(ResolveImageResponse {
                    found: false,
                    base64: String::new(),
                    media_type: String::new(),
                    url: String::new(),
                }));
            }
            Err(e) => {
                tracing::error!(%image_id, error = %e, "Failed to get image");
                return Err(Status::internal("Failed to get image"));
            }
        };

        let base64_data = base64::engine::general_purpose::STANDARD.encode(&image_row.data);

        Ok(Response::new(ResolveImageResponse {
            found: true,
            base64: base64_data,
            media_type: image_row.content_type,
            url: String::new(),
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
        let use_presigned = self.api_base_url.is_some() && self.presign_secret.is_some();

        for proto_id in req.image_ids {
            let image_id = parse_uuid(Some(&proto_id))?;

            if use_presigned {
                // Metadata-only lookup — avoids loading the full image blob
                match self.db.get_image_info(req.org_id, image_id).await {
                    Ok(Some(info)) => {
                        let url = self
                            .presigned_image_url(image_id, req.org_id)
                            .unwrap_or_default();
                        images.insert(
                            image_id.to_string(),
                            ResolvedImageData {
                                base64: String::new(),
                                media_type: info.content_type,
                                url,
                            },
                        );
                    }
                    Ok(None) => {
                        tracing::debug!(%image_id, "Image not found during batch resolution");
                    }
                    Err(e) => {
                        tracing::warn!(%image_id, error = %e, "Failed to get image during batch resolution");
                    }
                }
            } else {
                // Fallback: base64 over gRPC
                match self.db.get_image(req.org_id, image_id).await {
                    Ok(Some(row)) => {
                        let base64_data =
                            base64::engine::general_purpose::STANDARD.encode(&row.data);
                        images.insert(
                            image_id.to_string(),
                            ResolvedImageData {
                                base64: base64_data,
                                media_type: row.content_type,
                                url: String::new(),
                            },
                        );
                    }
                    Ok(None) => {
                        tracing::debug!(%image_id, "Image not found during batch resolution");
                    }
                    Err(e) => {
                        tracing::warn!(%image_id, error = %e, "Failed to get image during batch resolution");
                    }
                }
            }
        }

        Ok(Response::new(ResolveImagesResponse { images }))
    }

    async fn create_image_artifact(
        &self,
        request: Request<CreateImageArtifactRequest>,
    ) -> Result<Response<CreateImageArtifactResponse>, Status> {
        let req = request.into_inner();
        if req.data.len() > crate::api::images::MAX_IMAGE_SIZE {
            return Err(Status::invalid_argument(format!(
                "Image exceeds maximum size of {}MB",
                crate::api::images::MAX_IMAGE_SIZE / (1024 * 1024)
            )));
        }
        let metadata = req
            .metadata
            .as_ref()
            .map(everruns_internal_protocol::proto_struct_to_json)
            .unwrap_or_else(|| serde_json::json!({}));
        let (thumbnail_data, thumbnail_content_type) =
            crate::api::images::generate_thumbnail(&req.data, &req.content_type)
                .map(|(data, content_type)| (Some(data), Some(content_type)))
                .unwrap_or((None, None));

        let row = self
            .db
            .create_image(
                req.org_id,
                crate::storage::models::CreateImageRow {
                    org_id: req.org_id,
                    filename: req.filename,
                    content_type: req.content_type,
                    size_bytes: req.data.len() as i64,
                    data: req.data,
                    thumbnail_data,
                    thumbnail_content_type,
                    metadata,
                },
            )
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "Failed to create image artifact");
                Status::internal("Failed to create image artifact")
            })?;

        Ok(Response::new(CreateImageArtifactResponse {
            image: Some(Self::image_info_row_to_proto(
                crate::storage::models::ImageInfoRow {
                    id: row.id,
                    org_id: row.org_id,
                    filename: row.filename,
                    content_type: row.content_type,
                    size_bytes: row.size_bytes,
                    metadata: row.metadata,
                    created_at: row.created_at,
                },
            )),
        }))
    }

    async fn get_image_artifact(
        &self,
        request: Request<GetImageArtifactRequest>,
    ) -> Result<Response<GetImageArtifactResponse>, Status> {
        let req = request.into_inner();
        let image_id = parse_uuid(req.image_id.as_ref())?;
        let row = self.db.get_image(req.org_id, image_id).await.map_err(|e| {
            tracing::error!(%image_id, error = %e, "Failed to get image artifact");
            Status::internal("Failed to get image artifact")
        })?;

        Ok(Response::new(GetImageArtifactResponse {
            image: row.map(Self::image_row_to_proto),
        }))
    }

    async fn get_image_artifact_info(
        &self,
        request: Request<GetImageArtifactInfoRequest>,
    ) -> Result<Response<GetImageArtifactInfoResponse>, Status> {
        let req = request.into_inner();
        let image_id = parse_uuid(req.image_id.as_ref())?;
        let row = self
            .db
            .get_image_info(req.org_id, image_id)
            .await
            .map_err(|e| {
                tracing::error!(%image_id, error = %e, "Failed to get image artifact info");
                Status::internal("Failed to get image artifact info")
            })?;

        Ok(Response::new(GetImageArtifactInfoResponse {
            image: row.map(Self::image_info_row_to_proto),
        }))
    }

    async fn get_default_provider_credentials(
        &self,
        request: Request<GetDefaultProviderCredentialsRequest>,
    ) -> Result<Response<GetDefaultProviderCredentialsResponse>, Status> {
        let req = request.into_inner();
        let resolved = if req.provider_id.is_empty() {
            self.provider_resolver_service
                .resolve_provider_credentials(req.org_id, &req.provider_type)
                .await
                .map(|value| {
                    value.map(|credentials| {
                        (
                            req.provider_type.clone(),
                            Some(credentials.api_key),
                            credentials.base_url,
                            Default::default(),
                        )
                    })
                })
        } else {
            self.provider_resolver_service
                .resolve_runtime_provider_config(req.org_id, &req.provider_id)
                .await
                .map(|value| {
                    value.map(|provider| {
                        (
                            provider.provider_type,
                            provider.api_key,
                            provider.base_url,
                            provider.request_options,
                        )
                    })
                })
        }
        .map_err(|e| {
            tracing::error!(
                provider_type = %req.provider_type,
                error = %e,
                "Failed to resolve provider credentials"
            );
            Status::internal("Failed to resolve provider credentials")
        })?;

        Ok(Response::new(match resolved {
            Some((provider_type, api_key, base_url, request_options)) => {
                // Empty when the connection configures nothing, so the worker
                // applies no options rather than an empty JSON object.
                let request_options_json = if request_options.is_empty() {
                    String::new()
                } else {
                    serde_json::to_string(&request_options).unwrap_or_default()
                };
                GetDefaultProviderCredentialsResponse {
                    found: true,
                    api_key: api_key.unwrap_or_default(),
                    base_url: base_url.unwrap_or_default(),
                    provider_type,
                    request_options_json,
                }
            }
            None => GetDefaultProviderCredentialsResponse {
                found: false,
                api_key: String::new(),
                base_url: String::new(),
                provider_type: String::new(),
                request_options_json: String::new(),
            },
        }))
    }

    // ========================================================================
    // MCP server operations
    // ========================================================================

    async fn get_mcp_server_by_prefix(
        &self,
        request: Request<GetMcpServerByPrefixRequest>,
    ) -> Result<Response<GetMcpServerByPrefixResponse>, Status> {
        let req = request.into_inner();
        let mut runtime_agent_id = None;

        if let Some(session_id) = req.session_id.as_ref() {
            let session_id = parse_uuid(Some(session_id))?;
            let internal_caller = everruns_core::Caller::internal(req.org_id);

            if let Some(session) = self
                .session_service
                .get(&internal_caller, session_id, None)
                .await
                .map_err(|e| {
                    tracing::error!("Failed to get session for scoped MCP lookup: {}", e);
                    Status::internal("Failed to resolve scoped MCP server")
                })?
                && let Some(harness) = crate::domains::harnesses::queries::resolve_effective(
                    &self.db,
                    req.org_id,
                    session.harness_id,
                )
                .await
                .map_err(|e| {
                    tracing::error!("Failed to get harness for scoped MCP lookup: {}", e);
                    Status::internal("Failed to resolve scoped MCP server")
                })?
            {
                runtime_agent_id = session.agent_id;
                let agent = if let Some(agent_id) = session.agent_id {
                    crate::domains::agents::queries::get_by_public_id(
                        &self.db,
                        req.org_id,
                        &agent_id.to_string(),
                    )
                    .await
                    .map_err(|e| {
                        tracing::error!("Failed to get agent for scoped MCP lookup: {}", e);
                        Status::internal("Failed to resolve scoped MCP server")
                    })?
                } else {
                    None
                };

                if let Some(r) = crate::domains::mcp_servers::scoped_mcp::resolve_scoped_mcp_server_with_capabilities(
                    &harness,
                    agent.as_ref(),
                    &session,
                    &req.server_prefix,
                    self.capability_service.registry(),
                ) {
                    let secret_bindings = crate::domains::agents::credentials::resolve_runtime_secret_bindings(
                        self.db.as_ref(),
                        self.encryption.as_deref(),
                        req.org_id,
                        runtime_agent_id,
                        &r.name,
                        &r.url,
                    )
                    .await
                    .map_err(|error| {
                        tracing::error!(%error, "Failed to resolve Agent MCP credentials");
                        Status::internal("Failed to resolve MCP credentials")
                    })?;
                    return Ok(Response::new(GetMcpServerByPrefixResponse {
                        server: Some(McpServerInfo {
                            id: Some(proto::Uuid {
                                value: r.id.to_string(),
                            }),
                            name: r.name,
                            url: r.url,
                            api_key: r.api_key,
                            headers: r.headers,
                            auth_mode: r.auth_mode.to_string(),
                            protocol_mode: r.protocol_mode.to_string(),
                            oauth_provider_id: r.oauth_provider_id,
                            secret_bindings: flatten_secret_bindings(secret_bindings),
                        }),
                    }));
                }
            }
        }

        let internal_caller = everruns_core::Caller::internal(req.org_id);
        let resolved = self
            .mcp_server_service
            .resolve_by_prefix(&internal_caller, &req.server_prefix)
            .await
            .map_err(|e| {
                tracing::error!("Failed to resolve MCP server: {}", e);
                Status::internal("Failed to resolve MCP server")
            })?;

        let server_info = if let Some(r) = resolved {
            let secret_bindings =
                crate::domains::agents::credentials::resolve_runtime_secret_bindings(
                    self.db.as_ref(),
                    self.encryption.as_deref(),
                    req.org_id,
                    runtime_agent_id,
                    &r.name,
                    &r.url,
                )
                .await
                .map_err(|error| {
                    tracing::error!(%error, "Failed to resolve Agent MCP credentials");
                    Status::internal("Failed to resolve MCP credentials")
                })?;
            Some(McpServerInfo {
                id: Some(proto::Uuid {
                    value: r.id.to_string(),
                }),
                name: r.name,
                url: r.url,
                api_key: r.api_key,
                headers: r.headers,
                auth_mode: r.auth_mode.to_string(),
                protocol_mode: r.protocol_mode.to_string(),
                oauth_provider_id: r.oauth_provider_id,
                secret_bindings: flatten_secret_bindings(secret_bindings),
            })
        } else {
            None
        };

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
            .collect::<everruns_provider::error::Result<Vec<_>>>()
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
                everruns_provider::typed_id::LeasedResourceId::from_uuid(resource_id),
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
                everruns_provider::typed_id::LeasedResourceId::from_uuid(resource_id),
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
    // Session resource registry operations
    // ========================================================================

    async fn register_session_resource(
        &self,
        request: Request<RegisterSessionResourceRequest>,
    ) -> Result<Response<RegisterSessionResourceResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let registry = self.session_resource_registry();

        let entry = registry
            .register(everruns_core::RegisterSessionResource {
                session_id: session_id.into(),
                resource_id: req.resource_id,
                kind: req.kind,
                display_name: req.display_name,
                status: everruns_core::SessionResourceStatus::from(req.status.as_str()),
                metadata: req
                    .metadata
                    .as_ref()
                    .map(everruns_internal_protocol::proto_struct_to_json)
                    .unwrap_or_else(|| serde_json::json!({})),
            })
            .await
            .map_err(|e| {
                tracing::error!("Failed to register session resource: {e}");
                Status::internal("Failed to register session resource")
            })?;

        Ok(Response::new(RegisterSessionResourceResponse {
            entry: Some(session_resource_entry_to_proto(&entry)),
        }))
    }

    async fn update_session_resource_status(
        &self,
        request: Request<UpdateSessionResourceStatusRequest>,
    ) -> Result<Response<UpdateSessionResourceStatusResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let registry = self.session_resource_registry();

        let entry = registry
            .update_status(
                session_id.into(),
                &req.resource_id,
                everruns_core::SessionResourceStatus::from(req.status.as_str()),
            )
            .await
            .map_err(|e| {
                tracing::error!("Failed to update session resource status: {e}");
                Status::internal("Failed to update session resource status")
            })?;

        Ok(Response::new(UpdateSessionResourceStatusResponse {
            entry: entry.as_ref().map(session_resource_entry_to_proto),
        }))
    }

    async fn list_session_resources(
        &self,
        request: Request<ListSessionResourcesRequest>,
    ) -> Result<Response<ListSessionResourcesResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let registry = self.session_resource_registry();

        let filter = if req.kind.is_some() || req.status.is_some() {
            Some(everruns_core::SessionResourceFilter {
                kind: req.kind,
                status: req
                    .status
                    .as_deref()
                    .map(everruns_core::SessionResourceStatus::from),
            })
        } else {
            None
        };

        let entries = registry
            .list(session_id.into(), filter.as_ref())
            .await
            .map_err(|e| {
                tracing::error!("Failed to list session resources: {e}");
                Status::internal("Failed to list session resources")
            })?;

        Ok(Response::new(ListSessionResourcesResponse {
            entries: entries
                .iter()
                .map(session_resource_entry_to_proto)
                .collect(),
        }))
    }

    async fn deregister_session_resource(
        &self,
        request: Request<DeregisterSessionResourceRequest>,
    ) -> Result<Response<DeregisterSessionResourceResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let registry = self.session_resource_registry();

        let removed = registry
            .deregister(session_id.into(), &req.resource_id)
            .await
            .map_err(|e| {
                tracing::error!("Failed to deregister session resource: {e}");
                Status::internal("Failed to deregister session resource")
            })?;

        Ok(Response::new(DeregisterSessionResourceResponse { removed }))
    }

    // ========================================================================
    // Session task registry
    // ========================================================================

    async fn create_session_task(
        &self,
        request: Request<CreateSessionTaskRequest>,
    ) -> Result<Response<SessionTaskResponse>, Status> {
        let req = request.into_inner();
        let create = req
            .create
            .ok_or_else(|| Status::invalid_argument("Missing create payload"))?;
        let input = everruns_internal_protocol::proto_to_create_session_task(create)
            .map_err(|e| Status::invalid_argument(format!("Invalid task create payload: {e}")))?;

        let task = self
            .session_task_registry()
            .create(input)
            .await
            .map_err(|e| {
                tracing::error!("Failed to create session task: {e}");
                Status::internal("Failed to create session task")
            })?;

        Ok(Response::new(SessionTaskResponse {
            task: Some(everruns_internal_protocol::session_task_to_proto(&task)),
        }))
    }

    async fn update_session_task(
        &self,
        request: Request<UpdateSessionTaskRequest>,
    ) -> Result<Response<OptionalSessionTaskResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let update_proto = req
            .update
            .ok_or_else(|| Status::invalid_argument("Missing update payload"))?;
        let update = everruns_internal_protocol::proto_to_session_task_update(update_proto);

        let task = self
            .session_task_registry()
            .update(session_id.into(), &req.task_id, update)
            .await
            .map_err(|e| {
                tracing::error!("Failed to update session task: {e}");
                Status::internal("Failed to update session task")
            })?;

        Ok(Response::new(OptionalSessionTaskResponse {
            task: task
                .as_ref()
                .map(everruns_internal_protocol::session_task_to_proto),
        }))
    }

    async fn get_session_task(
        &self,
        request: Request<GetSessionTaskRequest>,
    ) -> Result<Response<OptionalSessionTaskResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;

        let task = self
            .session_task_registry()
            .get(session_id.into(), &req.task_id)
            .await
            .map_err(|e| {
                tracing::error!("Failed to get session task: {e}");
                Status::internal("Failed to get session task")
            })?;

        Ok(Response::new(OptionalSessionTaskResponse {
            task: task
                .as_ref()
                .map(everruns_internal_protocol::session_task_to_proto),
        }))
    }

    async fn list_session_tasks(
        &self,
        request: Request<ListSessionTasksRequest>,
    ) -> Result<Response<ListSessionTasksResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;

        let filter = if req.kind.is_some() || req.state.is_some() {
            Some(everruns_core::SessionTaskFilter {
                kind: req.kind,
                state: req
                    .state
                    .as_deref()
                    .map(everruns_core::SessionTaskState::from),
            })
        } else {
            None
        };

        let tasks = self
            .session_task_registry()
            .list(session_id.into(), filter.as_ref())
            .await
            .map_err(|e| {
                tracing::error!("Failed to list session tasks: {e}");
                Status::internal("Failed to list session tasks")
            })?;

        Ok(Response::new(ListSessionTasksResponse {
            tasks: tasks
                .iter()
                .map(everruns_internal_protocol::session_task_to_proto)
                .collect(),
        }))
    }

    async fn request_cancel_session_task(
        &self,
        request: Request<RequestCancelSessionTaskRequest>,
    ) -> Result<Response<OptionalSessionTaskResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;

        let task = self
            .session_task_registry()
            .request_cancel(session_id.into(), &req.task_id)
            .await
            .map_err(|e| {
                tracing::error!("Failed to request session task cancel: {e}");
                Status::internal("Failed to request session task cancel")
            })?;

        Ok(Response::new(OptionalSessionTaskResponse {
            task: task
                .as_ref()
                .map(everruns_internal_protocol::session_task_to_proto),
        }))
    }

    async fn record_session_task_message(
        &self,
        request: Request<RecordSessionTaskMessageRequest>,
    ) -> Result<Response<SessionTaskMessageResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let message_proto = req
            .message
            .ok_or_else(|| Status::invalid_argument("Missing message payload"))?;
        let message = everruns_internal_protocol::proto_to_new_task_message(message_proto);

        let stored = self
            .session_task_registry()
            .record_message(session_id.into(), &req.task_id, message)
            .await
            .map_err(|e| {
                tracing::error!("Failed to record session task message: {e}");
                Status::internal("Failed to record session task message")
            })?;

        Ok(Response::new(SessionTaskMessageResponse {
            message: Some(everruns_internal_protocol::task_message_to_proto(&stored)),
        }))
    }

    async fn list_session_task_messages(
        &self,
        request: Request<ListSessionTaskMessagesRequest>,
    ) -> Result<Response<ListSessionTaskMessagesResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;

        let messages = self
            .session_task_registry()
            .list_messages(session_id.into(), &req.task_id, req.limit, None)
            .await
            .map_err(|e| {
                tracing::error!("Failed to list session task messages: {e}");
                Status::internal("Failed to list session task messages")
            })?;

        Ok(Response::new(ListSessionTaskMessagesResponse {
            messages: messages
                .iter()
                .map(everruns_internal_protocol::task_message_to_proto)
                .collect(),
        }))
    }

    async fn list_orphaned_session_tasks(
        &self,
        request: Request<ListOrphanedSessionTasksRequest>,
    ) -> Result<Response<ListOrphanedSessionTasksResponse>, Status> {
        let req = request.into_inner();
        if req.stale_after_seconds <= 0 {
            return Err(Status::invalid_argument(
                "stale_after_seconds must be positive",
            ));
        }
        if req.limit <= 0 {
            return Err(Status::invalid_argument("limit must be positive"));
        }
        let stale_after = chrono::Duration::seconds(req.stale_after_seconds);
        // Cap the response size regardless of what the caller asks for.
        let limit = req.limit.min(1000);

        let pairs = self
            .db
            .list_orphaned_session_task_ids(stale_after, limit)
            .await
            .map_err(|e| {
                tracing::error!("Failed to list orphaned session tasks: {e}");
                Status::internal("Failed to list orphaned session tasks")
            })?;

        let entries = pairs
            .into_iter()
            .map(|(session_id, task_id)| OrphanedSessionTaskEntry {
                session_id: session_id.uuid().to_string(),
                task_id,
            })
            .collect();

        Ok(Response::new(ListOrphanedSessionTasksResponse { entries }))
    }

    async fn prune_terminal_session_tasks(
        &self,
        request: Request<PruneTerminalSessionTasksRequest>,
    ) -> Result<Response<PruneTerminalSessionTasksResponse>, Status> {
        let req = request.into_inner();
        if req.ttl_seconds <= 0 {
            return Err(Status::invalid_argument("ttl_seconds must be positive"));
        }
        if req.limit <= 0 {
            return Err(Status::invalid_argument("limit must be positive"));
        }
        let ttl = chrono::Duration::seconds(req.ttl_seconds);
        // Cap the batch size regardless of what the caller asks for.
        let limit = req.limit.min(1000);

        let pruned = self
            .db
            .prune_terminal_session_tasks_with_artifacts(ttl, limit)
            .await
            .map_err(|e| {
                tracing::error!("Failed to prune terminal session tasks: {e}");
                Status::internal("Failed to prune terminal session tasks")
            })?;

        Ok(Response::new(PruneTerminalSessionTasksResponse {
            pruned: pruned as i64,
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
            .create_schedule_enforcing_limits(
                session_id.into(),
                req.description,
                req.cron_expression,
                scheduled_at,
                req.timezone,
            )
            .await
            .map_err(|e| match e {
                everruns_core::session_schedule::ScheduleLimitError::Rejected(msg) => {
                    Status::resource_exhausted(msg)
                }
                everruns_core::session_schedule::ScheduleLimitError::Store(err) => {
                    tracing::error!("Failed to create schedule: {}", err);
                    Status::internal(format!("Failed to create schedule: {}", err))
                }
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
                everruns_provider::typed_id::ScheduleId::from_uuid(schedule_id),
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

    async fn count_active_org_schedules(
        &self,
        request: Request<CountActiveOrgSchedulesRequest>,
    ) -> Result<Response<CountActiveOrgSchedulesResponse>, Status> {
        let req = request.into_inner();
        let store = self.schedule_store(req.org_id)?;

        let count = store.count_active_org_schedules().await.map_err(|e| {
            tracing::error!("Failed to count active org schedules: {}", e);
            Status::internal(format!("Failed to count active org schedules: {}", e))
        })?;

        Ok(Response::new(CountActiveOrgSchedulesResponse { count }))
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

    async fn execute_command(
        &self,
        request: Request<ExecuteCommandRequest>,
    ) -> Result<Response<ExecuteCommandResponse>, Status> {
        let req = request.into_inner();

        if req.api_version != COMMAND_API_VERSION_V1 {
            return Err(Status::invalid_argument(format!(
                "Unsupported command api_version: {}",
                req.api_version
            )));
        }

        if req.params_json.len() > MAX_EXECUTE_COMMAND_PARAMS_BYTES {
            return Err(Status::resource_exhausted(format!(
                "Command params payload too large: size exceeds {} byte limit",
                MAX_EXECUTE_COMMAND_PARAMS_BYTES
            )));
        }

        let params = if req.params_json.is_empty() {
            serde_json::json!({})
        } else {
            serde_json::from_slice::<serde_json::Value>(&req.params_json)
                .map_err(|e| Status::invalid_argument(format!("Invalid params_json: {e}")))?
        };

        let Some(_) = params.as_object() else {
            return Err(Status::invalid_argument(
                "Command params must be a JSON object",
            ));
        };

        // gRPC ExecuteCommand is the auth boundary for worker-driven platform
        // tools. Respect the provided user_id; do not silently upgrade to
        // Caller::internal for user-owned sessions.
        let caller = match req.user_id.as_deref() {
            Some(user_id) => {
                let user_id = user_id
                    .parse()
                    .map_err(|e| Status::invalid_argument(format!("Invalid user_id: {e}")))?;
                crate::auth::caller_resolution::caller_for_user(&self.db, req.org_id, user_id)
                    .await
                    .map_err(|e| {
                        Status::permission_denied(format!("Failed to resolve caller: {e}"))
                    })?
            }
            None => everruns_core::Caller::internal(req.org_id),
        };
        let ctx = self.domain_ctx_for_caller(caller);
        let response = match crate::domains::common::dispatch(&req.name, params, &ctx).await {
            Ok(ok_json) => ExecuteCommandResponse {
                result: Some(proto::execute_command_response::Result::OkJson(
                    ok_json.into_bytes(),
                )),
            },
            Err(error) => ExecuteCommandResponse {
                result: Some(proto::execute_command_response::Result::Error(
                    command_error_to_proto(error),
                )),
            },
        };

        Ok(Response::new(response))
    }

    async fn list_commands(
        &self,
        _request: Request<ListCommandsRequest>,
    ) -> Result<Response<ListCommandsResponse>, Status> {
        let mut commands: Vec<CommandCatalogEntry> =
            inventory::iter::<crate::domains::common::CommandDescriptor>
                .into_iter()
                .map(|desc| {
                    let meta = (desc.meta)();
                    let positional_arg = (desc.positional_arg)();
                    CommandCatalogEntry {
                        name: meta.name.to_string(),
                        api_version: COMMAND_API_VERSION_V1.to_string(),
                        category: meta.category.to_string(),
                        description: meta.description.to_string(),
                        method: meta.method.to_string(),
                        path: meta.path.to_string(),
                        schema_hash: command_schema_hash(&meta, positional_arg),
                        positional_arg: positional_arg.map(str::to_string),
                    }
                })
                .collect();

        commands.sort_by(|a, b| a.name.cmp(&b.name));

        Ok(Response::new(ListCommandsResponse { commands }))
    }

    async fn invoke_platform_command_surface(
        &self,
        request: Request<InvokePlatformCommandSurfaceRequest>,
    ) -> Result<Response<InvokePlatformCommandSurfaceResponse>, Status> {
        // THREAT[TM-AGENT-017]: The worker supplies only the bound session and
        // org. Reload the session and resolve its persisted owner server-side;
        // never accept a caller identity across this trust boundary.
        let req = request.into_inner();
        if req.arguments_json.len() > MAX_EXECUTE_COMMAND_PARAMS_BYTES {
            return Err(Status::resource_exhausted(format!(
                "Platform command arguments exceed {} byte limit",
                MAX_EXECUTE_COMMAND_PARAMS_BYTES
            )));
        }
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let session = self
            .session_service
            .get(
                &everruns_core::Caller::internal(req.org_id),
                session_id,
                None,
            )
            .await
            .map_err(|error| {
                tracing::error!(%error, %session_id, org_id = req.org_id, "Failed to load Platform command session");
                Status::internal("Failed to load Platform command session")
            })?
            .ok_or_else(|| Status::not_found("Session not found"))?;
        let user_id = session.resolved_owner_user_id.ok_or_else(|| {
            Status::permission_denied(
                "Platform command execution requires a user-owned session with a resolved owner",
            )
        })?;
        let caller = crate::auth::caller_resolution::caller_for_user(&self.db, req.org_id, user_id)
            .await
            .map_err(|error| {
                tracing::warn!(%error, %session_id, org_id = req.org_id, %user_id, "Failed to resolve Platform command caller");
                Status::permission_denied("Failed to resolve Platform command caller")
            })?;
        let operation = match PlatformCommandSurfaceOperation::try_from(req.operation) {
            Ok(PlatformCommandSurfaceOperation::Discover) => {
                crate::services::platform_command_surface::Operation::Discover
            }
            Ok(PlatformCommandSurfaceOperation::Query) => {
                crate::services::platform_command_surface::Operation::Query
            }
            Ok(PlatformCommandSurfaceOperation::Execute) => {
                crate::services::platform_command_surface::Operation::Execute
            }
            _ => {
                return Err(Status::invalid_argument(
                    "Invalid platform command operation",
                ));
            }
        };
        let arguments = if req.arguments_json.is_empty() {
            serde_json::json!({})
        } else {
            serde_json::from_slice::<serde_json::Value>(&req.arguments_json).map_err(|error| {
                Status::invalid_argument(format!("Invalid arguments_json: {error}"))
            })?
        };
        if !arguments.is_object() {
            return Err(Status::invalid_argument(
                "Platform command arguments must be a JSON object",
            ));
        }

        let api_base = self
            .api_base_url
            .clone()
            .unwrap_or_else(|| "http://localhost:9300".to_string());
        let ui_base = std::env::var("PUBLIC_APP_URL")
            .or_else(|_| std::env::var("FRONTEND_URL"))
            .unwrap_or_else(|_| api_base.clone());
        let feature_flags = crate::services::org_feature_flags::resolve_org_feature_flags(
            &self.db,
            req.org_id,
            &everruns_platform::FeatureFlags::current(),
        )
        .await
        .map_err(|error| {
            tracing::error!(%error, org_id = req.org_id, "Failed to resolve Platform command feature flags");
            Status::internal("Failed to resolve organization feature flags")
        })?;
        let context = crate::api::mcp_endpoint::catalog::CatalogContext {
            domain_ctx: self
                .domain_ctx_for_caller(caller)
                .with_feature_flags(feature_flags),
            link_builder: crate::api::common::UrlBuilder::new(&api_base, &ui_base),
        };
        let result =
            crate::services::platform_command_surface::invoke(operation, &arguments, context).await;
        let result = match result {
            Ok(output) => {
                Some(proto::invoke_platform_command_surface_response::Result::Output(output))
            }
            Err(error) => {
                Some(proto::invoke_platform_command_surface_response::Result::Error(error))
            }
        };
        Ok(Response::new(InvokePlatformCommandSurfaceResponse {
            result,
        }))
    }

    async fn platform_list_harnesses(
        &self,
        request: Request<PlatformListHarnessesRequest>,
    ) -> Result<Response<PlatformListHarnessesResponse>, Status> {
        let req = request.into_inner();
        let rows = self
            .db
            .list_harnesses(req.org_id, None, false)
            .await
            .map_err(|e| Status::internal(format!("Failed to list harnesses: {}", e)))?;
        let harnesses = crate::domains::harnesses::queries::load_harnesses_list(&self.db, rows)
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
        let capabilities: Vec<everruns_capability::CapabilityRef> = req
            .capabilities
            .iter()
            .map(|id| everruns_capability::CapabilityRef::new(id.as_str()))
            .collect();

        let create_req = crate::domains::harnesses::types::CreateHarnessRequest {
            name: req.name,
            display_name: req.display_name,
            description: req.description,
            // Worker proto carries a plain string; empty/whitespace means no base prompt.
            system_prompt: (!req.system_prompt.trim().is_empty()).then_some(req.system_prompt),
            parent_harness_id: req
                .parent_harness_id
                .as_ref()
                .map(|parent_id| parse_uuid(Some(parent_id)))
                .transpose()?
                .map(everruns_provider::typed_id::HarnessId::from_uuid),
            default_model_id: None,
            tags: vec![],
            capabilities,
            initial_files: vec![],
            mcp_servers: Default::default(),
            network_access: None,
            embedder_metadata: Default::default(),
        };

        use crate::domains::common::Command;
        let ctx = self.domain_ctx(req.org_id);
        let harness = crate::domains::harnesses::CreateHarness(create_req)
            .run(&ctx)
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

        let update_req = crate::domains::harnesses::types::UpdateHarnessRequest {
            name: req.name,
            display_name: req.display_name,
            description: req.description,
            system_prompt: req.system_prompt,
            parent_harness_id: if req.clear_parent_harness_id.unwrap_or(false) {
                Some(None)
            } else {
                req.parent_harness_id
                    .as_ref()
                    .map(|parent_id| parse_uuid(Some(parent_id)))
                    .transpose()?
                    .map(everruns_provider::typed_id::HarnessId::from_uuid)
                    .map(Some)
            },
            default_model_id: None,
            tags: None,
            capabilities: None,
            initial_files: None,
            mcp_servers: None,
            network_access: None,
            status: None,
            embedder_metadata: None,
        };

        use crate::domains::common::Command;
        let harness_public_id =
            everruns_provider::typed_id::HarnessId::from_uuid(harness_id).to_string();
        let ctx = self.domain_ctx(req.org_id);
        let harness = crate::domains::harnesses::UpdateHarnessCmd {
            id: harness_public_id,
            req: update_req,
        }
        .run(&ctx)
        .await
        .map_err(|e| Status::internal(format!("Failed to update harness: {}", e)))?;

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

        use crate::domains::common::Command;
        let harness_public_id =
            everruns_provider::typed_id::HarnessId::from_uuid(harness_id).to_string();
        let ctx = self.domain_ctx(req.org_id);
        crate::domains::harnesses::DeleteHarness {
            id: harness_public_id,
        }
        .run(&ctx)
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

        use crate::domains::common::Command;
        let harness_public_id =
            everruns_provider::typed_id::HarnessId::from_uuid(harness_id).to_string();
        let ctx = self.domain_ctx(req.org_id);
        let harness = crate::domains::harnesses::CopyHarness {
            id: harness_public_id,
        }
        .run(&ctx)
        .await
        .map_err(|e| Status::internal(format!("Failed to copy harness: {}", e)))?;

        // If a new_name was provided, update the copy with the new name
        let harness = if let Some(new_name) = req.new_name {
            let update_req = crate::domains::harnesses::types::UpdateHarnessRequest {
                name: Some(new_name),
                display_name: None,
                description: None,
                system_prompt: None,
                parent_harness_id: None,
                default_model_id: None,
                tags: None,
                capabilities: None,
                initial_files: None,
                mcp_servers: None,
                network_access: None,
                status: None,
                embedder_metadata: None,
            };
            crate::domains::harnesses::UpdateHarnessCmd {
                id: harness.id.to_string(),
                req: update_req,
            }
            .run(&ctx)
            .await
            .map_err(|e| Status::internal(format!("Failed to rename copied harness: {}", e)))
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
        let pagination = crate::api::common::Pagination::new(0, 1000);
        let (rows, _total) = self
            .db
            .list_agents(req.org_id, None, false, pagination)
            .await
            .map_err(|e| Status::internal(format!("Failed to list agents: {}", e)))?;
        let agents = crate::domains::agents::queries::load_agents_list(&self.db, rows)
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
        let capabilities: Vec<everruns_capability::CapabilityRef> = req
            .capabilities
            .iter()
            .map(|id| everruns_capability::CapabilityRef::new(id.as_str()))
            .collect();

        let create_req = crate::domains::agents::types::CreateAgentRequest {
            id: None,
            name: req.name.clone(),
            display_name: req.display_name,
            description: req.description,
            system_prompt: req.system_prompt,
            default_model_id: None,
            harness_id: None,
            harness_name: None,
            tags: vec![],
            capabilities,
            initial_files: vec![],
            tools: vec![],
            mcp_servers: Default::default(),
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
        };

        use crate::domains::common::Command;
        let ctx = self.domain_ctx(req.org_id);
        let agent = crate::domains::agents::CreateAgent(create_req)
            .run(&ctx)
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

        let public_id = everruns_provider::typed_id::AgentId::from_uuid(agent_id).to_string();

        let update_req = crate::domains::agents::types::UpdateAgentRequest {
            name: req.name,
            display_name: req.display_name,
            description: req.description,
            system_prompt: req.system_prompt,
            default_model_id: None,
            harness_id: None,
            harness_name: None,
            tags: None,
            capabilities: None,
            initial_files: None,
            tools: None,
            mcp_servers: None,
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
            status: None,
        };

        use crate::domains::common::Command;
        let ctx = self.domain_ctx(req.org_id);
        let updated = crate::domains::agents::UpdateAgentCmd {
            id: public_id,
            req: update_req,
        }
        .run(&ctx)
        .await
        .map_err(|e| Status::internal(format!("Failed to update agent: {}", e)))?;

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

        use crate::domains::common::Command;
        let public_id = everruns_provider::typed_id::AgentId::from_uuid(agent_id).to_string();
        let ctx = self.domain_ctx(req.org_id);
        crate::domains::agents::DeleteAgent { id: public_id }
            .run(&ctx)
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
            .list(
                &internal_caller,
                None,
                &crate::storage::SessionListFilters {
                    agent_id: agent_id.map(everruns_provider::typed_id::AgentId::from_uuid),
                    ..Default::default()
                },
                pagination,
            )
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
            let public_id = everruns_provider::typed_id::AgentId::from_uuid(aid).to_string();
            let agent =
                crate::domains::agents::queries::get_by_public_id(&self.db, req.org_id, &public_id)
                    .await
                    .map_err(|e| Status::internal(format!("Failed to get agent: {}", e)))?
                    .ok_or_else(|| Status::not_found("Agent not found"))?;
            Some(agent.public_id)
        } else {
            None
        };

        // Parse blueprint config from JSON string if present
        let blueprint_config: Option<serde_json::Value> = req
            .blueprint_config_json
            .as_ref()
            .and_then(|s| serde_json::from_str(s).ok());

        let create_req = crate::api::sessions::CreateSessionRequest {
            source: None,
            workspace_id: None,
            harness_id: Some(everruns_provider::typed_id::HarnessId::from_uuid(
                harness_id,
            )),
            harness_name: None,
            agent_id: agent_public_id,
            agent_name: None,
            agent_identity_id: None,
            title: req.title,
            goal: None,
            locale: req.locale,
            tags: vec![],
            model_id: None,
            capabilities: vec![],
            tools: vec![],
            mcp_servers: Default::default(),
            system_prompt: None,
            initial_files: vec![],
            hints: None,
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
            parent_session_id: None,
            forked_from_session_id: None,
            budget_root_session_id: None,
            seed: everruns_core::SessionSeedMode::Fresh,
        };

        let session = if let Some(blueprint_id) = req.blueprint_id {
            self.session_service
                .create_blueprint_session(
                    &internal_caller,
                    harness_id,
                    blueprint_id,
                    blueprint_config,
                    // Platform-initiated creation is a delegated child of a
                    // running session (subagent spawn / handoff), not an
                    // ordinary API call.
                    everruns_platform::SessionSource::Subagent,
                    create_req,
                )
                .await
                .map_err(|e| Status::internal(format!("Failed to create session: {}", e)))?
        } else {
            self.session_service
                .create(
                    &internal_caller,
                    harness_id,
                    agent_uuid,
                    create_req.agent_id,
                    everruns_platform::SessionSource::Subagent,
                    create_req,
                )
                .await
                .map_err(|e| Status::internal(format!("Failed to create session: {}", e)))?
        };

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
            id: everruns_provider::typed_id::MessageId::from_uuid(message_id),
            role: everruns_core::MessageRole::User,
            content: vec![everruns_core::ContentPart::text(&req.content)],
            phase: None,
            controls: None,
            metadata: None,
            external_actor: None,
            created_at: now,
        };

        let session_id_typed = everruns_provider::typed_id::SessionId::from_uuid(session_id);
        let message_id_typed = everruns_provider::typed_id::MessageId::from_uuid(message_id);

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
                        None,
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

    async fn invoke_scheduled_app_channel(
        &self,
        request: Request<InvokeScheduledAppChannelRequest>,
    ) -> Result<Response<InvokeScheduledAppChannelResponse>, Status> {
        let req = request.into_inner();
        let runner = self
            .runner
            .clone()
            .ok_or_else(|| Status::unavailable("Agent runner not available"))?;
        let message_service = crate::domains::messages::MessageService::new(
            self.db.clone(),
            runner,
            false,
            self.event_service.event_delivery().clone(),
        );

        let result = crate::domains::apps::invoke_scheduled_app_channel(
            &self.db,
            self.encryption.as_ref(),
            &self.session_service,
            &message_service,
            req.org_id,
            &req.app_id,
            &req.channel_id,
        )
        .await
        .map_err(command_error_to_status)?;

        Ok(Response::new(InvokeScheduledAppChannelResponse {
            session_id: result.session_id.to_string(),
            created_session: result.created_session,
        }))
    }

    async fn invoke_agent_trigger(
        &self,
        request: Request<InvokeAgentTriggerRequest>,
    ) -> Result<Response<InvokeAgentTriggerResponse>, Status> {
        let req = request.into_inner();
        let runner = self
            .runner
            .clone()
            .ok_or_else(|| Status::unavailable("Agent runner not available"))?;
        let message_service = crate::domains::messages::MessageService::new(
            self.db.clone(),
            runner,
            false,
            self.event_service.event_delivery().clone(),
        );

        let result = crate::domains::agent_triggers::invoke_agent_trigger(
            &self.db,
            &self.session_service,
            &message_service,
            req.org_id,
            &req.agent_id,
            &req.trigger_id,
        )
        .await
        .map_err(command_error_to_status)?;

        Ok(Response::new(InvokeAgentTriggerResponse {
            session_id: result.session_id.to_string(),
            created_session: result.created_session,
        }))
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
        let feature_flags = crate::services::org_feature_flags::resolve_org_feature_flags(
            &self.db,
            req.org_id,
            &everruns_platform::FeatureFlags::current(),
        )
        .await
        .map_err(|error| {
            tracing::error!(%error, org_id = req.org_id, "Failed to resolve Platform capability feature flags");
            Status::internal("Failed to resolve organization feature flags")
        })?;
        let mut capabilities = self
            .capability_service
            .list_all(req.org_id)
            .await
            .map_err(|e| Status::internal(format!("Failed to list capabilities: {}", e)))?;
        capabilities
            .retain(|capability| feature_flags.is_capability_enabled(capability.id.as_str()));

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
                is_guardrail: c.is_guardrail,
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
        let base_url = everruns_core::config::env_string_any(
            &["PUBLIC_APP_URL", "FRONTEND_URL", "APP_URL"],
            "http://localhost:9300",
        );

        Ok(Response::new(PlatformGetBaseUrlResponse { base_url }))
    }

    // =========================================================================
    // Budget Operations
    // =========================================================================

    async fn check_budgets_for_session(
        &self,
        request: Request<CheckBudgetsForSessionRequest>,
    ) -> Result<Response<CheckBudgetsForSessionResponse>, Status> {
        let req = request.into_inner();

        let all_budgets = self
            .budget_service
            .list_budgets_for_session_hierarchy(
                req.org_id,
                &req.session_id,
                req.agent_id.as_deref(),
            )
            .await;

        if all_budgets.is_empty() {
            return Ok(Response::new(CheckBudgetsForSessionResponse {
                status: "no_budgets".into(),
                budgets: vec![],
                hint: Some(
                    "No budgets are configured for this session. You can proceed without budget constraints.".into(),
                ),
            }));
        }

        // Also run the action check to determine overall status
        let check_result = self
            .budget_service
            .check_budgets_for_session(req.org_id, &req.session_id, req.agent_id.as_deref())
            .await;

        let summaries: Vec<BudgetSummaryProto> = all_budgets
            .iter()
            .map(|b| {
                let pct = if b.limit > 0.0 {
                    (b.balance / b.limit * 100.0).clamp(0.0, 100.0)
                } else {
                    100.0
                };
                BudgetSummaryProto {
                    currency: b.currency.clone(),
                    limit: b.limit,
                    balance: b.balance,
                    soft_limit: b.soft_limit,
                    percent_remaining: (pct * 10.0).round() / 10.0,
                    status: b.status.clone(),
                }
            })
            .collect();

        let overall_status = match check_result.action.as_str() {
            "stop" => "exhausted",
            "pause" => "paused",
            "warn" => "warning",
            _ => "active",
        };

        let hint = if overall_status == "active" {
            let min_pct = summaries
                .iter()
                .map(|s| s.percent_remaining)
                .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
                .unwrap_or(100.0);
            if min_pct < 50.0 {
                Some(format!(
                    "{min_pct}% of budget remaining. Consider prioritizing task completion."
                ))
            } else {
                Some(format!("{min_pct}% of budget remaining."))
            }
        } else {
            check_result.message.clone()
        };

        Ok(Response::new(CheckBudgetsForSessionResponse {
            status: overall_status.into(),
            budgets: summaries,
            hint,
        }))
    }

    async fn check_outbound_tool_rate_limit(
        &self,
        request: Request<CheckOutboundToolRateLimitRequest>,
    ) -> Result<Response<CheckOutboundToolRateLimitResponse>, Status> {
        let req = request.into_inner();
        let allowed = match &self.org_rate_limiter {
            Some(limiter) => limiter.check_outbound_tool_call(&req.org_key).await.is_ok(),
            None => {
                tracing::error!(
                    org_key = %req.org_key,
                    "gRPC outbound tool rate limiter is not configured; denying tool call"
                );
                false
            }
        };

        Ok(Response::new(CheckOutboundToolRateLimitResponse {
            allowed,
        }))
    }

    async fn execute_machine_payment(
        &self,
        request: Request<ExecuteMachinePaymentRequest>,
    ) -> Result<Response<ExecuteMachinePaymentResponse>, Status> {
        use everruns_core::payment::{MachinePaymentRequest, PaymentMethod, PaymentRail};
        use everruns_core::tool_execution::PaymentAuthority;
        use everruns_provider::typed_id::{AgentId, SessionId};

        let req = request.into_inner();
        let session_id = SessionId::parse(&req.session_id)
            .or_else(|_| uuid::Uuid::parse_str(&req.session_id).map(SessionId::from_uuid))
            .map_err(|error| Status::invalid_argument(format!("Invalid session_id: {error}")))?;
        let agent_id = req
            .agent_id
            .as_deref()
            .map(|value| {
                AgentId::parse(value)
                    .or_else(|_| uuid::Uuid::parse_str(value).map(AgentId::from_uuid))
            })
            .transpose()
            .map_err(|error| Status::invalid_argument(format!("Invalid agent_id: {error}")))?;
        let method: PaymentMethod = req.method.parse().map_err(|error| {
            Status::invalid_argument(format!("Invalid payment method: {error}"))
        })?;
        let rail_preference = req
            .rail_preference
            .iter()
            .map(|rail| rail.parse::<PaymentRail>())
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|error| Status::invalid_argument(format!("Invalid payment rail: {error}")))?;
        let body = req
            .body
            .as_ref()
            .map(everruns_internal_protocol::proto_value_to_json);
        let metadata = req
            .metadata
            .as_ref()
            .map(everruns_internal_protocol::proto_value_to_json)
            .unwrap_or_else(|| serde_json::json!({}));

        let authority = crate::domains::payments::ServerPaymentAuthority::new(
            self.db.clone(),
            self.encryption.clone(),
            req.org_id,
            agent_id,
        );
        let response = authority
            .execute_machine_payment(
                session_id,
                MachinePaymentRequest {
                    capability: req.capability,
                    operation: req.operation,
                    method,
                    url: req.url,
                    body,
                    max_amount_usd: req.max_amount_usd,
                    rail_preference,
                    metadata,
                },
            )
            .await
            .map_err(payment_error_to_status)?;

        Ok(Response::new(ExecuteMachinePaymentResponse {
            attempt_id: response.attempt_id.map(|id| id.to_string()),
            amount_usd: response.amount_usd,
            rail: response.rail.map(|rail| rail.to_string()),
            response: Some(everruns_internal_protocol::json_to_proto_value(
                &response.response,
            )),
            receipt: Some(everruns_internal_protocol::json_to_proto_value(
                &response.receipt,
            )),
        }))
    }

    async fn authorize_session_creation(
        &self,
        request: Request<AuthorizeSessionCreationRequest>,
    ) -> Result<Response<AuthorizeSessionCreationResponse>, Status> {
        // THREAT[TM-AUTHZ-014]: Resolve the current session owner server-side;
        // the worker cannot supply a user identity for this decision.
        let req = request.into_inner();
        let session_id = everruns_provider::typed_id::SessionId::parse(&req.session_id)
            .or_else(|_| {
                uuid::Uuid::parse_str(&req.session_id)
                    .map(everruns_provider::typed_id::SessionId::from_uuid)
            })
            .map_err(|error| Status::invalid_argument(format!("Invalid session_id: {error}")))?;
        let session = self
            .db
            .get_session(req.org_id, session_id)
            .await
            .map_err(|error| Status::internal(format!("Failed to load session: {error}")))?
            .ok_or_else(|| Status::not_found("Session not found"))?;
        let user_id = session.resolved_owner_user_id.ok_or_else(|| {
            Status::permission_denied(
                "Detached session creation requires a user-owned session with a resolved owner",
            )
        })?;
        let caller = crate::auth::caller_resolution::caller_for_user(&self.db, req.org_id, user_id)
            .await
            .map_err(|error| {
                Status::permission_denied(format!("Failed to resolve session owner: {error}"))
            })?;
        crate::domains::sessions::SESSION_MANAGE
            .evaluate_with(self.permission_resolver.as_ref(), &caller)
            .map_err(|error| Status::permission_denied(error.message))?;
        let root_session_id = session.root_session_id.unwrap_or(session.id);
        Ok(Response::new(AuthorizeSessionCreationResponse {
            budget_root_session_id: root_session_id.to_string(),
        }))
    }
}

// Helper functions for status conversion

fn payment_error_to_status(error: everruns_provider::error::AgentLoopError) -> Status {
    use everruns_provider::error::AgentLoopError;

    match error {
        AgentLoopError::Configuration(message) | AgentLoopError::ToolExecution(message) => {
            Status::failed_precondition(message)
        }
        AgentLoopError::SessionNotFound(session_id) => {
            Status::not_found(format!("Session not found: {session_id}"))
        }
        AgentLoopError::MessageStore(message) => Status::internal(message),
        AgentLoopError::Internal(error) => Status::internal(error.to_string()),
        other => Status::internal(other.to_string()),
    }
}

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
    use super::{
        DEFAULT_TURN_CONTEXT_MESSAGE_LIMIT, MAX_TURN_CONTEXT_MESSAGE_LIMIT,
        normalize_turn_context_message_limit,
    };

    #[test]
    fn normalize_turn_context_message_limit_uses_clamped_default() {
        assert_eq!(
            normalize_turn_context_message_limit(None, DEFAULT_TURN_CONTEXT_MESSAGE_LIMIT),
            DEFAULT_TURN_CONTEXT_MESSAGE_LIMIT
        );
        assert_eq!(
            normalize_turn_context_message_limit(None, i32::MAX),
            MAX_TURN_CONTEXT_MESSAGE_LIMIT
        );
    }

    #[test]
    fn normalize_turn_context_message_limit_rejects_non_positive_values() {
        assert_eq!(normalize_turn_context_message_limit(Some(0), 123), 123);
        assert_eq!(normalize_turn_context_message_limit(Some(-50), 123), 123);
    }

    #[test]
    fn normalize_turn_context_message_limit_clamps_large_values() {
        assert_eq!(
            normalize_turn_context_message_limit(Some(i32::MAX), 123),
            MAX_TURN_CONTEXT_MESSAGE_LIMIT
        );
    }
}

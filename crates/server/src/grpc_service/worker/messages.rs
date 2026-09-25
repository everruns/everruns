//! Turn context, message load/append, journal, compaction checkpoints.
//!
//! Handler bodies for the `WorkerService` RPCs in this group. The trait impl in
//! `super::super::worker_service_impl` is a delegation layer only: a trait impl
//! cannot span modules, so the work lives here and the trait forwards to it.

use super::support::*;
use crate::grpc_service::*;

impl WorkerServiceImpl {
    pub(crate) async fn handle_get_turn_context(
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
        let feature_flags = crate::services::org_feature_flags::resolve_org_feature_flags_cached(
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
                crate::domains::mcp_servers::scoped_mcp::validate_effective_mcp_servers(&effective)
            {
                tracing::warn!(error = %error, "Invalid scoped MCP server config, skipping");
                vec![]
            } else {
                match crate::domains::mcp_servers::scoped_mcp::build_materialized_scoped_mcp_tool_definitions(
                    &self.db,
                    req.org_id,
                    &effective,
                    Some(session.id),
                    self.connection_resolver.as_ref(),
                    self.mcp_server_service.egress_service().as_ref(),
                )
                .await
                {
                    Ok(definitions) => definitions
                        .into_iter()
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
                        .collect(),
                    Err(error) => {
                        tracing::warn!(error = %error, "Failed to build scoped MCP tool definitions");
                        vec![]
                    }
                }
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

    pub(crate) async fn handle_get_message(
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
            .map_err(|e| internal_status("Failed to list messages", e))?;

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

    pub(crate) async fn handle_load_messages(
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
            .map_err(|e| internal_status("Failed to list messages", e))?;
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
            .map_err(|e| internal_status("Failed to count messages", e))?
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

    pub(crate) async fn handle_native_async_journal(
        &self,
        request: Request<proto::NativeAsyncJournalRequest>,
    ) -> Result<Response<proto::NativeAsyncJournalResponse>, Status> {
        use everruns_core::native_async_store::{
            MAX_NATIVE_ASYNC_CHECKPOINT_BYTES, NativeAsyncLease, NativeAsyncStore,
        };
        use proto::native_async_journal_request::Operation;
        let req = request.into_inner();
        let operation = Operation::try_from(req.operation)
            .map_err(|_| Status::invalid_argument("invalid journal operation"))?;
        if req.checkpoint_json.len() > MAX_NATIVE_ASYNC_CHECKPOINT_BYTES {
            return Err(Status::resource_exhausted(
                "native async checkpoint exceeds 8 MiB",
            ));
        }
        let lease = NativeAsyncLease {
            org_id: req.org_id,
            session_id: parse_uuid(req.session_id.as_ref())?.into(),
            turn_id: parse_uuid(req.turn_id.as_ref())?.into(),
            owner: parse_uuid(req.owner.as_ref())?,
        };
        let pool = self.db.pool().ok_or_else(|| {
            Status::failed_precondition("native async requires shared durable storage")
        })?;
        let encryption = self.encryption.clone().ok_or_else(|| {
            Status::failed_precondition("checkpoint encryption is not configured")
        })?;
        let store = crate::storage::PgNativeAsyncStore::new(pool.clone(), encryption);
        let checkpoint = match operation {
            Operation::Acquire => Some(store.acquire(lease).await),
            Operation::Load => Some(store.load(lease).await),
            Operation::Renew => {
                store.renew(lease).await.map_err(|_| {
                    Status::failed_precondition("native async ownership fence lost")
                })?;
                None
            }
            Operation::Save => {
                let checkpoint = serde_json::from_slice(&req.checkpoint_json)
                    .map_err(|_| Status::invalid_argument("invalid native async checkpoint"))?;
                store.save(lease, &checkpoint).await.map_err(|_| {
                    Status::failed_precondition("native async checkpoint write failed")
                })?;
                None
            }
            Operation::Release => {
                store.release(lease).await.map_err(|_| {
                    Status::failed_precondition("native async ownership fence lost")
                })?;
                None
            }
        }
        .transpose()
        .map_err(|_| Status::failed_precondition("native async journal unavailable or fenced"))?;
        let checkpoint_json = checkpoint
            .map(|checkpoint| serde_json::to_vec(&checkpoint))
            .transpose()
            .map_err(|_| Status::internal("cannot encode native async checkpoint"))?
            .unwrap_or_default();
        Ok(Response::new(proto::NativeAsyncJournalResponse {
            checkpoint_json,
        }))
    }

    pub(crate) async fn handle_get_compaction_checkpoint(
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
            .map_err(|error| internal_status("Failed to load compaction checkpoint", error))?
            .map(|checkpoint| {
                let payload_json = serde_json::to_vec(&checkpoint.payload).map_err(|error| {
                    internal_status("Failed to encode compaction checkpoint", error)
                })?;
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

    pub(crate) async fn handle_install_compaction_checkpoint(
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
            .map_err(|error| internal_status("Failed to install compaction checkpoint", error))?;
        Ok(Response::new(proto::InstallCompactionCheckpointResponse {
            installed,
        }))
    }

    pub(crate) async fn handle_add_message(
        &self,
        request: Request<AddMessageRequest>,
    ) -> Result<Response<AddMessageResponse>, Status> {
        use chrono::Utc;
        use everruns_core::{
            ContentPart, Controls, EventContext, EventRequest, RuntimeMessage, RuntimeMessageRole,
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
        let role = RuntimeMessageRole::from(req.role.as_str());

        // Create the message
        let message = RuntimeMessage {
            id: uuid::Uuid::now_v7().into(),
            role: role.clone(),
            content,
            phase: None,
            phase_source: None,
            controls,
            metadata,
            external_actor: None,
            created_at: Utc::now(),
        };

        // Create typed event request based on role
        let event_request = match role {
            RuntimeMessageRole::User => EventRequest::new(
                session_id.into(),
                EventContext::empty(),
                InputMessageData::new(message.clone()),
            ),
            RuntimeMessageRole::Agent => EventRequest::new(
                session_id.into(),
                EventContext::empty(),
                OutputMessageCompletedData::new(message.clone()),
            ),
            RuntimeMessageRole::System | RuntimeMessageRole::ToolResult => {
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
            phase: message
                .phase
                .map(|phase| phase.as_provider_str().to_string()),
            phase_source: message
                .phase_source
                .map(|source| source.as_str().to_string()),
            external_actor,
        };

        Ok(Response::new(AddMessageResponse {
            message: Some(proto_message),
        }))
    }
}

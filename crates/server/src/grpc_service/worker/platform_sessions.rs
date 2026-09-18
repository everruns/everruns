//! Platform session management and messaging.
//!
//! Handler bodies for the `WorkerService` RPCs in this group. The trait impl in
//! `super::super::worker_service_impl` is a delegation layer only: a trait impl
//! cannot span modules, so the work lives here and the trait forwards to it.

use super::support::*;
use crate::grpc_service::*;

impl WorkerServiceImpl {
    pub(crate) async fn handle_platform_list_sessions(
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
            .map_err(|e| internal_status("Failed to list sessions", e))?;

        let proto_sessions = sessions.iter().map(schema_session_to_proto).collect();
        Ok(Response::new(PlatformListSessionsResponse {
            sessions: proto_sessions,
        }))
    }

    pub(crate) async fn handle_platform_create_session(
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
                    .map_err(|e| internal_status("Failed to get agent", e))?
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
                .map_err(|e| internal_status("Failed to create session", e))?
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
                .map_err(|e| internal_status("Failed to create session", e))?
        };

        Ok(Response::new(PlatformCreateSessionResponse {
            session: Some(schema_session_to_proto(&session)),
        }))
    }

    pub(crate) async fn handle_platform_delete_session(
        &self,
        request: Request<PlatformDeleteSessionRequest>,
    ) -> Result<Response<PlatformDeleteSessionResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let internal_caller = everruns_core::Caller::internal(req.org_id);

        self.session_service
            .delete(&internal_caller, session_id)
            .await
            .map_err(|e| internal_status("Failed to delete session", e))?;

        Ok(Response::new(PlatformDeleteSessionResponse {}))
    }

    pub(crate) async fn handle_platform_send_message(
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
            .map_err(|e| internal_status("Failed to get session", e))?
            .ok_or_else(|| Status::not_found("Session not found"))?;

        // Create message event
        let message_id = uuid::Uuid::now_v7();
        let now = chrono::Utc::now();

        let core_message = everruns_core::Message {
            id: everruns_provider::typed_id::MessageId::from_uuid(message_id),
            role: everruns_core::MessageRole::User,
            content: vec![everruns_core::ContentPart::text(&req.content)],
            phase: None,
            phase_source: None,
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
            .map_err(|e| internal_status("Failed to emit message event", e))?;

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

    pub(crate) async fn handle_invoke_scheduled_app_channel(
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

    pub(crate) async fn handle_invoke_agent_trigger(
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

    pub(crate) async fn handle_platform_get_messages(
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
            .map_err(|e| internal_status("Failed to list messages", e))?;

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

    pub(crate) async fn handle_platform_wait_for_idle(
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
                .map_err(|e| internal_status("Failed to get session", e))?
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
}

// Message service for business logic
//
// Messages are stored as events in the events table. This service handles:
// - Creating user message events
// - Listing messages by querying message events
// - Workflow triggering for user messages

use crate::api::messages::{ContentPart, CreateMessageRequest, Message, MessageRole};
use crate::domains::notifications::NotificationService;
use crate::domains::sessions::limits::OrgCaps;
use crate::errors::{BadRequestError, ConflictError, ResourceNotFoundError};
use crate::execution_metadata;
use crate::services::waiting_turn_resolution::execute_waiting_turn_resolution;
use crate::services::{EventService, PrincipalService};
use crate::storage::StorageBackend;
use crate::storage::models::{
    CreateSessionParticipantRow, ReserveActiveTurnSlotResult, WaitingTurnResolutionPlan,
};
use anyhow::Result;
use chrono::Utc;
use everruns_core::Event;
use everruns_core::events::{
    EventContext, EventData, EventRequest, InputMessageData, OutputMessageCompletedData,
    ToolCompletedData, deserialize_event_data,
};
use everruns_platform::{SessionParticipantKind, SessionParticipantRole};
use everruns_provider::typed_id::{AgentId, HarnessId, MessageId, PrincipalId, SessionId};
use everruns_worker::AgentRunner;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

pub struct MessageService {
    db: Arc<StorageBackend>,
    event_service: EventService,
    notification_service: NotificationService,
    notifications_enabled: bool,
    runner: Arc<dyn AgentRunner>,
    caps: OrgCaps,
}

pub struct CreateMessageContext {
    pub org_id: i64,
    pub user_id: Option<Uuid>,
    pub harness_id: Uuid,
    pub agent_id: Option<Uuid>,
    pub session_id: Uuid,
    pub event_metadata: Option<serde_json::Value>,
    /// HTTP request ID for log correlation. Propagated to durable turn input.
    pub request_id: Option<String>,
}

impl MessageService {
    pub fn new(
        db: Arc<StorageBackend>,
        runner: Arc<dyn AgentRunner>,
        notifications_enabled: bool,
        event_delivery: crate::event_delivery::EventDelivery,
    ) -> Self {
        let event_service = EventService::new(db.clone(), event_delivery);
        let notification_service = NotificationService::new(db.clone());
        Self {
            db,
            event_service,
            notification_service,
            notifications_enabled,
            runner,
            caps: OrgCaps::from_env(),
        }
    }

    pub fn with_caps(mut self, caps: OrgCaps) -> Self {
        self.caps = caps;
        self
    }

    async fn ensure_active_user_participant(
        &self,
        org_id: i64,
        session_id: SessionId,
        user_id: Uuid,
    ) -> Result<PrincipalId> {
        let principal = PrincipalService::new(self.db.clone())
            .ensure_user_principal(org_id, user_id)
            .await?;
        let display_name = self
            .db
            .get_user(user_id)
            .await?
            .map(|user| user.name.trim().to_string())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "User".to_string());

        self.db
            .ensure_active_user_session_participant(CreateSessionParticipantRow {
                org_id,
                session_id,
                kind: SessionParticipantKind::User,
                agent_id: None,
                agent_version_id: None,
                principal_id: principal.id,
                display_name: Some(display_name),
                role: SessionParticipantRole::Member,
                joined_at: None,
            })
            .await?;

        Ok(principal.id)
    }

    /// Create a user message from API request
    ///
    /// Only user messages can be created via the API. This method:
    /// - Creates a message event in the events table
    /// - Triggers workflow execution for the session
    pub async fn create(
        &self,
        ctx: CreateMessageContext,
        req: CreateMessageRequest,
    ) -> Result<Message> {
        tracing::info!(
            session_id = %ctx.session_id,
            harness_id = %ctx.harness_id,
            agent_id = ?ctx.agent_id,
            request_id = ?ctx.request_id,
            "Creating user message"
        );

        let content: Vec<ContentPart> = req
            .message
            .content
            .into_iter()
            .map(ContentPart::from)
            .collect();
        let message_id = Uuid::now_v7();
        let now = Utc::now();
        let session_id = SessionId::from_uuid(ctx.session_id);
        let message_id_typed = MessageId::from_uuid(message_id);
        let core_message = everruns_core::RuntimeMessage {
            id: message_id_typed,
            role: everruns_core::RuntimeMessageRole::User,
            content: content.clone(),
            phase: None,
            phase_source: None,
            controls: req.controls.clone(),
            metadata: req.metadata.clone(),
            external_actor: req.external_actor.clone(),
            created_at: now,
        };
        let event_metadata = if let Some(metadata) = ctx.event_metadata.clone() {
            Some(metadata)
        } else if let Some(user_id) = ctx.user_id {
            let principal_id = self
                .ensure_active_user_participant(ctx.org_id, session_id, user_id)
                .await?;
            execution_metadata::interactive_user_metadata(Some(user_id), Some(principal_id))
        } else {
            self.session_owner_message_metadata(ctx.org_id, session_id)
                .await
        };
        let mut input_event = EventRequest::new(
            session_id,
            EventContext::empty(),
            InputMessageData::new(core_message),
        );
        if let Some(metadata) = event_metadata {
            input_event = input_event.with_metadata(metadata);
        }
        let mut resolution_events = self
            .pending_client_tool_cancellation_events(session_id)
            .await?;
        resolution_events.push(input_event.clone());
        let resolution_plan = WaitingTurnResolutionPlan {
            kind: "user_message".to_string(),
            events: resolution_events,
            session_values: Vec::new(),
            response: serde_json::json!({ "input_message_id": message_id }),
        };

        let (previous_status, resolution_claim) = match self
            .db
            .reserve_active_turn_slot_for_org(
                ctx.org_id,
                session_id,
                self.caps.max_active_turns as i64,
                resolution_plan,
            )
            .await?
        {
            ReserveActiveTurnSlotResult::Accepted {
                previous_status,
                resolution_claim,
            } => (previous_status, resolution_claim),
            ReserveActiveTurnSlotResult::Conflict { current_status } => {
                return Err(ConflictError::new(format!(
                    "Session already has a turn resolution in progress (current status: {current_status})"
                ))
                .into());
            }
            ReserveActiveTurnSlotResult::AtCapacity { active_turns } => {
                return Err(BadRequestError::new(format!(
                    "Too many active turns: org has {} turns executing (limit {}); retry later",
                    active_turns, self.caps.max_active_turns
                ))
                .into());
            }
            ReserveActiveTurnSlotResult::SessionNotFound => {
                return Err(ResourceNotFoundError::new("Session").into());
            }
        };

        let reserved_new_turn = resolution_claim.is_none();
        let result: Result<Message> = async {
            let (runtime_message, sequence) = if let Some(claim) = &resolution_claim {
                let stored_events = execute_waiting_turn_resolution(
                    &self.db,
                    &self.event_service,
                    &self.runner,
                    ctx.org_id,
                    session_id,
                    claim,
                )
                .await?;
                let runtime_message = claim
                    .plan
                    .events
                    .iter()
                    .find_map(|request| match &request.data {
                        EventData::InputMessage(data) => Some(data.message.clone()),
                        _ => None,
                    })
                    .ok_or_else(|| anyhow::anyhow!("resolution plan has no input message"))?;
                let sequence = stored_events
                    .iter()
                    .find(|event| event.event_type == "input.message")
                    .and_then(|event| event.sequence)
                    .unwrap_or(0);
                tracing::info!(
                    session_id = %ctx.session_id,
                    input_message_id = %runtime_message.id,
                    recovered = claim.recovered,
                    "Parked turn resumed after user message"
                );
                (runtime_message, sequence)
            } else {
                let stored_event = self.event_service.emit(input_event).await?;
                let runner = self.runner.clone();
                let org_id = ctx.org_id;
                let harness_id = HarnessId::from_uuid(ctx.harness_id);
                let agent_id = ctx.agent_id.map(AgentId::from_uuid);
                let request_id = ctx.request_id.clone();
                let request_id_log = request_id.as_deref().unwrap_or("").to_string();
                tokio::spawn(async move {
                    if let Err(error) = runner
                        .start_run(
                            org_id,
                            session_id,
                            harness_id,
                            agent_id,
                            message_id_typed,
                            request_id,
                        )
                        .await
                    {
                        tracing::error!(
                            session_id = %session_id,
                            input_message_id = %message_id_typed,
                            request_id = %request_id_log,
                            error = %error,
                            "Failed to start turn workflow"
                        );
                    }
                });
                let runtime_message = match stored_event.data {
                    EventData::InputMessage(data) => data.message,
                    _ => return Err(anyhow::anyhow!("stored input event changed type")),
                };
                (runtime_message, stored_event.sequence.unwrap_or(0))
            };

            let message = Message {
                id: runtime_message.id,
                session_id,
                sequence,
                role: MessageRole::User,
                content: runtime_message.content,
                phase: None,
                phase_source: None,
                controls: runtime_message.controls,
                metadata: runtime_message.metadata,
                external_actor: runtime_message.external_actor,
                created_at: runtime_message.created_at,
            };
            if self.notifications_enabled
                && let Some(user_id) = ctx.user_id
            {
                self.notification_service
                    .create_turn_request(ctx.org_id, user_id, session_id, message.id)
                    .await?;
            }
            Ok(message)
        }
        .await;

        if result.is_err()
            && reserved_new_turn
            && let Err(release_err) = self
                .db
                .release_active_turn_slot_for_org(ctx.org_id, session_id, &previous_status)
                .await
        {
            tracing::warn!(
                org_id = ctx.org_id,
                session_id = %session_id,
                error = %release_err,
                "Failed to release active-turn slot after message-create failure"
            );
        }
        result
    }

    async fn pending_client_tool_cancellation_events(
        &self,
        session_id: SessionId,
    ) -> Result<Vec<EventRequest>> {
        let events = self
            .db
            .list_events(
                session_id,
                None,
                None,
                &["tool.call_requested".to_string()],
                &[],
                None,
                Some(1),
            )
            .await?;
        let Some(event) = events.last() else {
            return Ok(Vec::new());
        };
        let EventData::ToolCallRequested(requested) =
            deserialize_event_data(&event.event_type, event.data.clone())
        else {
            return Ok(Vec::new());
        };
        let turn_id = everruns_provider::typed_id::TurnId::from_uuid(session_id.uuid());
        let event_message_id = MessageId::from_uuid(session_id.uuid());

        Ok(requested
            .tool_calls
            .into_iter()
            .map(|tool_call| {
                EventRequest::new(
                    session_id,
                    EventContext::turn(turn_id, event_message_id),
                    ToolCompletedData::failure(
                        tool_call.id,
                        tool_call.name,
                        "cancelled".to_string(),
                        "Cancelled because the user sent a message instead".to_string(),
                        None,
                    ),
                )
            })
            .collect())
    }

    /// Access the registered durable runner. Used by sibling callers that
    /// need to cancel an in-flight workflow without going through the
    /// session command surface.
    pub fn runner(&self) -> &Arc<dyn AgentRunner> {
        &self.runner
    }

    /// Access the underlying event service. Used by sibling callers that
    /// need to emit lifecycle events outside of `MessageService::create`.
    pub fn event_service(&self) -> &EventService {
        &self.event_service
    }

    async fn session_owner_message_metadata(
        &self,
        org_id: i64,
        session_id: SessionId,
    ) -> Option<serde_json::Value> {
        let session = match self.db.get_session_unscoped(session_id).await {
            Ok(Some(session)) if session.org_id == org_id => session,
            Ok(_) => return None,
            Err(err) => {
                tracing::warn!(
                    org_id,
                    session_id = %session_id,
                    error = %err,
                    "Failed to resolve session owner for message metadata"
                );
                return None;
            }
        };

        Some(json!({
            "initiator": { "type": "api_key" },
            "acting_principal": { "type": "api_key" },
            "initiator_principal_id": session.owner_principal_id,
            "acting_principal_id": session.owner_principal_id,
        }))
    }

    pub async fn list(&self, session_id: Uuid) -> Result<Vec<Message>> {
        self.list_limited(session_id, None).await
    }

    pub async fn list_limited(&self, session_id: Uuid, limit: Option<i32>) -> Result<Vec<Message>> {
        let events = self
            .db
            .list_message_events_limited(SessionId::from_uuid(session_id), limit)
            .await?;
        let mut messages = Vec::with_capacity(events.len());

        for event_row in events {
            match Self::event_to_message(
                session_id,
                &event_row.data,
                &event_row.event_type,
                event_row.sequence,
            ) {
                Ok(message) => messages.push(message),
                Err(e) => {
                    tracing::warn!("Failed to parse message from event {}: {}", event_row.id, e);
                }
            }
        }

        Ok(messages)
    }

    /// Convert stored event data to API Message
    ///
    /// Handles two formats:
    /// - Legacy format: full Event struct with id, type, data, etc.
    /// - New format: EventData directly (InputMessageData, OutputMessageCompletedData, etc.)
    fn event_to_message(
        session_id: Uuid,
        data: &serde_json::Value,
        event_type: &str,
        sequence: i32,
    ) -> std::result::Result<Message, String> {
        // Helper to convert EventData to Message
        let convert =
            |event_data: everruns_core::EventData| -> std::result::Result<Message, String> {
                let core_message = match &event_data {
                    everruns_core::EventData::InputMessage(data) => &data.message,
                    everruns_core::EventData::OutputMessageCompleted(data) => &data.message,
                    everruns_core::EventData::ToolCompleted(data) => {
                        // Separate text and image parts from the result content
                        let mut images: Vec<everruns_provider::tool_types::ToolResultImage> =
                            Vec::new();
                        let result: Option<serde_json::Value> =
                            data.result
                                .as_ref()
                                .map(|parts: &Vec<everruns_core::ContentPart>| {
                                    for part in parts {
                                        if let everruns_core::ContentPart::Image(img) = part
                                            && let (Some(b64), Some(mt)) =
                                                (&img.base64, &img.media_type)
                                        {
                                            images.push(
                                                everruns_provider::tool_types::ToolResultImage {
                                                    base64: b64.clone(),
                                                    media_type: mt.clone(),
                                                },
                                            );
                                        }
                                    }
                                    let text_parts: Vec<&everruns_core::ContentPart> = parts
                                        .iter()
                                        .filter(|p| {
                                            matches!(p, everruns_core::ContentPart::Text(_))
                                        })
                                        .collect();
                                    if text_parts.len() == 1
                                        && let everruns_core::ContentPart::Text(t) = text_parts[0]
                                    {
                                        return serde_json::Value::String(t.text.clone());
                                    }
                                    serde_json::to_value(&text_parts).unwrap_or_default()
                                });
                        let msg = if images.is_empty() {
                            everruns_core::RuntimeMessage::tool_result(
                                &data.tool_call_id,
                                result,
                                data.error.clone(),
                            )
                        } else {
                            everruns_core::RuntimeMessage::tool_result_with_images(
                                &data.tool_call_id,
                                result,
                                images,
                            )
                        };
                        return Ok(Message {
                            id: msg.id,
                            session_id: SessionId::from_uuid(session_id),
                            sequence,
                            role: MessageRole::from(msg.role.to_string().as_str()),
                            content: msg.content,
                            phase: None,
                            phase_source: None,
                            controls: None,
                            metadata: None,
                            external_actor: None,
                            created_at: msg.created_at,
                        });
                    }
                    _ => return Err("unexpected event type".to_string()),
                };

                Ok(Message {
                    id: core_message.id,
                    session_id: SessionId::from_uuid(session_id),
                    sequence,
                    role: MessageRole::from(core_message.role.to_string().as_str()),
                    // `to_public` drops signatures and encrypted payloads:
                    // replay state is not content and must not cross an API
                    // boundary.
                    content: core_message.clone().into_public().content,
                    phase: core_message.phase,
                    phase_source: core_message.phase_source,
                    controls: core_message.controls.clone(),
                    metadata: core_message.metadata.clone(),
                    external_actor: core_message.external_actor.clone(),
                    created_at: core_message.created_at,
                })
            };

        // First try to parse as full Event (legacy format)
        // This has required fields like id, type, session_id, data
        if let Ok(event) = serde_json::from_value::<Event>(data.clone()) {
            return convert(event.data);
        }

        // Fallback: try to parse as specific EventData type directly (new format)
        // We use the event_type hint since EventData's Raw variant catches everything
        match event_type {
            "input.message" => {
                let d: InputMessageData = serde_json::from_value(data.clone())
                    .map_err(|e| format!("invalid input.message data: {}", e))?;
                convert(everruns_core::EventData::InputMessage(d))
            }
            "output.message.completed" => {
                let d: OutputMessageCompletedData = serde_json::from_value(data.clone())
                    .map_err(|e| format!("invalid output.message.completed data: {}", e))?;
                convert(everruns_core::EventData::OutputMessageCompleted(d))
            }
            "tool.completed" => {
                let d: ToolCompletedData = serde_json::from_value(data.clone())
                    .map_err(|e| format!("invalid tool.completed data: {}", e))?;
                convert(everruns_core::EventData::ToolCompleted(d))
            }
            _ => Err(format!("unexpected event type for message: {}", event_type)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domains::sessions::limits::OrgCaps;
    use crate::errors::BadRequestError;
    use crate::storage::{
        RESOLVING_TOOL_RESULTS_STATUS, StorageBackend,
        models::{CreateUserRow, UpdateSession},
    };
    use async_trait::async_trait;
    use everruns_provider::typed_id::{AgentId, HarnessId, MessageId, SessionId};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct NoopRunner;

    #[async_trait]
    impl AgentRunner for NoopRunner {
        async fn start_run(
            &self,
            _org_id: i64,
            _session_id: SessionId,
            _harness_id: HarnessId,
            _agent_id: Option<AgentId>,
            _input_message_id: MessageId,
            _request_id: Option<String>,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn resume_after_tool_results(
            &self,
            _session_id: SessionId,
            _resolution_id: uuid::Uuid,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn cancel_run(&self, _session_id: SessionId) -> anyhow::Result<()> {
            Ok(())
        }

        async fn is_running(&self, _session_id: SessionId) -> bool {
            false
        }

        async fn active_count(&self) -> usize {
            0
        }
    }

    struct FailOnceResumeRunner {
        calls: AtomicUsize,
    }

    #[async_trait]
    impl AgentRunner for FailOnceResumeRunner {
        async fn start_run(
            &self,
            _org_id: i64,
            _session_id: SessionId,
            _harness_id: HarnessId,
            _agent_id: Option<AgentId>,
            _input_message_id: MessageId,
            _request_id: Option<String>,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn resume_after_tool_results(
            &self,
            _session_id: SessionId,
            _resolution_id: uuid::Uuid,
        ) -> anyhow::Result<()> {
            if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
                anyhow::bail!("durable resume enqueue failed");
            }
            Ok(())
        }

        async fn cancel_run(&self, _session_id: SessionId) -> anyhow::Result<()> {
            Ok(())
        }

        async fn is_running(&self, _session_id: SessionId) -> bool {
            false
        }

        async fn active_count(&self) -> usize {
            0
        }
    }

    async fn create_test_session(
        db: &StorageBackend,
        org_id: i64,
    ) -> crate::storage::models::SessionRow {
        db.create_session(crate::storage::models::CreateSessionRow {
            source: everruns_platform::SessionSource::Api,
            workspace_id: None,
            org_id,
            harness_id: None,
            app_id: None,
            endpoint_id: None,
            agent_id: None,
            agent_version_id: None,
            agent_config_hash: None,
            agent_identity_id: None,
            owner_principal_id: everruns_provider::typed_id::PrincipalId::from_seed(org_id as u128),
            resolved_owner_user_id: None,
            title: None,
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
            mcp_servers: serde_json::json!({}),
            system_prompt: None,
            initial_files: serde_json::json!([]),
            hints: None,
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
            blueprint_id: None,
            blueprint_config: None,
            parent_session_id: None,
            budget_root_session_id: None,
        })
        .await
        .unwrap()
    }

    async fn park_test_session(db: &StorageBackend, session: &crate::storage::models::SessionRow) {
        db.update_session(
            session.org_id,
            session.id,
            UpdateSession {
                status: Some("waiting_for_tool_results".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        db.create_event(crate::storage::models::CreateEventRow {
            session_id: session.id,
            event_type: "tool.call_requested".to_string(),
            ts: Utc::now(),
            context: serde_json::json!({}),
            data: serde_json::json!({
                "tool_calls": [{
                    "id": "call_parked",
                    "name": "ask_user",
                    "arguments": {}
                }]
            }),
            metadata: None,
            tags: None,
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn expired_waiting_turn_claim_recovers_persisted_plan_and_fences_old_owner() {
        let db = StorageBackend::in_memory();
        let session = create_test_session(&db, 1).await;
        park_test_session(&db, &session).await;
        let original_plan = WaitingTurnResolutionPlan {
            kind: "original".to_string(),
            events: Vec::new(),
            session_values: Vec::new(),
            response: serde_json::json!({ "winner": "original" }),
        };
        let first = match db
            .claim_waiting_turn(1, session.id, original_plan.clone())
            .await
            .unwrap()
        {
            crate::storage::models::ClaimWaitingTurnResult::Claimed(claim) => claim,
            other => panic!("expected initial claim, got {other:?}"),
        };
        db.abandon_waiting_turn_claim(1, session.id, first.resolution_id, first.claim_token)
            .await
            .unwrap();

        let replacement_plan = WaitingTurnResolutionPlan {
            kind: "replacement".to_string(),
            events: Vec::new(),
            session_values: Vec::new(),
            response: serde_json::json!({ "winner": "replacement" }),
        };
        let recovered = match db
            .claim_waiting_turn(1, session.id, replacement_plan)
            .await
            .unwrap()
        {
            crate::storage::models::ClaimWaitingTurnResult::Claimed(claim) => claim,
            other => panic!("expected recovered claim, got {other:?}"),
        };

        assert!(recovered.recovered);
        assert_eq!(recovered.resolution_id, first.resolution_id);
        assert_ne!(recovered.claim_token, first.claim_token);
        assert_eq!(recovered.plan, original_plan);
        assert!(
            !db.complete_waiting_turn_claim(1, session.id, first.resolution_id, first.claim_token,)
                .await
                .unwrap()
        );
        assert!(
            db.complete_waiting_turn_claim(
                1,
                session.id,
                recovered.resolution_id,
                recovered.claim_token,
            )
            .await
            .unwrap()
        );
    }

    #[tokio::test]
    async fn active_turn_cap_enforced() {
        let db = Arc::new(StorageBackend::in_memory());
        let runner: Arc<dyn AgentRunner> = Arc::new(NoopRunner);
        let delivery = crate::event_delivery::EventDelivery::in_memory();

        let svc = MessageService::new(db.clone(), runner, false, delivery).with_caps(OrgCaps {
            max_concurrent_sessions: 10_000,
            max_active_turns: 1,
        });

        // Seed an 'active' session so count_active_turns_for_org returns 1.
        // max_active_turns = 1 so 1 active turn exactly hits the cap.
        let session = create_test_session(&db, 1).await;
        db.update_session(
            1,
            session.id,
            UpdateSession {
                status: Some("active".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let ctx = CreateMessageContext {
            org_id: 1,
            user_id: None,
            harness_id: session.id.uuid(),
            agent_id: None,
            session_id: session.id.uuid(),
            event_metadata: None,
            request_id: None,
        };

        let err = svc
            .create(ctx, CreateMessageRequest::user("hello"))
            .await
            .unwrap_err();
        assert!(
            err.downcast_ref::<BadRequestError>().is_some(),
            "expected BadRequestError, got: {err}"
        );
        assert!(
            err.to_string().contains("Too many active turns"),
            "got: {err}"
        );
    }

    #[tokio::test]
    async fn parked_turn_resumes_at_new_turn_capacity() {
        let db = Arc::new(StorageBackend::in_memory());
        let runner: Arc<dyn AgentRunner> = Arc::new(NoopRunner);
        let delivery = crate::event_delivery::EventDelivery::in_memory();
        let svc = MessageService::new(db.clone(), runner, false, delivery).with_caps(OrgCaps {
            max_concurrent_sessions: 10_000,
            max_active_turns: 1,
        });
        let active = create_test_session(&db, 1).await;
        db.update_session(
            1,
            active.id,
            UpdateSession {
                status: Some("active".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let parked = create_test_session(&db, 1).await;
        park_test_session(&db, &parked).await;

        let message = svc
            .create(
                CreateMessageContext {
                    org_id: 1,
                    user_id: None,
                    harness_id: parked.id.uuid(),
                    agent_id: None,
                    session_id: parked.id.uuid(),
                    event_metadata: None,
                    request_id: None,
                },
                CreateMessageRequest::user("typed answer"),
            )
            .await
            .unwrap();

        assert_eq!(message.session_id, parked.id);
        assert_eq!(
            db.get_session(1, parked.id).await.unwrap().unwrap().status,
            "active"
        );
    }

    #[tokio::test]
    async fn parked_turn_event_write_failure_preserves_plan_for_retry() {
        let db = Arc::new(StorageBackend::in_memory());
        let svc = MessageService::new(
            db.clone(),
            Arc::new(NoopRunner),
            false,
            crate::event_delivery::EventDelivery::in_memory(),
        );
        let session = create_test_session(&db, 1).await;
        park_test_session(&db, &session).await;
        db.force_storage_failure("create_event");

        let error = svc
            .create(
                CreateMessageContext {
                    org_id: 1,
                    user_id: None,
                    harness_id: session.id.uuid(),
                    agent_id: None,
                    session_id: session.id.uuid(),
                    event_metadata: None,
                    request_id: None,
                },
                CreateMessageRequest::user("typed answer"),
            )
            .await
            .expect_err("event write must fail");

        assert!(error.to_string().contains("relation"));
        assert_eq!(
            db.get_session(1, session.id).await.unwrap().unwrap().status,
            RESOLVING_TOOL_RESULTS_STATUS
        );

        let recovered = svc
            .create(
                CreateMessageContext {
                    org_id: 1,
                    user_id: None,
                    harness_id: session.id.uuid(),
                    agent_id: None,
                    session_id: session.id.uuid(),
                    event_metadata: None,
                    request_id: None,
                },
                CreateMessageRequest::user("replacement must not win"),
            )
            .await
            .expect("expired resolution claim should recover");
        assert_eq!(
            db.get_session(1, session.id).await.unwrap().unwrap().status,
            "active"
        );
        let input_events = db
            .list_events(
                session.id,
                None,
                None,
                &["input.message".to_string()],
                &[],
                None,
                None,
            )
            .await
            .unwrap();
        assert_eq!(input_events.len(), 1);
        assert_eq!(
            input_events[0].data["message"]["content"][0]["text"],
            "typed answer"
        );
        assert_eq!(
            recovered.id.to_string(),
            input_events[0].data["message"]["id"]
        );
    }

    #[tokio::test]
    async fn parked_turn_partial_commit_retry_is_idempotent() {
        let db = Arc::new(StorageBackend::in_memory());
        let svc = MessageService::new(
            db.clone(),
            Arc::new(FailOnceResumeRunner {
                calls: AtomicUsize::new(0),
            }),
            false,
            crate::event_delivery::EventDelivery::in_memory(),
        );
        let session = create_test_session(&db, 1).await;
        park_test_session(&db, &session).await;

        let error = svc
            .create(
                CreateMessageContext {
                    org_id: 1,
                    user_id: None,
                    harness_id: session.id.uuid(),
                    agent_id: None,
                    session_id: session.id.uuid(),
                    event_metadata: None,
                    request_id: None,
                },
                CreateMessageRequest::user("typed answer"),
            )
            .await
            .expect_err("durable enqueue must fail");

        assert_eq!(error.to_string(), "durable resume enqueue failed");
        assert_eq!(
            db.get_session(1, session.id).await.unwrap().unwrap().status,
            RESOLVING_TOOL_RESULTS_STATUS
        );

        svc.create(
            CreateMessageContext {
                org_id: 1,
                user_id: None,
                harness_id: session.id.uuid(),
                agent_id: None,
                session_id: session.id.uuid(),
                event_metadata: None,
                request_id: None,
            },
            CreateMessageRequest::user("replacement must not duplicate"),
        )
        .await
        .expect("retry should finish the persisted plan");

        let events = db
            .list_events(session.id, None, None, &[], &[], None, None)
            .await
            .unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|event| event.event_type == "tool.completed")
                .count(),
            1
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| event.event_type == "input.message")
                .count(),
            1
        );
        assert!(events.iter().any(|event| {
            event.event_type == "input.message"
                && event.data["message"]["content"][0]["text"] == "typed answer"
        }));
        assert_eq!(
            db.get_session(1, session.id).await.unwrap().unwrap().status,
            "active"
        );
    }

    #[tokio::test]
    async fn create_message_without_user_id_uses_session_owner_participant_metadata() {
        let db = Arc::new(StorageBackend::in_memory());
        let runner: Arc<dyn AgentRunner> = Arc::new(NoopRunner);
        let delivery = crate::event_delivery::EventDelivery::in_memory();

        let svc = MessageService::new(db.clone(), runner, false, delivery).with_caps(OrgCaps {
            max_concurrent_sessions: 10_000,
            max_active_turns: 10_000,
        });

        let session = create_test_session(&db, 1).await;
        let owner_participant = db
            .list_session_participants(1, session.id)
            .await
            .unwrap()
            .into_iter()
            .find(|row| {
                row.kind == "user"
                    && row.principal_id == session.owner_principal_id
                    && row.left_at.is_none()
            })
            .expect("session owner user participant");

        let message = svc
            .create(
                CreateMessageContext {
                    org_id: 1,
                    user_id: None,
                    harness_id: session.id.uuid(),
                    agent_id: None,
                    session_id: session.id.uuid(),
                    event_metadata: None,
                    request_id: None,
                },
                CreateMessageRequest::user("owner provenance"),
            )
            .await
            .unwrap();
        assert_eq!(message.session_id, session.id);

        let events = db
            .list_message_events_limited(session.id, None)
            .await
            .unwrap();
        let input = events
            .into_iter()
            .find(|row| row.event_type == "input.message")
            .expect("input message event");
        let metadata = input.metadata.expect("input message metadata");
        assert_eq!(
            metadata
                .get("initiator_principal_id")
                .and_then(|value| value.as_str()),
            Some(session.owner_principal_id.to_string().as_str())
        );
        assert_eq!(
            metadata
                .get("participant_id")
                .and_then(|value| value.as_str()),
            Some(owner_participant.id.to_string().as_str())
        );
    }

    #[tokio::test]
    async fn create_message_rejoins_user_who_left_session() {
        let db = Arc::new(StorageBackend::in_memory());
        let runner: Arc<dyn AgentRunner> = Arc::new(NoopRunner);
        let delivery = crate::event_delivery::EventDelivery::in_memory();
        let svc = MessageService::new(db.clone(), runner, false, delivery).with_caps(OrgCaps {
            max_concurrent_sessions: 10_000,
            max_active_turns: 10_000,
        });

        let user = db
            .create_user(CreateUserRow {
                email: "returning-user@example.com".to_string(),
                name: "Returning User".to_string(),
                avatar_url: None,
                roles: vec!["user".to_string()],
                password_hash: None,
                email_verified: true,
                auth_provider: None,
                auth_provider_id: None,
                external_id: None,
            })
            .await
            .unwrap();
        let principal = PrincipalService::new(db.clone())
            .ensure_user_principal(1, user.id)
            .await
            .unwrap();
        let session = create_test_session(&db, 1).await;
        let original_participant = db
            .ensure_active_user_session_participant(CreateSessionParticipantRow {
                org_id: 1,
                session_id: session.id,
                kind: SessionParticipantKind::User,
                agent_id: None,
                agent_version_id: None,
                principal_id: principal.id,
                display_name: Some("Returning User".to_string()),
                role: SessionParticipantRole::Member,
                joined_at: None,
            })
            .await
            .unwrap();
        db.leave_session_participant(1, session.id, original_participant.id)
            .await
            .unwrap()
            .expect("leave initial participant");

        svc.create(
            CreateMessageContext {
                org_id: 1,
                user_id: Some(user.id),
                harness_id: session.id.uuid(),
                agent_id: None,
                session_id: session.id.uuid(),
                event_metadata: None,
                request_id: None,
            },
            CreateMessageRequest::user("I am back"),
        )
        .await
        .unwrap();

        let participants = db.list_session_participants(1, session.id).await.unwrap();
        let active_participant = participants
            .iter()
            .find(|row| row.principal_id == principal.id && row.left_at.is_none())
            .expect("returning user rejoins");
        assert_ne!(active_participant.id, original_participant.id);
        assert_eq!(active_participant.principal_id, principal.id);
        assert_eq!(
            active_participant.display_name.as_deref(),
            Some("Returning User")
        );

        let events = db
            .list_message_events_limited(session.id, None)
            .await
            .unwrap();
        let input = events
            .into_iter()
            .find(|row| row.event_type == "input.message")
            .expect("input message event");
        assert_eq!(
            input
                .metadata
                .as_ref()
                .and_then(|metadata| metadata.get("participant_id"))
                .and_then(|value| value.as_str()),
            Some(active_participant.id.to_string().as_str())
        );
    }

    #[tokio::test]
    async fn active_turn_cap_reserves_started_session_before_persisting() {
        let db = Arc::new(StorageBackend::in_memory());
        let runner: Arc<dyn AgentRunner> = Arc::new(NoopRunner);
        let delivery = crate::event_delivery::EventDelivery::in_memory();

        let svc = MessageService::new(db.clone(), runner, false, delivery).with_caps(OrgCaps {
            max_concurrent_sessions: 10_000,
            max_active_turns: 1,
        });

        let first = create_test_session(&db, 1).await;
        let second = create_test_session(&db, 1).await;

        let first_message = svc
            .create(
                CreateMessageContext {
                    org_id: 1,
                    user_id: None,
                    harness_id: first.id.uuid(),
                    agent_id: None,
                    session_id: first.id.uuid(),
                    event_metadata: None,
                    request_id: None,
                },
                CreateMessageRequest::user("first"),
            )
            .await
            .unwrap();
        assert_eq!(first_message.session_id, first.id);
        assert_eq!(db.count_active_turns_for_org(1).await.unwrap(), 1);

        let err = svc
            .create(
                CreateMessageContext {
                    org_id: 1,
                    user_id: None,
                    harness_id: second.id.uuid(),
                    agent_id: None,
                    session_id: second.id.uuid(),
                    event_metadata: None,
                    request_id: None,
                },
                CreateMessageRequest::user("second"),
            )
            .await
            .unwrap_err();
        assert!(
            err.downcast_ref::<BadRequestError>().is_some(),
            "expected BadRequestError, got: {err}"
        );
        assert!(
            err.to_string().contains("Too many active turns"),
            "got: {err}"
        );
        assert_eq!(db.count_active_turns_for_org(1).await.unwrap(), 1);
        assert!(
            db.list_message_events_limited(second.id, None)
                .await
                .unwrap()
                .is_empty(),
            "rejected turn must not persist a queued message"
        );
    }
}

// Message service for business logic
//
// Messages are stored as events in the events table. This service handles:
// - Creating user message events
// - Listing messages by querying message events
// - Workflow triggering for user messages

use crate::api::messages::{ContentPart, CreateMessageRequest, Message, MessageRole};
use crate::domains::notifications::NotificationService;
use crate::domains::sessions::limits::OrgCaps;
use crate::errors::{BadRequestError, ResourceNotFoundError};
use crate::execution_metadata;
use crate::services::{EventService, PrincipalService};
use crate::storage::StorageBackend;
use crate::storage::models::{CreateSessionParticipantRow, ReserveActiveTurnSlotResult};
use anyhow::Result;
use chrono::Utc;
use everruns_core::Event;
use everruns_core::events::{
    EventContext, EventRequest, InputMessageData, OutputMessageCompletedData, ToolCompletedData,
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

        let previous_status = match self
            .db
            .reserve_active_turn_slot_for_org(
                ctx.org_id,
                SessionId::from_uuid(ctx.session_id),
                self.caps.max_active_turns as i64,
            )
            .await?
        {
            ReserveActiveTurnSlotResult::Reserved { previous_status } => previous_status,
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

        // The slot is now reserved (session marked `active`). Run the remaining
        // work under a guard that releases the reservation if anything fails, so
        // a rejected turn never leaks active-turn capacity.
        let reservation_org_id = ctx.org_id;
        let reservation_session_id = SessionId::from_uuid(ctx.session_id);
        let result: Result<Message> = async {
            // Convert InputContentPart array to ContentPart array
            let content: Vec<ContentPart> = req
                .message
                .content
                .into_iter()
                .map(ContentPart::from)
                .collect();

            // Generate a new message ID
            let message_id = Uuid::now_v7();
            let now = Utc::now();

            // Build the core message
            let core_message = everruns_core::Message {
                id: message_id.into(),
                role: everruns_core::MessageRole::User,
                content: content.clone(),
                phase: None,
                thinking: None,
                thinking_signature: None,
                controls: req.controls.clone(),
                metadata: req.metadata.clone(),
                external_actor: req.external_actor.clone(),
                created_at: now,
            };

            // Convert to typed IDs for emit/runner
            let session_id_typed = SessionId::from_uuid(ctx.session_id);
            let harness_id_typed = HarnessId::from_uuid(ctx.harness_id);
            let agent_id_typed = ctx.agent_id.map(AgentId::from_uuid);
            let message_id_typed = MessageId::from_uuid(message_id);

            // Emit as typed event using EventService
            let event_metadata = if let Some(metadata) = ctx.event_metadata {
                Some(metadata)
            } else if let Some(user_id) = ctx.user_id {
                let principal_id = self
                    .ensure_active_user_participant(ctx.org_id, session_id_typed, user_id)
                    .await?;
                execution_metadata::interactive_user_metadata(Some(user_id), Some(principal_id))
            } else {
                self.session_owner_message_metadata(ctx.org_id, session_id_typed)
                    .await
            };
            let mut event_request = EventRequest::new(
                session_id_typed,
                EventContext::empty(),
                InputMessageData::new(core_message),
            );
            if let Some(metadata) = event_metadata {
                event_request = event_request.with_metadata(metadata);
            }
            let stored_event = self.event_service.emit(event_request).await?;

            // Construct API Message
            let message = Message {
                id: message_id_typed,
                session_id: session_id_typed,
                sequence: stored_event.sequence.unwrap_or(0),
                role: MessageRole::User,
                content,
                controls: req.controls,
                metadata: req.metadata,
                external_actor: req.external_actor,
                created_at: now,
            };

            if self.notifications_enabled
                && let Some(user_id) = ctx.user_id
            {
                self.notification_service
                    .create_turn_request(ctx.org_id, user_id, session_id_typed, message_id_typed)
                    .await?;
            }

            // Start workflow for user message in background (don't block the response)
            // The message is already persisted, so we can return immediately
            let runner = self.runner.clone();
            let request_id = ctx.request_id.clone();
            let request_id_str = request_id.as_deref().unwrap_or("").to_string();
            let session_id_str = ctx.session_id.to_string();
            let message_id_str = message_id.to_string();
            tokio::spawn(async move {
                if let Err(e) = runner
                    .start_run(
                        ctx.org_id,
                        session_id_typed,
                        harness_id_typed,
                        agent_id_typed,
                        message_id_typed,
                        request_id,
                    )
                    .await
                {
                    tracing::error!(
                        session_id = %session_id_str,
                        input_message_id = %message_id_str,
                        request_id = %request_id_str,
                        error = %e,
                        "Failed to start turn workflow"
                    );
                } else {
                    tracing::info!(
                        session_id = %session_id_str,
                        input_message_id = %message_id_str,
                        request_id = %request_id_str,
                        "Turn workflow started"
                    );
                }
            });

            Ok(message)
        }
        .await;

        if result.is_err()
            && let Err(release_err) = self
                .db
                .release_active_turn_slot_for_org(
                    reservation_org_id,
                    reservation_session_id,
                    &previous_status,
                )
                .await
        {
            tracing::warn!(
                org_id = reservation_org_id,
                session_id = %reservation_session_id,
                error = %release_err,
                "Failed to release active-turn slot after message-create failure"
            );
        }
        result
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
                            everruns_core::Message::tool_result(
                                &data.tool_call_id,
                                result,
                                data.error.clone(),
                            )
                        } else {
                            everruns_core::Message::tool_result_with_images(
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
                    content: core_message.content.clone(),
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
        StorageBackend,
        models::{CreateUserRow, UpdateSession},
    };
    use async_trait::async_trait;
    use everruns_provider::typed_id::{AgentId, HarnessId, MessageId, SessionId};

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

        async fn resume_after_tool_results(&self, _session_id: SessionId) -> anyhow::Result<()> {
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

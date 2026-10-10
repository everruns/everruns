// Message service for business logic
//
// Messages are stored as events in the events table. This service handles:
// - Creating user message events
// - Listing messages by querying message events
// - Workflow triggering for user messages

use crate::domains::messages::types::{
    CreateMessageRequest, Message, MessageDelivery, MessageRole,
};
use crate::domains::notifications::NotificationService;
use crate::domains::sessions::limits::OrgCaps;
use crate::domains::sessions::record::{SessionParticipantKind, SessionParticipantRole};
use crate::domains::tool_results::waiting_turn_resolution::execute_waiting_turn_resolution;
use crate::domains::users::PrincipalService;
use crate::errors::{BadRequestError, ConflictError, ResourceNotFoundError};
use crate::execution_metadata;
use crate::services::EventService;
use crate::storage::StorageBackend;
use crate::storage::VirtualUserRow;
use crate::storage::runtime_identity::InvocationRows;
use crate::storage::{
    AgentRow, CreateSessionParticipantRow, ReserveActiveTurnSlotResult, SessionRow,
    WaitingTurnResolutionPlan,
};
use anyhow::Result;
use chrono::Utc;
use everruns_contracts::typed_id::{
    AgentId, HarnessId, MessageId, PrincipalId, SessionId, SessionParticipantId,
};
use everruns_core::ContentPart;
use everruns_core::Event;
use everruns_core::builtins::ask_user::{ASK_USER_TOOL_NAME, AskUserStatus};
use everruns_core::events::{
    EventContext, EventData, EventRequest, InputMessageData, OutputMessageCompletedData,
    ToolCompletedData, deserialize_event_data,
};
use everruns_core::host::TurnBackend;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

pub struct MessageService {
    db: Arc<StorageBackend>,
    event_service: EventService,
    notification_service: NotificationService,
    notifications_enabled: bool,
    runner: Arc<dyn TurnBackend>,
    caps: OrgCaps,
}

pub struct CreateMessageContext {
    pub runtime_subject_principal_id: Option<PrincipalId>,
    pub org_id: i64,
    pub user_id: Option<Uuid>,
    pub harness_id: Uuid,
    pub agent_id: Option<Uuid>,
    pub session_id: Uuid,
    pub event_metadata: Option<serde_json::Value>,
    /// HTTP request ID for log correlation. Propagated to durable turn input.
    pub request_id: Option<String>,
}

/// Records the caller already loaded for this request, reused by
/// [`MessageService::create_with`]. Each is used only when it is the exact
/// record the service would otherwise read; anything else is read fresh.
#[derive(Debug, Default)]
pub struct CreateMessagePrefetch {
    /// The target session's stored row.
    pub session: Option<SessionRow>,
    /// The responder agent named by `CreateMessageContext::agent_id`.
    pub responder: Option<AgentRow>,
}

impl MessageService {
    pub fn new(
        db: Arc<StorageBackend>,
        runner: Arc<dyn TurnBackend>,
        notifications_enabled: bool,
        event_delivery: crate::live_updates::event_delivery::EventDelivery,
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
        default_subject: Option<&VirtualUserRow>,
    ) -> Result<(PrincipalId, SessionParticipantId)> {
        let principals = PrincipalService::new(self.db.clone());
        let principal = match default_subject {
            Some(row) => {
                principals
                    .ensure_default_virtual_user_principal_for(org_id, user_id, row)
                    .await?
            }
            None => {
                principals
                    .ensure_default_virtual_user_principal(org_id, user_id)
                    .await?
            }
        };
        let display_name = principal
            .metadata
            .get("name")
            .and_then(serde_json::Value::as_str)
            .map(|name| name.trim().to_string())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "User".to_string());

        let participant = self
            .db
            .ensure_active_user_session_participant(CreateSessionParticipantRow {
                org_id,
                session_id,
                kind: SessionParticipantKind::User,
                agent_id: None,
                principal_id: principal.id,
                display_name: Some(display_name),
                role: SessionParticipantRole::Member,
                joined_at: None,
            })
            .await?;

        Ok((principal.id, participant.id))
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
        self.create_with(ctx, req, CreateMessagePrefetch::default())
            .await
    }

    /// [`Self::create`] reusing records the caller already loaded for this
    /// request. Every send used to re-read the session, the responder agent
    /// and the default runtime user several times over, one database round
    /// trip each, before the turn could start.
    pub async fn create_with(
        &self,
        ctx: CreateMessageContext,
        req: CreateMessageRequest,
        prefetch: CreateMessagePrefetch,
    ) -> Result<Message> {
        self.create_inner(ctx, req, prefetch, None).await
    }

    /// [`Self::create`] for a message whose turn runs a saved script instead
    /// of the model (Tools in Shell D9). The marker is reserved metadata a
    /// client cannot set, so it is added after the client's keys are cleaned.
    pub async fn create_script_run(
        &self,
        ctx: CreateMessageContext,
        req: CreateMessageRequest,
        run: &everruns_contracts::runtime::saved_scripts::ScriptRun,
    ) -> Result<Message> {
        let marker = (
            everruns_contracts::runtime::saved_scripts::SCRIPT_RUN_METADATA_KEY.to_string(),
            serde_json::to_value(run)?,
        );
        self.create_inner(ctx, req, CreateMessagePrefetch::default(), Some(marker))
            .await
    }

    async fn create_inner(
        &self,
        ctx: CreateMessageContext,
        req: CreateMessageRequest,
        prefetch: CreateMessagePrefetch,
        platform_metadata: Option<(String, serde_json::Value)>,
    ) -> Result<Message> {
        tracing::info!(
            session_id = %ctx.session_id,
            harness_id = %ctx.harness_id,
            agent_id = ?ctx.agent_id,
            request_id = ?ctx.request_id,
            "Creating user message"
        );

        if let Some(client_message_id) = req.client_message_id
            && let Some(row) = self
                .db
                .find_input_message_by_client_id(
                    SessionId::from_uuid(ctx.session_id),
                    &client_message_id.to_string(),
                )
                .await?
        {
            tracing::info!(
                session_id = %ctx.session_id,
                %client_message_id,
                "Duplicate send; returning the stored message"
            );
            return stored_input_message(row);
        }

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
        // `default_virtual_user` is 4 round trips; resolve it once and hand
        // the row to the invocation check and the participant upsert.
        let default_subject = match (ctx.runtime_subject_principal_id, &ctx.event_metadata) {
            (None, None) => match ctx.user_id {
                Some(id) => Some((id, self.db.default_virtual_user(ctx.org_id, id).await?)),
                None => None,
            },
            _ => None,
        };
        let runtime_subject = if let Some(principal_id) = ctx.runtime_subject_principal_id {
            let p = self
                .db
                .get_principal(ctx.org_id, principal_id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("Runtime principal not found"))?;
            if p.kind != "virtual_user" || p.status != "active" {
                anyhow::bail!("Invalid runtime principal");
            }
            p.subject_id
                .map(everruns_contracts::typed_id::VirtualUserId::from_uuid)
        } else if ctx.event_metadata.is_none() {
            default_subject.as_ref().map(|(_, row)| row.id)
        } else {
            None
        };
        // Other ingress adapters cannot retarget a shared Playground session or
        // bypass the source-specific command policy and feature gate.
        let playground_user_id = match prefetch.session {
            Some(session) if session.id == session_id && session.org_id == ctx.org_id => {
                session.playground_user_id
            }
            _ => self
                .db
                .get_session(ctx.org_id, session_id)
                .await?
                .and_then(|session| session.playground_user_id),
        };
        if let Some(subject) = playground_user_id
            && (runtime_subject != Some(subject)
                || ctx
                    .event_metadata
                    .as_ref()
                    .and_then(|m| m.get("type"))
                    .and_then(|v| v.as_str())
                    != Some("playground"))
        {
            anyhow::bail!("Playground messages must use the authorised Playground command path");
        }
        let responder_row = if let Some(id) = ctx.agent_id {
            let public = everruns_contracts::typed_id::AgentId::from_uuid(id).to_string();
            let prefetched = prefetch
                .responder
                .filter(|row| row.org_id == ctx.org_id && row.public_id == public);
            let row = match prefetched {
                Some(row) => Some(row),
                None => match self.db.get_agent_by_public_id(ctx.org_id, &public).await? {
                    Some(row) => Some(row),
                    None => self.db.get_agent(ctx.org_id, id.into()).await?,
                },
            };
            Some(
                row.ok_or_else(|| anyhow::anyhow!("Responder not available in this organization"))?,
            )
        } else {
            None
        };
        let responder = responder_row.as_ref().map(|row| row.id.uuid());
        self.db
            .record_runtime_invocation_with(
                ctx.org_id,
                session_id,
                message_id,
                runtime_subject,
                if ctx.event_metadata.is_none() {
                    ctx.user_id
                } else {
                    None
                },
                responder,
                InvocationRows {
                    default_subject: default_subject.as_ref().map(|(user, row)| (*user, row)),
                    responder: responder_row.as_ref(),
                },
            )
            .await?;

        // Platform origin keys (task wake-ups) are reserved: a client cannot
        // dress its own text up as a platform notice.
        let mut metadata = req.metadata.clone();
        everruns_core::message::strip_reserved_message_metadata(&mut metadata);
        if let Some((key, value)) = platform_metadata {
            metadata.get_or_insert_default().insert(key, value);
        }
        if let Some(client_message_id) = req.client_message_id {
            metadata.get_or_insert_default().insert(
                everruns_core::message::CLIENT_MESSAGE_ID_METADATA_KEY.to_string(),
                serde_json::Value::String(client_message_id.to_string()),
            );
        }
        let core_message = everruns_core::RuntimeMessage {
            id: message_id_typed,
            role: everruns_core::RuntimeMessageRole::User,
            content: content.clone(),
            phase: None,
            phase_source: None,
            controls: req.controls.clone(),
            metadata,
            external_actor: req.external_actor.clone(),
            created_at: now,
        };
        let event_metadata = if let Some(principal_id) = ctx.runtime_subject_principal_id {
            let display_name = match runtime_subject {
                Some(id) => self
                    .db
                    .get_virtual_user(ctx.org_id, id)
                    .await?
                    .map(|v| v.name),
                None => None,
            };
            self.db
                .ensure_active_user_session_participant(CreateSessionParticipantRow {
                    org_id: ctx.org_id,
                    session_id,
                    kind: SessionParticipantKind::User,
                    agent_id: None,
                    principal_id,
                    display_name,
                    role: SessionParticipantRole::Member,
                    joined_at: None,
                })
                .await?;
            Some(
                serde_json::json!({"type":"virtual_user","principal_id":principal_id,"virtual_user_id":runtime_subject,"source":ctx.event_metadata}),
            )
        } else if let Some(metadata) = ctx.event_metadata.clone() {
            Some(metadata)
        } else if let Some(user_id) = ctx.user_id {
            let (principal_id, participant_id) = self
                .ensure_active_user_participant(
                    ctx.org_id,
                    session_id,
                    user_id,
                    default_subject.as_ref().map(|(_, row)| row),
                )
                .await?;
            // Carrying the participant id spares event preparation from
            // re-reading the session and its participants to find it.
            execution_metadata::interactive_user_metadata(Some(user_id), Some(principal_id)).map(
                |mut metadata| {
                    if let Some(object) = metadata.as_object_mut() {
                        object.insert(
                            "participant_id".to_string(),
                            serde_json::Value::String(participant_id.to_string()),
                        );
                    }
                    metadata
                },
            )
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
                metrics::counter!(crate::metrics_names::ORG_ACTIVE_TURN_CAP_REJECTIONS_TOTAL)
                    .increment(1);
                tracing::warn!(
                    org_id = ctx.org_id,
                    active_turns,
                    limit = self.caps.max_active_turns,
                    "org active-turn cap reached; message refused"
                );
                crate::domains::health_issues::active_turns::record_limit_reached(
                    self.db.clone(),
                    ctx.org_id,
                );
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
        let delivery = if resolution_claim.is_some() {
            MessageDelivery::Resumed
        } else if previous_status == "active" {
            MessageDelivery::Steered
        } else {
            MessageDelivery::Started
        };
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
                let harness_id = HarnessId::from_uuid(ctx.harness_id);
                let agent_id = ctx.agent_id.map(AgentId::from_uuid);
                let scope = crate::turns::scope(ctx.org_id, harness_id, agent_id);
                let request_id = ctx.request_id.clone();
                let request_id_log = request_id.as_deref().unwrap_or("").to_string();
                let request =
                    crate::turns::stored_message(session_id, scope, message_id_typed, request_id);
                // The turn reads the stored message, so it starts once that
                // commits.
                crate::storage::transaction::spawn_after_commit(async move {
                    if let Err(error) = crate::turns::start(&*runner, request).await {
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
                delivery: Some(delivery),
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
        let turn_id = everruns_contracts::typed_id::TurnId::from_uuid(session_id.uuid());
        let event_message_id = MessageId::from_uuid(session_id.uuid());

        Ok(requested
            .tool_calls
            .into_iter()
            .map(|tool_call| {
                let completed = if tool_call.name == ASK_USER_TOOL_NAME
                    && tool_call.arguments.get("questions").is_some()
                {
                    let result = crate::domains::tool_results::ask_user_result::build_result(
                        AskUserStatus::Cancelled,
                        Vec::new(),
                    );
                    ToolCompletedData::success(
                        tool_call.id,
                        tool_call.name,
                        vec![everruns_core::message::ContentPart::tool_result_text(
                            &serde_json::to_value(result).unwrap_or_default(),
                        )],
                        None,
                    )
                } else {
                    ToolCompletedData::failure(
                        tool_call.id,
                        tool_call.name,
                        "cancelled".to_string(),
                        "Cancelled because the user sent a message instead".to_string(),
                        None,
                    )
                };
                EventRequest::new(
                    session_id,
                    EventContext::turn(turn_id, event_message_id),
                    completed,
                )
            })
            .collect())
    }

    /// Access the registered durable runner. Used by sibling callers that
    /// need to cancel an in-flight workflow without going through the
    /// session command surface.
    pub fn runner(&self) -> &Arc<dyn TurnBackend> {
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
                        let mut images: Vec<everruns_contracts::tool_types::ToolResultImage> =
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
                                                everruns_contracts::tool_types::ToolResultImage {
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
                            delivery: None,
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
                    delivery: None,
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

/// The stored user message a duplicate send resolves to.
fn stored_input_message(row: crate::storage::EventRow) -> Result<Message> {
    let data: InputMessageData = serde_json::from_value(row.data)?;
    let message = data.message;
    Ok(Message {
        id: message.id,
        session_id: row.session_id,
        sequence: row.sequence,
        role: MessageRole::User,
        content: message.content,
        phase: None,
        phase_source: None,
        controls: message.controls,
        metadata: message.metadata,
        external_actor: message.external_actor,
        delivery: Some(MessageDelivery::Duplicate),
        created_at: message.created_at,
    })
}

#[cfg(test)]
mod tests;

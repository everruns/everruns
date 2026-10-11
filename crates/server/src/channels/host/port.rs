//! The channel host's session port over the server's services.
//!
//! Decisions:
//! - Sessions are created with `SessionService::create_from_app` and the
//!   channel's tenant, agent and owner from the host scope, so they carry the
//!   same provenance (`sessions.channel_id`, owner, source) as before the host.
//! - A request's identified end user (`runtime_owner`) owns the sessions
//!   it creates and is the subject its turns run as, as AG-UI and Public Chat
//!   visitors were before the host.
//! - Errors are the services' own, wrapped as `ChannelError::Other`, so a
//!   handler classifies a budget refusal or a missing session as before
//!   ([`super::channel_command_error`]).
//! - Messages go through `MessageService::create`, the path every surface
//!   uses: budgets, participants, steering and audit apply unchanged. Its
//!   `delivery` says whether a turn started; a resumed or duplicate message
//!   reports `Steered`, since no new turn owes a reply.
//! - Events are the live `EventDelivery` subscription, preceded by a durable
//!   replay after the caller's sequence, the way the session SSE route
//!   follows a session. Live events the replay already returned are skipped.
//! - On a channel whose speakers join its sessions (Slack), each sender
//!   becomes a session participant the turn runs as, so their grants and the
//!   approval audit name them, and the session's thread context accumulates
//!   who spoke. Losing the thread context costs a line of context, never the
//!   reply, so its failures only log.
//! - History seeded into a new session is written as `input.message` events
//!   that start no turn. A message marked [`HISTORY_NOTICE`] is a note about
//!   the history (it was cut short), written as a system message.
//! - A platform rename writes the title through the same no-op-suppressing
//!   event the agent's retitle uses, so the two directions settle instead of
//!   echoing.

use std::collections::VecDeque;
use std::sync::Arc;

use async_trait::async_trait;
use everruns_contracts::runtime::channel::{ChannelError, ControlAction};
use everruns_contracts::runtime::message::{ContentPart, InputContentPart};
use everruns_contracts::runtime::message_retriever::InputMessage;
use everruns_contracts::typed_id::SessionId;
use everruns_core::channel_runtime::{
    ChannelConfig, ChannelEventStream, ChannelSessionPort, DeliveryEvent, NewChannelSession,
    SendOutcome,
};
use futures::StreamExt;

use everruns_contracts::runtime::channel::{
    ChannelViewContext, ExternalActor, THREAD_CONTEXT_KV_KEY, ThreadContext, decode_thread_context,
    encode_thread_context,
};
use everruns_contracts::typed_id::MessageId;
use everruns_core::events::{EventContext, EventRequest, InputMessageData};
use everruns_core::host::session_services::capabilities::session::session_title_updated_event;
use everruns_core::{RuntimeMessage, RuntimeMessageRole};
use serde_json::Value;

use super::ServerChannelScope;
use crate::domains::agent_channels::record::ChannelType;
use crate::domains::messages::types::{
    CreateMessageRequest, InputMessage as ServerInputMessage, MessageDelivery, MessageRole,
};
use crate::domains::messages::{CreateMessageContext, MessageService};
use crate::domains::sessions::SessionService;
use crate::domains::sessions::record::SessionSource;
use crate::domains::sessions::record::{SessionParticipantKind, SessionParticipantRole};
use crate::domains::sessions::types::CreateSessionRequest;
use crate::domains::users::PrincipalService;
use crate::execution_metadata;
use crate::storage::{CreateSessionParticipantRow, SessionParticipantRow, UpsertSessionKeyValue};
use crate::storage::{ListEventsParams, StorageBackend, UpdateSession};

/// Durable events read per replay page.
const REPLAY_PAGE: i32 = 500;

/// Message metadata marking a seeded message as a note about the history
/// rather than something a person said.
pub const HISTORY_NOTICE: &str = "history_notice";

/// `ControlAction::Context` key for where the user is looking: the value is
/// `{"thread_ref", "platform", "view": ChannelViewContext}`.
pub const THREAD_VIEW_CONTEXT: &str = "thread_view";

/// [`ChannelSessionPort`] over the server's session, message and event
/// services.
#[derive(Clone)]
pub struct ServerChannelPort {
    db: Arc<StorageBackend>,
    sessions: Arc<SessionService>,
    messages: Arc<MessageService>,
}

impl ServerChannelPort {
    pub fn new(
        db: Arc<StorageBackend>,
        sessions: Arc<SessionService>,
        messages: Arc<MessageService>,
    ) -> Self {
        Self {
            db,
            sessions,
            messages,
        }
    }
}

fn scope(channel: &ChannelConfig) -> Result<&ServerChannelScope, ChannelError> {
    channel.scope.get::<ServerChannelScope>().ok_or_else(|| {
        ChannelError::Other(anyhow::anyhow!(
            "channel {} has no server scope",
            channel.name
        ))
    })
}

fn session_id(session_id: &str) -> Result<SessionId, ChannelError> {
    session_id
        .parse()
        .map_err(|_| ChannelError::BadRequest(format!("bad session id {session_id}")))
}

/// The origin recorded on a channel's sessions.
fn session_source(channel_type: &ChannelType) -> SessionSource {
    match channel_type {
        ChannelType::Slack => SessionSource::Slack,
        ChannelType::AgUi | ChannelType::PublicChat => SessionSource::AgUi,
        ChannelType::Fcp => SessionSource::Fcp,
        ChannelType::A2a => SessionSource::A2a,
        ChannelType::Schedule => SessionSource::Schedule,
        ChannelType::Webhook => SessionSource::Webhook,
        _ => SessionSource::Api,
    }
}

/// The parts a server message takes; tool calls and reasoning never come
/// from a channel.
fn input_parts(content: Vec<ContentPart>) -> Vec<InputContentPart> {
    content
        .into_iter()
        .filter_map(|part| match part {
            ContentPart::Text(text) => Some(InputContentPart::Text(text)),
            ContentPart::Image(image) => Some(InputContentPart::Image(image)),
            ContentPart::ImageFile(file) => Some(InputContentPart::ImageFile(file)),
            ContentPart::File(file) => Some(InputContentPart::File(file)),
            _ => None,
        })
        .collect()
}

#[async_trait]
impl ChannelSessionPort for ServerChannelPort {
    async fn create_session(&self, request: NewChannelSession) -> Result<String, ChannelError> {
        let ServerChannelScope {
            context,
            channel,
            runtime_owner,
            session_tags,
            session_hints,
            ..
        } = scope(&request.channel)?;
        // An identified end user owns their sessions; otherwise the channel's
        // owner does.
        let (owner, resolved_owner) = match runtime_owner {
            Some(principal) => (*principal, None),
            None => (context.owner_principal_id, context.resolved_owner_user_id),
        };
        let session = self
            .sessions
            .create_from_app(
                &everruns_core::Caller::internal(context.org_id),
                context.harness_id.uuid(),
                Some(context.agent_internal_id),
                context.agent_id,
                context.historical_app_id,
                Some(channel.internal_id),
                None,
                owner,
                resolved_owner,
                session_source(&channel.channel_type),
                CreateSessionRequest {
                    agent_id: context.agent_id,
                    harness_id: Some(context.harness_id),
                    virtual_user_id: context.virtual_user_id,
                    title: Some(request.title),
                    tags: [
                        format!("app:{}", context.public_id),
                        format!("app_channel:{}", channel.public_id),
                        format!("app_channel_type:{}", channel.channel_type),
                    ]
                    .into_iter()
                    .chain(session_tags.iter().cloned())
                    .collect(),
                    hints: session_hints.clone(),
                    ..CreateSessionRequest::default()
                },
            )
            .await
            .map_err(ChannelError::Other)?;
        Ok(session.id.to_string())
    }

    async fn send(
        &self,
        channel: &ChannelConfig,
        session: &str,
        message: InputMessage,
    ) -> Result<SendOutcome, ChannelError> {
        let ServerChannelScope {
            context,
            runtime_owner,
            runtime_subject,
            request_id,
            participant_thread_key,
            ..
        } = scope(channel)?;
        let session = session_id(session)?;
        let mut message = message;
        let mut subject = runtime_subject.or(*runtime_owner);
        let mut event_metadata = execution_metadata::channel_message_metadata(
            context.public_id,
            context.owner_principal_id,
            context.virtual_user_id,
        );
        if let (Some(thread_key), Some(actor)) =
            (participant_thread_key, message.external_actor.clone())
        {
            let participant = self.join(context.org_id, session, &actor).await?;
            subject = Some(participant.principal_id);
            let id = Value::String(participant.id.to_string());
            if let Some(map) = event_metadata.as_object_mut() {
                map.insert("participant_id".into(), id.clone());
            }
            let thread = message
                .metadata
                .as_ref()
                .and_then(|metadata| metadata.get(*thread_key))
                .and_then(Value::as_str)
                .map(str::to_string);
            message = message.with_metadata("participant_id", id);
            if let Some(thread) = thread {
                self.track_speaker(session, &thread, &actor).await;
            }
        }
        let tags = (!message.tags.is_empty()).then_some(message.tags);
        let created = self
            .messages
            .create(
                CreateMessageContext {
                    runtime_subject_principal_id: subject,
                    org_id: context.org_id,
                    user_id: None,
                    harness_id: context.harness_id.uuid(),
                    agent_id: Some(context.agent_internal_id),
                    session_id: session.uuid(),
                    event_metadata: Some(event_metadata),
                    request_id: request_id.clone(),
                },
                CreateMessageRequest {
                    message: ServerInputMessage {
                        role: MessageRole::User,
                        content: input_parts(message.content),
                    },
                    addressed_participant_id: None,
                    controls: message.controls,
                    metadata: message.metadata,
                    tags,
                    external_actor: message.external_actor,
                    client_message_id: None,
                },
            )
            .await
            .map_err(ChannelError::Other)?;
        Ok(match created.delivery {
            Some(MessageDelivery::Started) | None => SendOutcome::Started {
                input_message_id: created.id.to_string(),
            },
            Some(
                MessageDelivery::Steered | MessageDelivery::Resumed | MessageDelivery::Duplicate,
            ) => SendOutcome::Steered,
        })
    }

    async fn events(
        &self,
        _channel: &ChannelConfig,
        session: &str,
        after: Option<i64>,
    ) -> Result<ChannelEventStream, ChannelError> {
        let session = session_id(session)?;
        // Subscribe before replaying, so nothing written in between is lost.
        let subscription = self
            .messages
            .event_service()
            .event_delivery()
            .subscribe(session.uuid())
            .await?;
        let mut replay = VecDeque::new();
        let mut last = after;
        if let Some(after) = after {
            let mut cursor = i32::try_from(after).unwrap_or(i32::MAX);
            loop {
                let page = self
                    .messages
                    .event_service()
                    .list_advanced(&ListEventsParams {
                        session_id: session,
                        after_sequence: Some(cursor),
                        limit: Some(REPLAY_PAGE),
                        ..ListEventsParams::default()
                    })
                    .await?;
                let full = page.len() == REPLAY_PAGE as usize;
                for event in page {
                    if let Some(sequence) = event.sequence {
                        cursor = cursor.max(sequence);
                        last = Some(i64::from(cursor));
                    }
                    replay.extend(delivery_event(&event));
                }
                if !full {
                    break;
                }
            }
        }
        let live = futures::stream::unfold(subscription, move |mut subscription| async move {
            loop {
                let event = subscription.recv().await?;
                // In-memory delivery shares a partition between sessions.
                if event.session_id != session {
                    continue;
                }
                let replayed = matches!(
                    (event.sequence, last),
                    (Some(sequence), Some(last)) if i64::from(sequence) <= last
                );
                if replayed {
                    continue;
                }
                if let Some(event) = delivery_event(&event) {
                    return Some((event, subscription));
                }
            }
        });
        Ok(futures::stream::iter(replay).chain(live).boxed())
    }

    async fn discard_session(
        &self,
        channel: &ChannelConfig,
        session: &str,
    ) -> Result<(), ChannelError> {
        let ServerChannelScope { context, .. } = scope(channel)?;
        self.db
            .delete_session(context.org_id, session_id(session)?)
            .await?;
        Ok(())
    }

    async fn control(
        &self,
        channel: &ChannelConfig,
        session: &str,
        action: &ControlAction,
    ) -> Result<(), ChannelError> {
        let ServerChannelScope { context, .. } = scope(channel)?;
        let session = session_id(session)?;
        match action {
            ControlAction::Cancel => {
                crate::api::channel_api::cancel_session_turn_for(
                    &self.db,
                    &self.messages,
                    session,
                    "channel cancel",
                )
                .await?;
            }
            ControlAction::Rename { title } => self.rename(context.org_id, session, title).await?,
            ControlAction::Context { key, value } if key == THREAD_VIEW_CONTEXT => {
                self.set_view(session, value).await?
            }
            ControlAction::Context { key, .. } => {
                tracing::debug!(%session, %key, "channel host: unknown context update");
            }
        }
        Ok(())
    }

    async fn seed(
        &self,
        _channel: &ChannelConfig,
        session: &str,
        history: Vec<InputMessage>,
    ) -> Result<(), ChannelError> {
        let session = session_id(session)?;
        for message in history {
            let mut metadata = message.metadata.unwrap_or_default();
            let role = match metadata.remove(HISTORY_NOTICE) {
                Some(_) => RuntimeMessageRole::System,
                None => RuntimeMessageRole::User,
            };
            let message = RuntimeMessage {
                id: MessageId::new(),
                role,
                content: message.content,
                phase: None,
                phase_source: None,
                controls: None,
                metadata: (!metadata.is_empty()).then_some(metadata),
                external_actor: message.external_actor,
                created_at: chrono::Utc::now(),
            };
            self.messages
                .event_service()
                .emit(EventRequest::new(
                    session,
                    EventContext::empty(),
                    InputMessageData::new(message),
                ))
                .await?;
        }
        Ok(())
    }
}

impl ServerChannelPort {
    /// The sender as an active participant of the session.
    async fn join(
        &self,
        org_id: i64,
        session: SessionId,
        actor: &ExternalActor,
    ) -> Result<SessionParticipantRow, ChannelError> {
        let principal = PrincipalService::new(self.db.clone())
            .ensure_external_actor_principal(org_id, actor)
            .await?;
        Ok(self
            .db
            .ensure_active_user_session_participant(CreateSessionParticipantRow {
                org_id,
                session_id: session,
                kind: SessionParticipantKind::User,
                agent_id: None,
                principal_id: principal.id,
                display_name: Some(actor.display_label().to_string()),
                role: SessionParticipantRole::Member,
                joined_at: None,
            })
            .await?)
    }

    /// Add the sender to the session's thread context.
    async fn track_speaker(&self, session: SessionId, thread: &str, actor: &ExternalActor) {
        let mut thread_context = self
            .thread_context(session)
            .await
            .unwrap_or_else(|| ThreadContext::new(thread.to_string(), actor.source.clone()));
        if let Some(channel) = actor
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.get("channel"))
        {
            thread_context
                .platform_metadata
                .insert("channel_id".into(), channel.clone());
        }
        thread_context.track_participant(actor);
        if let Err(error) = self.save_thread_context(session, &thread_context).await {
            tracing::warn!(%session, %error, "channel host: could not keep a thread's participants");
        }
    }

    /// Record where the user is looking. A re-report of the same place
    /// writes nothing.
    async fn set_view(&self, session: SessionId, value: &Value) -> Result<(), ChannelError> {
        let view: ChannelViewContext =
            serde_json::from_value(value["view"].clone()).unwrap_or_default();
        let mut thread_context = match self.thread_context(session).await {
            Some(thread_context) => thread_context,
            None => ThreadContext::new(
                value["thread_ref"]
                    .as_str()
                    .unwrap_or("unknown")
                    .to_string(),
                value["platform"].as_str().unwrap_or_default().to_string(),
            ),
        };
        if thread_context.set_current_view(view) {
            self.save_thread_context(session, &thread_context).await?;
        }
        Ok(())
    }

    async fn thread_context(&self, session: SessionId) -> Option<ThreadContext> {
        match self
            .db
            .get_session_key_value(session.uuid(), THREAD_CONTEXT_KV_KEY)
            .await
        {
            Ok(row) => row.and_then(|row| decode_thread_context(&row.value)),
            Err(error) => {
                tracing::warn!(%session, %error, "channel host: could not read a thread context");
                None
            }
        }
    }

    async fn save_thread_context(
        &self,
        session: SessionId,
        thread_context: &ThreadContext,
    ) -> Result<(), ChannelError> {
        self.db
            .upsert_session_key_value(UpsertSessionKeyValue {
                session_id: session,
                key: THREAD_CONTEXT_KV_KEY.to_string(),
                value: encode_thread_context(thread_context)
                    .map_err(|error| ChannelError::Other(anyhow::anyhow!(error.to_string())))?,
            })
            .await?;
        Ok(())
    }

    /// Rename the session, unless it already has `title`.
    async fn rename(
        &self,
        org_id: i64,
        session: SessionId,
        title: &str,
    ) -> Result<(), ChannelError> {
        let Some(row) = self.db.get_session(org_id, session).await? else {
            return Ok(());
        };
        let Some(event) = session_title_updated_event(
            session,
            EventContext::empty(),
            row.title.clone(),
            title.to_string(),
        ) else {
            return Ok(());
        };
        self.db
            .update_session(
                org_id,
                session,
                UpdateSession {
                    title: Some(title.to_string()),
                    ..UpdateSession::default()
                },
            )
            .await?;
        self.messages.event_service().emit(event).await?;
        Ok(())
    }
}

fn delivery_event(event: &everruns_core::Event) -> Option<DeliveryEvent> {
    let envelope = serde_json::to_value(event).ok()?;
    DeliveryEvent::from_envelope(&envelope)
}

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

use super::ServerChannelScope;
use crate::domains::agent_channels::record::ChannelType;
use crate::domains::messages::types::{
    CreateMessageRequest, InputMessage as ServerInputMessage, MessageDelivery, MessageRole,
};
use crate::domains::messages::{CreateMessageContext, MessageService};
use crate::domains::sessions::SessionService;
use crate::domains::sessions::record::SessionSource;
use crate::domains::sessions::types::CreateSessionRequest;
use crate::execution_metadata;
use crate::storage::{ListEventsParams, StorageBackend, UpdateSession};

/// Durable events read per replay page.
const REPLAY_PAGE: i32 = 500;

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
            ..
        } = scope(channel)?;
        let session = session_id(session)?;
        let tags = (!message.tags.is_empty()).then_some(message.tags);
        let created = self
            .messages
            .create(
                CreateMessageContext {
                    runtime_subject_principal_id: runtime_subject.or(*runtime_owner),
                    org_id: context.org_id,
                    user_id: None,
                    harness_id: context.harness_id.uuid(),
                    agent_id: Some(context.agent_internal_id),
                    session_id: session.uuid(),
                    event_metadata: Some(execution_metadata::channel_message_metadata(
                        context.public_id,
                        context.owner_principal_id,
                        context.virtual_user_id,
                    )),
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
            ControlAction::Rename { title } => {
                self.db
                    .update_session(
                        context.org_id,
                        session,
                        UpdateSession {
                            title: Some(title.clone()),
                            ..UpdateSession::default()
                        },
                    )
                    .await?;
            }
            // Context updates arrive with Slack's assistant pane, which brings
            // its own handling.
            ControlAction::Context { key, .. } => {
                tracing::debug!(%session, %key, "channel host: context update not handled");
            }
        }
        Ok(())
    }
}

fn delivery_event(event: &everruns_core::Event) -> Option<DeliveryEvent> {
    let envelope = serde_json::to_value(event).ok()?;
    DeliveryEvent::from_envelope(&envelope)
}

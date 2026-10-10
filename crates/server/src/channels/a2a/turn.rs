// A2A turns on the shared channel host.
//
// Decisions:
// - A2A is a streaming channel of the host. A `shared_session` channel's
//   callers share one conversation, bound under `a2a:shared`; every other
//   mode, and every PACT caller (whose conversation is never shared), opens a
//   session of its own. A message naming a task or context the channel owns
//   continues that session: the A2A context and task ids are session ids.
// - The message is the channel's template rendered against the request, as
//   before the host; the host sends it and streams the turn.
// - A turn that parks on an approval waits on someone other than the caller.
//   `TurnEvents` follows the session past it, from the last sequence seen,
//   so a blocking send or a stream keeps waiting as it did on the raw
//   session subscription. A parked `ask_user` question is the caller's to
//   answer and ends the wait.

use std::collections::HashMap;

use chrono::Utc;
use everruns_contracts::typed_id::{PrincipalId, SessionId};
use everruns_core::channel_runtime::{ChannelEventStream, DeliveryEvent, ResolvedChannel};
use everruns_core::events::{EventData, TOOL_CALL_REQUESTED, deserialize_event_data};
use everruns_core::message_retriever::InputMessage;
use futures::StreamExt;
use serde_json::{Value, json};

use super::{ChannelA2aState, ask_user};
use crate::channels::host::{ServerChannelScope, channel_command_error};
use crate::domains::agent_channels::invocation::{
    A2A_MESSAGE_ID_METADATA, ChannelInvocationSource, channel_invocation_message_metadata,
    emit_channel_invocation_audit_event, render_message_template,
};
use crate::domains::agent_channels::record::SessionBinding;
use crate::domains::common::CommandError;

/// Binding key of a `shared_session` channel's one conversation.
const SHARED_CONVERSATION: &str = "a2a:shared";

/// A message for an A2A channel.
#[derive(Debug, Clone)]
pub(super) struct A2aTurnRequest {
    pub legacy_app_id: String,
    pub channel_id: String,
    /// Verbatim A2A `params` object from the JSON-RPC envelope.
    pub params: Value,
    /// Concatenated text parts from `params.message.parts` (newline-joined).
    pub text: String,
    pub message_id: Option<String>,
    pub task_id: String,
    pub context_id: Option<String>,
    pub role: Option<String>,
    /// A session the caller already proved is bound to this channel (an A2A
    /// `contextId` / `taskId` from an earlier response).
    pub continue_session: Option<SessionId>,
    /// Tag naming one authenticated caller (a PACT personal agent's user).
    /// A message that continues no session opens the caller's own session,
    /// tagged with it, whatever the channel's `session_mode`.
    pub caller_tag: Option<String>,
    /// The end user the turn runs as (a PACT delegation's company user).
    pub runtime_subject: Option<PrincipalId>,
}

/// A started (or steered) A2A turn.
pub(super) struct A2aTurn {
    pub session_id: SessionId,
    pub events: TurnEvents,
}

/// Send `req` on its session, running `before_send` once the session is
/// known and before the message goes out (push configs, PACT session
/// preparation).
pub(super) async fn start<F, Fut>(
    state: &ChannelA2aState,
    req: A2aTurnRequest,
    request_id: Option<String>,
    before_send: F,
) -> Result<A2aTurn, CommandError>
where
    F: FnOnce(SessionId) -> Fut,
    Fut: std::future::Future<Output = Result<(), CommandError>>,
{
    let (ingress, channel) = crate::domains::agent_channels::ingress::resolve_channel(
        &state.db,
        state.encryption.as_ref(),
        &req.channel_id,
    )
    .await?
    .ok_or_else(|| CommandError::not_found("Channel"))?;
    if !ingress.matches_legacy_app_id(&req.legacy_app_id) {
        return Err(CommandError::not_found("Channel"));
    }
    if !channel.status.is_live() {
        return Err(CommandError::forbidden("App is not published".to_string()));
    }
    let config = channel
        .a2a_config()
        .ok_or_else(|| CommandError::bad_request("Invalid A2A channel configuration"))?;
    let template_context = json!({
        "app": {
            "id": ingress.public_id.to_string(),
            "name": ingress.name.clone(),
        },
        "channel": {
            "id": channel.public_id.to_string(),
            "type": channel.channel_type.to_string(),
        },
        "invocation": {
            "source": "a2a",
            "triggered_at": Utc::now().to_rfc3339(),
        },
        "payload": req.params,
        "a2a": {
            "text": req.text,
            "message_id": req.message_id,
            "task_id": req.task_id,
            "context_id": req.context_id,
            "role": req.role,
        },
    });
    let text = render_message_template(&config.message, &template_context);
    if text.trim().is_empty() {
        return Err(CommandError::bad_request(
            "Rendered invocation message is empty",
        ));
    }

    let mut scope = ServerChannelScope::new(ingress.clone(), channel.clone());
    scope.runtime_subject = req.runtime_subject;
    scope.session_tags.extend(req.caller_tag.clone());
    scope.request_id = request_id;
    let resolved = state
        .channels
        .resolver
        .channel_for(scope)
        .ok_or_else(|| CommandError::internal(anyhow::anyhow!("A2A is not a host channel")))?;

    let (session, created) = match req.continue_session {
        Some(session) => (session.to_string(), false),
        // A caller-bound conversation is never shared.
        None if req.caller_tag.is_some() || config.session_mode != SessionBinding::Shared => {
            let title = format!("{} a2a {}", ingress.name, Utc::now().to_rfc3339());
            let session = state
                .channels
                .host
                .open(&resolved, &title)
                .await
                .map_err(channel_command_error)?;
            (session, true)
        }
        None => {
            let conversation = state
                .channels
                .host
                .conversation(
                    &resolved,
                    SHARED_CONVERSATION,
                    &format!("{} a2a", ingress.name),
                )
                .await
                .map_err(channel_command_error)?;
            (conversation.session_id, conversation.created)
        }
    };
    let session_id: SessionId = session
        .parse()
        .map_err(|_| CommandError::internal(anyhow::anyhow!("malformed session id {session}")))?;

    before_send(session_id).await?;

    let mut metadata: HashMap<String, Value> =
        channel_invocation_message_metadata(&ingress, &channel, ChannelInvocationSource::A2a);
    // The caller's A2A `messageId` rides on the stored message, so a retried
    // message can be matched to the reply it already got (PACT §4.3).
    if let Some(id) = req.message_id {
        metadata.insert(A2A_MESSAGE_ID_METADATA.to_string(), Value::String(id));
    }
    let mut message = InputMessage::user(text);
    message.metadata = Some(metadata);
    let turn = state
        .channels
        .host
        .stream_turn(&resolved, &session, message)
        .await
        .map_err(channel_command_error)?;

    emit_channel_invocation_audit_event(
        state.db.clone(),
        &ingress,
        &channel,
        session_id,
        ChannelInvocationSource::A2a,
        created,
    );

    Ok(A2aTurn {
        session_id,
        events: TurnEvents {
            state: state.clone(),
            channel: resolved,
            session,
            events: Some(turn.events),
            last_sequence: None,
            waiting_on_others: false,
        },
    })
}

/// The events of the turn running (or next to run) on a session of the
/// channel `channel_id` names: a caller resubscribing to its task.
pub(super) async fn follow(
    state: &ChannelA2aState,
    channel_id: &str,
    session_id: SessionId,
) -> Result<TurnEvents, CommandError> {
    let (ingress, channel) = crate::domains::agent_channels::ingress::resolve_channel(
        &state.db,
        state.encryption.as_ref(),
        channel_id,
    )
    .await?
    .ok_or_else(|| CommandError::not_found("Channel"))?;
    let resolved = state
        .channels
        .resolver
        .channel(&ingress, &channel)
        .ok_or_else(|| CommandError::internal(anyhow::anyhow!("A2A is not a host channel")))?;
    let session = session_id.to_string();
    let events = state
        .channels
        .host
        .follow(&resolved, &session, None)
        .await
        .map_err(channel_command_error)?;
    Ok(TurnEvents {
        state: state.clone(),
        channel: resolved,
        session,
        events: Some(events),
        last_sequence: None,
        waiting_on_others: false,
    })
}

/// The events of an A2A turn, up to the point it settles for the caller: its
/// end, or a question the caller must answer.
pub(super) struct TurnEvents {
    state: ChannelA2aState,
    channel: ResolvedChannel,
    session: String,
    events: Option<ChannelEventStream>,
    last_sequence: Option<i64>,
    /// The host's stream ended on a park that is someone else's to answer.
    waiting_on_others: bool,
}

impl TurnEvents {
    /// The next event, typed. `None` once the turn settled for the caller or
    /// the session's events ended.
    pub(super) async fn next(&mut self) -> Option<EventData> {
        loop {
            let events = match &mut self.events {
                Some(events) => events,
                None if self.waiting_on_others => {
                    self.waiting_on_others = false;
                    match self
                        .state
                        .channels
                        .host
                        .follow(&self.channel, &self.session, self.last_sequence)
                        .await
                    {
                        Ok(events) => self.events.insert(events),
                        Err(error) => {
                            tracing::warn!(%error, "A2A: could not follow a parked turn");
                            return None;
                        }
                    }
                }
                None => return None,
            };
            let Some(event) = events.next().await else {
                self.events = None;
                continue;
            };
            self.observe(&event);
            return Some(deserialize_event_data(&event.event_type, event.data));
        }
    }

    fn observe(&mut self, event: &DeliveryEvent) {
        if let Some(sequence) = event.sequence {
            self.last_sequence = Some(sequence);
        }
        // A park on anything but `ask_user` (an approval) is answered by
        // someone else, and the turn goes on; the host's stream ends there,
        // so follow the session past it.
        self.waiting_on_others = event.event_type == TOOL_CALL_REQUESTED
            && serde_json::from_value(event.data.clone())
                .ok()
                .and_then(|requested| ask_user::pending_ask_user_from_request(&requested))
                .is_none();
    }
}

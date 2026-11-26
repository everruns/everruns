//! Reply delivery: one turn's events in, platform calls out.
//!
//! Decisions (carried over from the server's Slack dispatcher, where each was
//! learned the hard way):
//! - A delivery belongs to one turn, keyed by its input message id. Events of
//!   other turns are skipped, except `turn.cancelled`: cancellation is
//!   session-scoped and its synthetic event carries a fresh id, so it ends the
//!   delivery once this turn's boundary was seen (EVE-966).
//! - Delivery needs no mode setting; the events say how the agent talks. A
//!   direct agent's completed assistant messages are posted. An explicit
//!   agent's assistant text is working notes (marked commentary from the moment
//!   it starts), which never reach the platform; what it sends with
//!   `send_message` arrives as `conversation.message` and is posted, unless
//!   the sender already delivered it. A successful `no_reply` is a deliberate
//!   answer: no end-of-turn notice.
//! - Streaming is per output message, not per turn. Text comes from the
//!   incremental `delta`, never from `data.accumulated`, which the Framework
//!   drops; `accumulated` only heals a gap when present. The completed message
//!   is authoritative for the final text. A guardrail replacement rewrites the
//!   open stream. Every opened stream is stopped, on every exit.
//! - A message is delivered once. Its id is remembered once it completed,
//!   closed or was replaced, so a delta that arrives late (live deltas and
//!   durable events travel separately) cannot reopen a stream and post it
//!   again.
//! - Status and title are advisory: a failure is logged and the turn goes on.
//! - A `request_approval` pause is drawn by adapters that can; task progress
//!   is one message, edited on flush and pushed a last time at the end. Both
//!   count as delivered.
//! - A turn that ends having delivered nothing posts exactly one short notice
//!   with no error detail (channels are often public), plus an optional link.
//!   A failure whose code says the agent's model or provider is set up wrong
//!   says so instead, so the person writing knows retrying will not help and
//!   who has to act. Only the category shows, never the model, provider or
//!   message, and the error-disclosure mode applies: in `generic` mode the
//!   code is already `processing_error`, so the plain notice is posted.
//! - No I/O happens outside the adapter, so the same object runs in a
//!   Framework task, serve, and the server's Postgres-driven dispatcher.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use super::checkpoint::DeliveryJournal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::warn;

use super::approval::approval_prompt;
use super::delivery_telemetry::{
    DeliveryKind, DeliveryObserver, DeliveryOutcome, DeliveryTelemetry,
};
use super::progress::TaskProgress;
use super::{ChannelDeliveryAdapter, DeliveryContext, DeliveryResult, OutboundChannelMessage};
use crate::conversation::{NO_REPLY_TOOL_NAME, SEND_MESSAGE_TOOL_NAME};

/// The agent's own reaction tool (Reactions capability). Reacting is talking,
/// not work, so it does not start the 👀 acknowledgement.
const REACT_TOOL_NAME: &str = "react";
/// `effects_done` markers for the acknowledgement, so a resumed delivery
/// neither repeats nor strands a reaction.
const ACK_STARTED_DONE: &str = "ack:started";
const ACK_ANSWERED_DONE: &str = "ack:answered";
const ACK_AGENT_REACTED: &str = "ack:agent";
use everruns_contracts::runtime::events::{
    CONVERSATION_MESSAGE, MESSAGE_REACTION_ADDED, MESSAGE_REACTION_REMOVED,
    OUTPUT_MESSAGE_COMPLETED, OUTPUT_MESSAGE_DELTA, OUTPUT_MESSAGE_REPLACED,
    OUTPUT_MESSAGE_STARTED, SESSION_TITLE_UPDATED, TASK_CREATED, TASK_UPDATED, TOOL_COMPLETED,
    TOOL_STARTED, TURN_CANCELLED, TURN_COMPLETED, TURN_FAILED, TURN_SEALED, TURN_STARTED,
};
use everruns_contracts::runtime::session_task::SessionTaskState;
use everruns_contracts::typed_id::SessionId;

/// The MCP App view a `tool.completed` carries, with its text result.
fn app_view(data: &Value) -> Option<everruns_contracts::runtime::channel::ChannelAppView> {
    if data.get("error").is_some_and(|error| !error.is_null()) {
        return None;
    }
    let app = everruns_contracts::runtime::mcp_apps::McpAppCallDetails::from_details(
        data.get("details"),
    )?;
    let text = data
        .get("result")
        .and_then(Value::as_array)
        .map(|parts| {
            parts
                .iter()
                .filter(|part| part.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();
    Some(everruns_contracts::runtime::channel::ChannelAppView {
        app,
        tool_call_id: data
            .get("tool_call_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        text,
    })
}

/// Flush a stream early once this much text is waiting.
pub const STREAM_FLUSH_CHARS: usize = 2_000;

/// How a delivery behaves. Hosts fill it from the channel's configuration.
#[derive(Debug, Clone)]
pub struct DeliveryOptions {
    /// Stream replies when the adapter can. Off posts each message once.
    pub stream: bool,
    /// Show status and title when the adapter can. Off for a conversation
    /// whose surface has no status line, such as a plain channel post.
    pub agent_surface: bool,
    /// Status shown while the agent works and no tool runs.
    pub thinking_status: String,
    /// Status shown while a tool runs. `None` keeps the thinking status, which
    /// says that work happens and nothing about what.
    pub tool_status: Option<String>,
    /// Link appended to the notice for a turn that delivered nothing.
    pub session_link: Option<String>,
    /// Deliver the agent's replies. Off for a channel that only takes
    /// messages in, such as a webhook: its turns run and nothing is posted.
    pub replies: bool,
    /// Acknowledge the message being answered with reactions: 👀 once the
    /// agent starts working on it, ✅ once it has sent its answer. Hosts turn
    /// it on for explicit agents allowed to react, on platforms that show
    /// reactions. The agent's own reaction on that message takes over.
    pub acknowledge: bool,
}

/// Reaction added to the answered message when the agent starts working.
pub const ACK_STARTED: &str = "👀";
/// Reaction added to the answered message once the agent sent its answer.
pub const ACK_ANSWERED: &str = "✅";

impl Default for DeliveryOptions {
    fn default() -> Self {
        Self {
            stream: true,
            agent_surface: true,
            thinking_status: "is thinking...".to_string(),
            tool_status: None,
            session_link: None,
            replies: true,
            acknowledge: false,
        }
    }
}

#[path = "delivery_checkpoint.rs"]
mod checkpoint_state;

/// One session event, as reply delivery reads it.
#[derive(Debug, Clone, PartialEq)]
pub struct DeliveryEvent {
    /// Durable sequence; `None` for ephemeral deltas.
    pub sequence: Option<i64>,
    pub event_type: String,
    pub data: Value,
    /// The turn's input message, from the event context.
    pub input_message_id: Option<String>,
    /// The turn, from the event context.
    pub turn_id: Option<String>,
    /// Host-owned trace anchor from a durable turn start, not input metadata.
    pub turn_traceparent: Option<String>,
}

impl DeliveryEvent {
    /// Read a canonical event envelope (`type`, `data`, `context`, `sequence`).
    pub fn from_envelope(envelope: &Value) -> Option<Self> {
        let sequence = envelope.get("sequence").and_then(Value::as_i64);
        let event_type = envelope.get("type")?.as_str()?;
        let turn_traceparent =
            if event_type == TURN_STARTED && sequence.is_some_and(|sequence| sequence > 0) {
                envelope
                    .get("metadata")
                    .and_then(|metadata| metadata.get(crate::causal::TURN_TRACEPARENT_KEY))
                    .and_then(Value::as_str)
                    .filter(|value| value.len() == 55)
                    .map(str::to_string)
            } else {
                None
            };
        Some(Self {
            sequence,
            event_type: event_type.to_string(),
            data: envelope.get("data").cloned().unwrap_or(Value::Null),
            input_message_id: context_field(envelope, "input_message_id"),
            turn_id: context_field(envelope, "turn_id"),
            turn_traceparent,
        })
    }

    /// Whether this event ends a turn.
    pub fn is_terminal(&self) -> bool {
        is_terminal_turn_event(&self.event_type)
    }
}

fn context_field(envelope: &Value, field: &str) -> Option<String> {
    envelope
        .get("context")
        .and_then(|context| context.get(field))
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// Turn-ending event types.
pub fn is_terminal_turn_event(event_type: &str) -> bool {
    matches!(
        event_type,
        TURN_COMPLETED | TURN_FAILED | TURN_CANCELLED | TURN_SEALED
    )
}

/// Whether delivery goes on after an event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeliveryStep {
    Continue,
    /// The turn ended with this event type; the delivery is finished.
    Finished(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Stream {
    handle: String,
    text: String,
    /// Bytes of `text` already sent; always a previous `text.len()`.
    sent: usize,
    #[serde(default)]
    replacement: Option<String>,
}

/// Delivers one turn's output to one conversation.
pub struct TurnDelivery {
    adapter: Arc<dyn ChannelDeliveryAdapter>,
    context: DeliveryContext,
    session_id: SessionId,
    input_message_id: String,
    options: DeliveryOptions,
    boundary_seen: bool,
    delivered: bool,
    finished: bool,
    tools_running: usize,
    last_status: Option<String>,
    streams: BTreeMap<String, Stream>,
    /// Messages already delivered (posted, closed or replaced). Late deltas
    /// and repeated completions for them are dropped.
    done: HashSet<String>,
    progress: TaskProgress,
    telemetry: DeliveryTelemetry,
    /// Messages whose stream could not open; they post when complete.
    unstreamable: HashSet<String>,
    /// Messages that started as working notes (explicit communication).
    notes: HashSet<String>,
    /// The agent sent a message with `send_message` this turn.
    sent_explicitly: bool,
    /// The `error_code` of this turn's `turn.failed`, if it failed.
    failure_code: Option<String>,
    /// Platform keys of the messages this delivery knows, by message id.
    keys: HashMap<String, String>,
    /// Links made since the host last took them, for it to persist.
    new_links: Vec<(String, String)>,
    journal: Option<Arc<DeliveryJournal>>,
    last_sequence: Option<i64>,
    effects_done: HashSet<String>,
    pub(super) buffered_replies: Vec<(String, String)>,
    pub(super) buffer_overflow: bool,
}

impl TurnDelivery {
    pub fn new(
        adapter: Arc<dyn ChannelDeliveryAdapter>,
        context: DeliveryContext,
        session_id: SessionId,
        input_message_id: impl Into<String>,
        options: DeliveryOptions,
    ) -> Self {
        Self {
            adapter,
            context,
            session_id,
            input_message_id: input_message_id.into(),
            options,
            boundary_seen: false,
            delivered: false,
            finished: false,
            tools_running: 0,
            last_status: None,
            streams: BTreeMap::new(),
            done: HashSet::new(),
            progress: TaskProgress::default(),
            telemetry: DeliveryTelemetry::default(),
            unstreamable: HashSet::new(),
            notes: HashSet::new(),
            sent_explicitly: false,
            failure_code: None,
            keys: HashMap::new(),
            new_links: Vec::new(),
            journal: None,
            last_sequence: None,
            effects_done: HashSet::new(),
            buffered_replies: Vec::new(),
            buffer_overflow: false,
        }
    }

    pub(super) fn set_observer(&mut self, observer: Option<Arc<dyn DeliveryObserver>>) {
        self.telemetry.observer = observer;
    }

    /// Tell the delivery a message's platform key it did not post itself
    /// (the input message, an earlier reply), so the agent's reactions on it
    /// reach the platform.
    pub fn know_message_key(&mut self, message_id: impl Into<String>, key: impl Into<String>) {
        self.keys.insert(message_id.into(), key.into());
    }

    /// Whether this delivery acknowledges the answered message with reactions.
    pub fn acknowledges(&self) -> bool {
        self.options.acknowledge && self.adapter.reactions().is_some()
    }

    /// Whether the delivery knows a message's platform key.
    pub fn knows_message(&self, message_id: &str) -> bool {
        self.keys.contains_key(message_id)
    }

    /// Keys of the messages posted since the last call, as `(key, message_id)`,
    /// for the host to link.
    pub fn take_links(&mut self) -> Vec<(String, String)> {
        std::mem::take(&mut self.new_links)
    }

    fn link(&mut self, message_id: &str, key: String) {
        self.keys
            .entry(message_id.to_string())
            .or_insert_with(|| key.clone());
        self.new_links.push((key, message_id.to_string()));
    }

    /// Where this delivery posts.
    pub fn context(&self) -> &DeliveryContext {
        &self.context
    }

    /// The input message this delivery answers.
    pub fn input_message_id(&self) -> &str {
        &self.input_message_id
    }

    /// Whether the platform accepted anything for this turn.
    pub fn delivered(&self) -> bool {
        self.delivered
    }

    /// Whether the turn ended and the delivery wrapped up.
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// Whether a [`flush`](Self::flush) has anything to push: stream text
    /// the platform has not seen, or changed task progress.
    pub fn has_pending_work(&self) -> bool {
        self.streams
            .values()
            .any(|stream| stream.sent < stream.text.len())
            || (self.progress.is_dirty() && self.adapter.progress().is_some())
    }

    fn streaming(&self) -> bool {
        !self.adapter.aggregate_replies()
            && self.options.stream
            && self.adapter.streaming().is_some()
    }

    /// Feed one event.
    pub async fn observe(&mut self, event: &DeliveryEvent) -> DeliveryStep {
        // Recover the authority's trace anchor even when its already-processed
        // start event is skipped by the output checkpoint.
        if event.event_type == TURN_STARTED
            && event.sequence.is_some_and(|sequence| sequence > 0)
            && event.input_message_id.as_deref() == Some(self.input_message_id.as_str())
            && let Some(carrier) = &event.turn_traceparent
        {
            self.telemetry.anchor(carrier);
        }
        if self.journal.is_some()
            && event
                .sequence
                .zip(self.last_sequence)
                .is_some_and(|(sequence, last)| sequence <= last)
            && !matches!(
                event.event_type.as_str(),
                OUTPUT_MESSAGE_COMPLETED
                    | CONVERSATION_MESSAGE
                    | TOOL_COMPLETED
                    | TURN_COMPLETED
                    | TURN_FAILED
                    | TURN_CANCELLED
                    | TURN_SEALED
            )
        {
            return DeliveryStep::Continue;
        }
        let step = self.observe_event(event).await;
        if let Some(sequence) = event.sequence {
            self.last_sequence = Some(
                self.last_sequence
                    .map_or(sequence, |last| last.max(sequence)),
            );
        }
        self.checkpoint().await;
        step
    }

    async fn observe_event(&mut self, event: &DeliveryEvent) -> DeliveryStep {
        if self.finished {
            return DeliveryStep::Finished(String::new());
        }
        let matches = event.input_message_id.as_deref() == Some(self.input_message_id.as_str());
        self.boundary_seen |= matches;
        let ours = matches || (event.event_type == TURN_CANCELLED && self.boundary_seen);
        if !ours {
            return DeliveryStep::Continue;
        }

        match event.event_type.as_str() {
            OUTPUT_MESSAGE_STARTED => {
                if event.data.get("phase").and_then(Value::as_str) == Some("commentary")
                    && let Some(id) = event.data.get("message_id").and_then(Value::as_str)
                {
                    self.notes.insert(id.to_string());
                }
            }
            OUTPUT_MESSAGE_DELTA
                if event
                    .data
                    .get("message_id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| self.notes.contains(id)) => {}
            OUTPUT_MESSAGE_DELTA if self.streaming() => self.on_delta(&event.data).await,
            CONVERSATION_MESSAGE => self.on_sent(&event.data).await,
            OUTPUT_MESSAGE_REPLACED if self.streaming() => self.on_replaced(&event.data).await,
            OUTPUT_MESSAGE_REPLACED if self.adapter.aggregate_replies() => {
                if let (Some(id), Some(replacement)) = (
                    event.data["message_id"].as_str(),
                    event.data["replacement"].as_str(),
                ) {
                    if let Some((_, text)) =
                        self.buffered_replies.iter_mut().find(|(key, _)| key == id)
                    {
                        if replacement.len() <= 131_072 {
                            *text = replacement.to_string();
                        } else {
                            self.buffer_overflow = true;
                        }
                    }
                    if self
                        .buffered_replies
                        .iter()
                        .map(|(_, text)| text.len() + 2)
                        .sum::<usize>()
                        > 131_072
                    {
                        self.buffer_overflow = true;
                    }
                    if let Some(journal) = &self.journal {
                        let _ = journal.save(self.snapshot()).await;
                    }
                }
            }
            OUTPUT_MESSAGE_COMPLETED => self.on_completed(&event.data).await,
            MESSAGE_REACTION_ADDED => self.on_reaction(&event.data, true).await,
            MESSAGE_REACTION_REMOVED => self.on_reaction(&event.data, false).await,
            TOOL_STARTED => {
                self.acknowledge_start(&event.data).await;
                self.tools_running += 1;
                self.set_status(self.current_status()).await;
            }
            // `tool.completed` also reports failures (there is no
            // `tool.failed`), so the counter cannot strand above zero.
            TOOL_COMPLETED => {
                self.tools_running = self.tools_running.saturating_sub(1);
                if chose_silence(&event.data) {
                    self.delivered = true;
                    self.buffered_replies.clear();
                }
                self.prompt_approval(&event.data).await;
                self.show_app_view(&event.data).await;
                self.set_status(self.current_status()).await;
            }
            TASK_CREATED | TASK_UPDATED => self.on_task(&event.data),
            TURN_STARTED => {
                if let Some(carrier) = &event.turn_traceparent {
                    self.telemetry.anchor(carrier);
                }
                self.set_status(self.options.thinking_status.clone()).await;
            }
            SESSION_TITLE_UPDATED => {
                if let Some(title) = event.data.get("title").and_then(Value::as_str)
                    && !title.trim().is_empty()
                {
                    self.set_title(title).await;
                }
            }
            _ => {}
        }

        if event.event_type == TURN_FAILED {
            self.failure_code = event
                .data
                .get("error_code")
                .and_then(Value::as_str)
                .map(str::to_string);
        }
        if event.is_terminal() {
            self.finish(&event.event_type).await;
            return DeliveryStep::Finished(event.event_type.clone());
        }
        DeliveryStep::Continue
    }

    /// Push text that open streams have not sent yet. Hosts call this on a
    /// timer so streamed text reaches the platform at a rate it absorbs.
    pub async fn flush(&mut self) {
        self.flush_pending().await;
        self.checkpoint().await;
    }

    async fn flush_pending(&mut self) {
        let ids: Vec<String> = self
            .streams
            .iter()
            .filter(|(_, stream)| stream.sent < stream.text.len())
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            self.flush_stream(&id, false).await;
        }
        if self.progress.is_dirty() {
            self.push_progress(false).await;
        }
    }

    /// Wrap up: stop open streams, clear the status, and post the notice when
    /// nothing reached the platform. Called by [`observe`](Self::observe) on a
    /// terminal event; hosts call it directly when the event stream ends early.
    pub async fn finish(&mut self, event_type: &str) {
        self.finish_turn(event_type).await;
        self.checkpoint().await;
    }

    async fn finish_turn(&mut self, event_type: &str) {
        if self.finished {
            return;
        }
        if self.adapter.aggregate_replies() {
            if event_type == TURN_COMPLETED
                && !self.delivered
                && !self.buffer_overflow
                && !self.buffered_replies.is_empty()
                && !self.done.contains("aggregate-final")
            {
                let text = self
                    .buffered_replies
                    .iter()
                    .map(|(_, text)| text.as_str())
                    .collect::<Vec<_>>()
                    .join("\n\n");
                if let Err(error) = self.post_message(text, Some("aggregate-final")).await {
                    warn!(session_id = %self.session_id, %error, "channel: aggregate reply needs recovery");
                    return;
                }
            }
            self.done.insert("aggregate-final".into());
            if self.before_write("turn-finished", true).await.is_err() {
                return;
            }
            let result = self
                .adapter
                .turn_finished(
                    &self.context,
                    if self.buffer_overflow {
                        TURN_FAILED
                    } else {
                        event_type
                    },
                )
                .await;
            self.after_write(&result).await;
            if !matches!(result, DeliveryResult::Ok) {
                return;
            }
            self.finished = true;
            return;
        }
        let ids: Vec<String> = self.streams.keys().cloned().collect();
        for id in ids {
            self.close_stream(&id).await;
        }
        self.set_status(String::new()).await;
        // A summary frozen mid-flight reads as live forever: the final push
        // says where the fan-out got to.
        self.push_progress(true).await;
        self.acknowledge_end(event_type).await;

        // An agent that talks through `send_message` may have posted progress
        // before failing; the person still needs to know the request ended.
        let explicit_failure =
            self.sent_explicitly && matches!(event_type, TURN_FAILED | TURN_CANCELLED);
        if (!self.delivered || explicit_failure) && !self.needs_recovery().await {
            let notice = self.notice(event_type);
            if let Err(error) = self.post(notice).await {
                warn!(session_id = %self.session_id, %error, "channel: could not post the end-of-turn notice");
            }
        }
        self.finished = true;
    }

    async fn on_delta(&mut self, data: &Value) {
        let Some(message_id) = data.get("message_id").and_then(Value::as_str) else {
            return;
        };
        if self.unstreamable.contains(message_id) || self.done.contains(message_id) {
            return;
        }
        let delta = data.get("delta").and_then(Value::as_str).unwrap_or("");
        let accumulated = data.get("accumulated").and_then(Value::as_str);

        if !self.streams.contains_key(message_id) {
            let operation = format!("reply:{message_id}:start");
            if self.before_write(operation, false).await.is_err() {
                return;
            }
            let started = match self.adapter.streaming() {
                Some(stream) => stream.start_checked(&self.context).await,
                None => return,
            };
            self.after_write(
                &started
                    .as_ref()
                    .map_or_else(Clone::clone, |_| DeliveryResult::Ok),
            )
            .await;
            match started {
                Ok(handle) => {
                    if let Some(reactions) = self.adapter.reactions() {
                        let key = reactions.stream_message_key(&handle, &self.context);
                        self.link(message_id, key);
                    }
                    self.streams.insert(
                        message_id.to_string(),
                        Stream {
                            handle,
                            text: String::new(),
                            sent: 0,
                            replacement: None,
                        },
                    );
                }
                Err(error) => {
                    warn!(session_id = %self.session_id, %error, "channel: could not open a stream, posting the message whole");
                    if self.journal.is_none()
                        || !matches!(
                            error,
                            DeliveryResult::RateLimited { .. } | DeliveryResult::TransientError(_)
                        )
                    {
                        self.unstreamable.insert(message_id.to_string());
                    }
                    return;
                }
            }
        }

        let Some(stream) = self.streams.get_mut(message_id) else {
            return;
        };
        match accumulated {
            // A full prefix heals a dropped delta. It may never shrink below
            // what was already sent.
            Some(full)
                if full.len() >= stream.sent && full.starts_with(&stream.text[..stream.sent]) =>
            {
                stream.text = full.to_string();
            }
            _ => stream.text.push_str(delta),
        }
        if stream.text.len() - stream.sent >= STREAM_FLUSH_CHARS {
            let id = message_id.to_string();
            self.flush_stream(&id, false).await;
        }
    }

    async fn on_replaced(&mut self, data: &Value) {
        let (Some(message_id), Some(replacement)) = (
            data.get("message_id").and_then(Value::as_str),
            data.get("replacement").and_then(Value::as_str),
        ) else {
            return;
        };
        if let Some(stream) = self.streams.get_mut(message_id) {
            stream.replacement = Some(replacement.to_string());
        }
        let Some(stream) = self.streams.get(message_id).cloned() else {
            return;
        };
        let Some(streaming) = self.adapter.streaming() else {
            return;
        };
        // `replace` closes the stream. When it fails the stream is still open,
        // and every start must end in a stop.
        let started = self.telemetry.start();
        if self
            .before_write(format!("reply:{message_id}:replace"), true)
            .await
            .is_err()
        {
            return;
        }
        let result = streaming
            .replace(&stream.handle, replacement, &self.context)
            .await;
        self.after_write(&result).await;
        let outcome = DeliveryOutcome::from(&result);
        self.telemetry.finish(
            self.adapter.platform(),
            started,
            DeliveryKind::Reply,
            outcome,
        );
        if let Some(error_type) = outcome.error_type() {
            warn!(session_id = %self.session_id, error_type, "channel: could not replace a streamed message");
            if self
                .before_write(format!("reply:{message_id}:replacement-stop"), true)
                .await
                .is_ok()
            {
                let stopped = streaming.stop(&stream.handle, &self.context).await;
                self.after_write(&stopped).await;
            }
            if self.journal.is_some()
                && matches!(
                    result,
                    DeliveryResult::RateLimited { .. }
                        | DeliveryResult::TransientError(_)
                        | DeliveryResult::Unknown { .. }
                )
            {
                return;
            }
        }
        if let Some(journal) = &self.journal {
            journal
                .resolve_retries(&format!("reply:{message_id}:"))
                .await;
        }
        self.streams.remove(message_id);
        self.done.insert(message_id.to_string());
        self.delivered = true;
    }

    async fn on_completed(&mut self, data: &Value) {
        let message_id = data
            .get("message")
            .and_then(|message| message.get("id"))
            .and_then(Value::as_str)
            .map(str::to_string);
        if let Some(id) = &message_id
            && self.done.contains(id)
        {
            return;
        }
        let text = response_text(data);
        if let (Some(id), Some(text), Some(journal)) = (&message_id, &text, &self.journal)
            && !self.streams.contains_key(id)
            && self.streaming()
            && journal.retrying(&format!("reply:{id}:start")).await
        {
            let id = id.clone();
            let text = text.clone();
            self.on_delta(&serde_json::json!({"message_id":id,"delta":text}))
                .await;
            if !self.streams.contains_key(&id) {
                return;
            }
        }

        if let Some(id) = &message_id
            && let Some(stream) = self.streams.get_mut(id)
        {
            // The completed text is authoritative: the last chunk tends to
            // arrive between the final delta and completion.
            if let Some(text) = &text
                && text.starts_with(&stream.text[..stream.sent])
            {
                stream.text = text.clone();
            }
            let started = self.telemetry.start();
            let outcome = self.close_stream(id).await;
            self.telemetry.finish(
                self.adapter.platform(),
                started,
                DeliveryKind::Reply,
                outcome,
            );
            return;
        }

        if is_working_note(data) {
            return;
        }
        let Some(text) = text else { return };
        if self.adapter.aggregate_replies() {
            // Bound durable projection size. An overflow suppresses the whole
            // automatic reply; sending a partial answer would hide the loss.
            let size: usize = self.buffered_replies.iter().map(|(_, t)| t.len() + 2).sum();
            if size.saturating_add(text.len()) > 131_072 {
                self.buffer_overflow = true;
            } else if !self.buffer_overflow {
                self.buffered_replies
                    .push((message_id.clone().unwrap_or_default(), text));
            }
            if let Some(id) = message_id {
                self.done.insert(id);
            }
            if let Some(journal) = &self.journal
                && let Err(error) = journal.save(self.snapshot()).await
            {
                warn!(%error, "channel: could not checkpoint aggregate reply");
            }
            return;
        }
        let started = self.telemetry.start();
        let posted = self.post_message(text, message_id.as_deref()).await;
        let outcome = posted
            .as_ref()
            .err()
            .map_or(DeliveryOutcome::Succeeded, DeliveryOutcome::from);
        self.telemetry.finish(
            self.adapter.platform(),
            started,
            DeliveryKind::Reply,
            outcome,
        );
        match posted {
            Ok(()) => {
                if let Some(id) = message_id {
                    self.done.insert(id);
                }
            }
            Err(_) => {
                if let Some(error_type) = outcome.error_type() {
                    warn!(session_id = %self.session_id, error_type, "channel: could not post a reply");
                }
            }
        }
    }

    /// A message the agent sent with `send_message`. A sender that already
    /// put it on the platform records where; anything else is posted here.
    async fn on_sent(&mut self, data: &Value) {
        self.sent_explicitly = true;
        if data
            .get("delivery")
            .and_then(|delivery| delivery.get("message_ref"))
            .and_then(Value::as_str)
            .is_some_and(|reference| !reference.is_empty())
        {
            self.delivered = true;
            return;
        }
        let Some(text) = data
            .get("text")
            .and_then(Value::as_str)
            .filter(|text| !text.trim().is_empty())
        else {
            return;
        };
        let key = effect_key("explicit", data);
        if self.done.contains(&key) {
            return;
        }
        if self.adapter.aggregate_replies() {
            let size: usize = self.buffered_replies.iter().map(|(_, t)| t.len() + 2).sum();
            if size.saturating_add(text.len()) > 131_072 {
                self.buffer_overflow = true;
            } else if !self.buffer_overflow {
                self.buffered_replies.push((key.clone(), text.to_string()));
            }
            self.done.insert(key);
            return;
        }
        if self
            .post_scoped_message(text.to_string(), None, format!("explicit:{key}"))
            .await
            .is_ok()
        {
            self.done.insert(key);
        }
    }

    /// Send what a stream has produced but not sent. Until `closing`, a
    /// trailing angle-bracket token that has not closed yet is held back.
    async fn flush_stream(&mut self, message_id: &str, closing: bool) -> DeliveryOutcome {
        let Some(streaming) = self.adapter.streaming() else {
            return DeliveryOutcome::Succeeded;
        };
        let Some(stream) = self.streams.get_mut(message_id) else {
            return DeliveryOutcome::Succeeded;
        };
        if stream.replacement.is_some() || stream.sent >= stream.text.len() {
            return DeliveryOutcome::Succeeded;
        }
        let pending = &stream.text[stream.sent..];
        let end = if closing {
            pending.len()
        } else {
            open_token_start(pending)
        };
        if end == 0 {
            return DeliveryOutcome::Succeeded;
        }
        let pending = pending[..end].to_string();
        let handle = stream.handle.clone();
        let offset = stream.sent;
        if let Err(error) = self
            .before_write(format!("reply:{message_id}:append:{offset}"), false)
            .await
        {
            return DeliveryOutcome::from(&error);
        }
        let result = streaming.append(&handle, &pending, &self.context).await;
        self.after_write(&result).await;
        let outcome = DeliveryOutcome::from(&result);
        if let Some(error_type) = outcome.error_type() {
            warn!(session_id = %self.session_id, error_type, "channel: could not append to a stream");
        } else {
            if let Some(stream) = self.streams.get_mut(message_id) {
                stream.sent += end;
            }
            self.delivered = true;
        }
        outcome
    }

    async fn close_stream(&mut self, message_id: &str) -> DeliveryOutcome {
        if let Some(replacement) = self
            .streams
            .get(message_id)
            .and_then(|stream| stream.replacement.clone())
        {
            self.on_replaced(
                &serde_json::json!({"message_id":message_id,"replacement":replacement}),
            )
            .await;
            return if self.streams.contains_key(message_id) {
                DeliveryOutcome::Transient
            } else {
                DeliveryOutcome::Succeeded
            };
        }
        let flushed = self.flush_stream(message_id, true).await;
        if matches!(
            flushed,
            DeliveryOutcome::Transient | DeliveryOutcome::RateLimited
        ) {
            return flushed;
        }
        let Some(stream) = self.streams.get(message_id).cloned() else {
            return flushed;
        };
        let Some(streaming) = self.adapter.streaming() else {
            return flushed;
        };
        if let Err(error) = self
            .before_write(format!("reply:{message_id}:stop"), true)
            .await
        {
            return DeliveryOutcome::from(&error);
        }
        let result = streaming.stop(&stream.handle, &self.context).await;
        self.after_write(&result).await;
        let stopped = DeliveryOutcome::from(&result);
        if matches!(
            stopped,
            DeliveryOutcome::Transient | DeliveryOutcome::RateLimited | DeliveryOutcome::Unknown
        ) {
            return stopped;
        }
        self.streams.remove(message_id);
        self.done.insert(message_id.to_string());
        if let Some(error_type) = stopped.error_type() {
            warn!(session_id = %self.session_id, error_type, "channel: could not stop a stream");
        }
        if stream.sent > 0 {
            self.delivered = true;
        }
        flushed.or(stopped)
    }

    fn current_status(&self) -> String {
        if self.tools_running == 0 {
            return self.options.thinking_status.clone();
        }
        self.options
            .tool_status
            .clone()
            .unwrap_or_else(|| self.options.thinking_status.clone())
    }

    fn agent_surface(&self) -> Option<&dyn super::ChannelAgentSurface> {
        self.options
            .agent_surface
            .then(|| self.adapter.agent_surface())
            .flatten()
    }

    async fn set_status(&mut self, status: String) {
        let Some(surface) = self.agent_surface() else {
            return;
        };
        if self.last_status.as_deref() == Some(status.as_str())
            || (self.last_status.is_none() && status.is_empty())
        {
            return;
        }
        if self.before_write("status", true).await.is_err() {
            return;
        }
        let result = surface.set_status(&status, &self.context).await;
        self.after_write(&result).await;
        match result {
            DeliveryResult::Ok => self.last_status = Some(status),
            error => {
                warn!(session_id = %self.session_id, %error, "channel: could not set the status");
            }
        }
    }

    async fn set_title(&self, title: &str) {
        let Some(surface) = self.agent_surface() else {
            return;
        };
        if self.before_write("title", true).await.is_err() {
            return;
        }
        let result = surface.set_title(title, &self.context).await;
        self.after_write(&result).await;
        if let error @ (DeliveryResult::TransientError(_)
        | DeliveryResult::PermanentError(_)
        | DeliveryResult::Unknown { .. }
        | DeliveryResult::RateLimited { .. }
        | DeliveryResult::Partial { .. }) = result
        {
            warn!(session_id = %self.session_id, %error, "channel: could not set the title");
        }
    }

    async fn prompt_approval(&mut self, data: &Value) {
        let Some(approvals) = self.adapter.approvals() else {
            return;
        };
        let Some(prompt) = approval_prompt(data) else {
            return;
        };
        let key = effect_key("approval", data);
        if self.effects_done.contains(&key) || self.before_write(&key, false).await.is_err() {
            return;
        }
        let started = self.telemetry.start();
        let result = approvals
            .prompt(self.session_id, &prompt, &self.context)
            .await;
        self.after_write(&result).await;
        let outcome = DeliveryOutcome::from(&result);
        if matches!(result, DeliveryResult::Ok) {
            self.effects_done.insert(key);
        }
        self.telemetry.finish(
            self.adapter.platform(),
            started,
            DeliveryKind::Approval,
            outcome,
        );
        if let Some(error_type) = outcome.error_type() {
            warn!(session_id = %self.session_id, error_type, "channel: could not show an approval prompt");
        } else {
            self.delivered = true;
        }
    }

    async fn show_app_view(&mut self, data: &Value) {
        let Some(surface) = self.adapter.app_views() else {
            return;
        };
        let Some(view) = app_view(data) else {
            return;
        };
        let key = format!("app:{}", view.tool_call_id);
        if self.effects_done.contains(&key) || self.before_write(&key, false).await.is_err() {
            return;
        }
        let result = surface.show(self.session_id, &view, &self.context).await;
        self.after_write(&result).await;
        if matches!(result, DeliveryResult::Ok) {
            self.effects_done.insert(key);
        }
        match result {
            DeliveryResult::Ok => {}
            error => {
                warn!(session_id = %self.session_id, %error, "channel: could not show an MCP App view");
            }
        }
    }

    fn on_task(&mut self, data: &Value) {
        let Some(task) = data.get("task") else {
            return;
        };
        let field = |name: &str| task.get(name).and_then(Value::as_str);
        if let (Some(id), Some(name), Some(state)) = (
            field("id"),
            field("display_name"),
            field("state").and_then(SessionTaskState::parse),
        ) {
            self.progress.observe(id, name, state);
        }
    }

    /// Post the progress message the first time and edit it after. `last`
    /// pushes even when nothing changed, so the text drops its in-flight
    /// framing.
    async fn push_progress(&mut self, last: bool) {
        let Some(surface) = self.adapter.progress() else {
            return;
        };
        if self.progress.is_empty() || !(self.progress.is_dirty() || last) {
            return;
        }
        let text = self.progress.render(last);
        match self.progress.handle().map(str::to_string) {
            Some(handle) => {
                if self.before_write("progress:update", true).await.is_err() {
                    return;
                }
                let result = surface.update(&handle, &text, &self.context).await;
                self.after_write(&result).await;
                match result {
                    DeliveryResult::Ok => {
                        self.progress.mark_updated();
                        self.delivered = true;
                    }
                    error => {
                        warn!(session_id = %self.session_id, %error, "channel: could not update task progress");
                    }
                }
            }
            None => {
                if self.before_write("progress:post", false).await.is_err() {
                    return;
                }
                let result = surface.post_checked(&text, &self.context).await;
                self.after_write(
                    &result
                        .as_ref()
                        .map_or_else(Clone::clone, |_| DeliveryResult::Ok),
                )
                .await;
                match result {
                    Ok(handle) => {
                        self.progress.mark_posted(handle);
                        self.delivered = true;
                    }
                    Err(error) => {
                        warn!(session_id = %self.session_id, %error, "channel: could not post task progress");
                    }
                }
            }
        }
    }

    async fn post(&mut self, text: String) -> Result<(), String> {
        self.post_message(text, None)
            .await
            .map_err(|error| error.to_string())
    }

    /// Post text; a reply's message id links the posted message to it.
    async fn post_message(
        &mut self,
        text: String,
        message_id: Option<&str>,
    ) -> Result<(), DeliveryResult> {
        self.post_scoped_message(
            text,
            message_id,
            format!("reply:{}:post", message_id.unwrap_or("notice")),
        )
        .await
    }

    async fn post_scoped_message(
        &mut self,
        text: String,
        message_id: Option<&str>,
        operation: String,
    ) -> Result<(), DeliveryResult> {
        self.before_write(operation, self.adapter.idempotent_replies())
            .await?;
        let message = OutboundChannelMessage {
            session_id: self.session_id,
            text,
            thread_ref: self.context.thread_ref.clone(),
            correlation_id: Some(self.input_message_id.clone()),
        };
        if let (Some(message_id), Some(reactions)) = (message_id, self.adapter.reactions()) {
            let result = reactions.post_keyed(&message, &self.context).await;
            self.after_write(
                &result
                    .as_ref()
                    .map_or_else(Clone::clone, |_| DeliveryResult::Ok),
            )
            .await;
            return match result {
                Ok(key) => {
                    self.delivered = true;
                    if let Some(key) = key {
                        self.link(message_id, key);
                    }
                    Ok(())
                }
                Err(DeliveryResult::Ok) => {
                    self.delivered = true;
                    Ok(())
                }
                Err(error) => {
                    for reference in error.confirmed() {
                        self.link(message_id, reference.clone());
                    }
                    Err(error)
                }
            };
        }
        let result = self.adapter.deliver(&message, &self.context).await;
        self.after_write(&result).await;
        match result {
            DeliveryResult::Ok => {
                self.delivered = true;
                Ok(())
            }
            error => Err(error),
        }
    }

    /// The agent's own reaction, mirrored onto the platform message. Other
    /// people's reactions came from a surface that already shows them.
    async fn on_reaction(&mut self, data: &Value, added: bool) {
        let Some(reactions) = self.adapter.reactions() else {
            return;
        };
        let field = |name: &str| data.get(name).and_then(Value::as_str);
        let by_agent = data
            .get("actor")
            .and_then(|actor| actor.get("kind"))
            .and_then(Value::as_str)
            == Some("agent");
        let (Some(message_id), Some(emoji)) = (field("message_id"), field("emoji")) else {
            return;
        };
        if !by_agent {
            return;
        }
        if message_id == self.input_message_id {
            self.effects_done.insert(ACK_AGENT_REACTED.into());
        }
        let Some(key) = self.keys.get(message_id) else {
            return;
        };
        if self
            .before_write(format!("reaction:{message_id}:{emoji}:{added}"), true)
            .await
            .is_err()
        {
            return;
        }
        let result = reactions.react(key, emoji, added, &self.context).await;
        self.after_write(&result).await;
        if let error @ (DeliveryResult::TransientError(_)
        | DeliveryResult::PermanentError(_)
        | DeliveryResult::Unknown { .. }
        | DeliveryResult::RateLimited { .. }
        | DeliveryResult::Partial { .. }) = result
        {
            warn!(session_id = %self.session_id, %error, "channel: could not mirror a reaction");
        }
    }

    /// 👀 on the answered message when the agent starts real work: its first
    /// tool call that is not just talking. A turn that only answers or stays
    /// quiet never flashes it, which matters for a channel where the agent
    /// hears every message and answers few.
    async fn acknowledge_start(&mut self, data: &Value) {
        if !self.options.acknowledge
            || self.effects_done.contains(ACK_STARTED_DONE)
            || self.effects_done.contains(ACK_AGENT_REACTED)
        {
            return;
        }
        let tool = data
            .pointer("/tool_call/name")
            .or_else(|| data.get("tool_name"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        if matches!(
            tool,
            NO_REPLY_TOOL_NAME | SEND_MESSAGE_TOOL_NAME | REACT_TOOL_NAME
        ) {
            return;
        }
        if self.acknowledge_react(ACK_STARTED, true).await {
            self.effects_done.insert(ACK_STARTED_DONE.into());
        }
    }

    /// At the end of the turn: ✅ when the agent sent its answer, and the 👀
    /// this delivery added comes off either way, so a failed or silent turn
    /// does not look like work still going on.
    async fn acknowledge_end(&mut self, event_type: &str) {
        if !self.options.acknowledge || self.effects_done.contains(ACK_AGENT_REACTED) {
            return;
        }
        if self.effects_done.contains(ACK_STARTED_DONE)
            && self.acknowledge_react(ACK_STARTED, false).await
        {
            self.effects_done.remove(ACK_STARTED_DONE);
        }
        if self.sent_explicitly
            && event_type == TURN_COMPLETED
            && !self.effects_done.contains(ACK_ANSWERED_DONE)
            && self.acknowledge_react(ACK_ANSWERED, true).await
        {
            self.effects_done.insert(ACK_ANSWERED_DONE.into());
        }
    }

    /// React on the answered message. False when the platform has no
    /// reactions, the message's key is unknown (the turn did not start from a
    /// platform message, such as a schedule), or the call failed.
    async fn acknowledge_react(&mut self, emoji: &str, add: bool) -> bool {
        let Some(reactions) = self.adapter.reactions() else {
            return false;
        };
        let Some(key) = self.keys.get(&self.input_message_id).cloned() else {
            return false;
        };
        if self
            .before_write(format!("ack:{emoji}:{add}"), true)
            .await
            .is_err()
        {
            return false;
        }
        let result = reactions.react(&key, emoji, add, &self.context).await;
        self.after_write(&result).await;
        match result {
            DeliveryResult::Ok => true,
            error => {
                warn!(session_id = %self.session_id, %error, "channel: could not acknowledge with a reaction");
                false
            }
        }
    }

    fn notice(&self, event_type: &str) -> String {
        let misconfigured = event_type == TURN_FAILED
            && self
                .failure_code
                .as_deref()
                .is_some_and(is_setup_error_code);
        let headline = match event_type {
            _ if misconfigured => {
                "This agent is not set up correctly, so it cannot answer right now. Someone who manages it needs to check its model and provider settings."
            }
            TURN_FAILED | TURN_SEALED => "The agent could not finish this request.",
            TURN_CANCELLED => "This request was cancelled.",
            _ => "The agent finished without a reply.",
        };
        match &self.options.session_link {
            Some(link) => format!("{headline} {link}"),
            None => headline.to_string(),
        }
    }
}

/// Whether a turn failure's code means the agent's model or provider is set
/// up wrong: retrying will not help until someone who manages it acts.
pub fn is_setup_error_code(code: &str) -> bool {
    use everruns_contracts::user_facing_error::codes;
    matches!(
        code,
        codes::MODEL_UNAVAILABLE
            | codes::MODEL_NOT_CONFIGURED
            | codes::PROVIDER_MISCONFIGURED
            | codes::PROVIDER_QUOTA_EXHAUSTED
            | codes::PROVIDER_ATTESTATION_REQUIRED
    )
}

/// The text of a completed assistant message. Blank parts are not reply text:
/// a tool-calling step can carry one, and some platforms refuse empty posts.
pub fn response_text(data: &Value) -> Option<String> {
    let parts: Vec<&str> = data
        .get("message")?
        .get("content")?
        .as_array()?
        .iter()
        .filter(|part| part.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|part| part.get("text").and_then(Value::as_str))
        .filter(|text| !text.trim().is_empty())
        .collect();
    (!parts.is_empty()).then(|| parts.join("\n"))
}

/// Whether a completed assistant message is an explicit agent's working note.
fn is_working_note(data: &Value) -> bool {
    data.get("message")
        .and_then(|message| message.get("phase_source"))
        .and_then(Value::as_str)
        == Some("communication")
}

/// Whether a `tool.completed` is the agent deliberately not replying.
fn chose_silence(data: &Value) -> bool {
    data.get("tool_name").and_then(Value::as_str) == Some(NO_REPLY_TOOL_NAME)
        && data.get("success").and_then(Value::as_bool) == Some(true)
}

/// Longest unclosed `<...` tail a stream flush holds back.
const MAX_HELD_TOKEN_CHARS: usize = 64;

/// Where an unclosed trailing `<...` token starts in `pending`, or its length
/// when there is none.
///
/// Platforms parse `<...>` tokens (Slack's `<!here>`, `<@U123>`) from the
/// whole streamed message, and a delivery adapter can only check or rewrite a
/// token it sees whole. Holding a short unclosed tail until the next flush
/// keeps a token inside one append; a long one is not a token and goes out.
fn open_token_start(pending: &str) -> usize {
    match pending.rfind('<') {
        Some(start)
            if !pending[start..].contains('>') && pending.len() - start <= MAX_HELD_TOKEN_CHARS =>
        {
            start
        }
        _ => pending.len(),
    }
}

#[cfg(test)]
#[path = "delivery_tests.rs"]
mod tests;

fn effect_key(kind: &str, data: &Value) -> String {
    use base64::Engine;
    use sha2::{Digest, Sha256};
    if let Some(id) = ["tool_call_id", "message_id", "id"]
        .into_iter()
        .find_map(|name| data.get(name).and_then(Value::as_str))
    {
        return format!("{kind}:{id}");
    }
    fn canonical(value: &Value) -> Value {
        match value {
            Value::Object(object) => {
                let sorted = object
                    .iter()
                    .map(|(key, value)| (key.clone(), canonical(value)))
                    .collect::<BTreeMap<_, _>>();
                Value::Object(sorted.into_iter().collect())
            }
            Value::Array(values) => Value::Array(values.iter().map(canonical).collect()),
            value => value.clone(),
        }
    }
    format!(
        "{kind}:{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(Sha256::digest(canonical(data).to_string().as_bytes()))
    )
}

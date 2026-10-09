//! Reply delivery: one turn's events in, platform calls out.
//!
//! Decisions (carried over from the server's Slack dispatcher, where each was
//! learned the hard way):
//! - A delivery belongs to one turn, keyed by its input message id. Events of
//!   other turns are skipped, except `turn.cancelled`: cancellation is
//!   session-scoped and its synthetic event carries a fresh id, so it ends the
//!   delivery once this turn's boundary was seen (EVE-966).
//! - Automatic mode posts each completed assistant message. Tool-only mode
//!   posts nothing itself; `channel_post_message` receipts count as delivered.
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
//! - No I/O happens outside the adapter, so the same object runs in a
//!   Framework task, serve, and the server's Postgres-driven dispatcher.

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use serde_json::Value;
use tracing::warn;

use super::approval::approval_prompt;
use super::progress::TaskProgress;
use super::{
    ChannelDeliveryAdapter, ChannelReplyMode, DeliveryContext, DeliveryResult,
    OutboundChannelMessage,
};
use crate::channel_messaging::CHANNEL_POST_MESSAGE_TOOL_NAME;
use everruns_contracts::runtime::events::{
    OUTPUT_MESSAGE_COMPLETED, OUTPUT_MESSAGE_DELTA, OUTPUT_MESSAGE_REPLACED, SESSION_TITLE_UPDATED,
    TASK_CREATED, TASK_UPDATED, TOOL_COMPLETED, TOOL_STARTED, TURN_CANCELLED, TURN_COMPLETED,
    TURN_FAILED, TURN_SEALED, TURN_STARTED,
};
use everruns_contracts::runtime::session_task::SessionTaskState;
use everruns_contracts::typed_id::SessionId;

/// Flush a stream early once this much text is waiting.
pub const STREAM_FLUSH_CHARS: usize = 2_000;

/// How a delivery behaves. Hosts fill it from the channel's configuration.
#[derive(Debug, Clone)]
pub struct DeliveryOptions {
    /// Which agent output reaches the platform.
    pub reply_mode: ChannelReplyMode,
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
}

impl Default for DeliveryOptions {
    fn default() -> Self {
        Self {
            reply_mode: ChannelReplyMode::AllMessages,
            stream: true,
            agent_surface: true,
            thinking_status: "is thinking...".to_string(),
            tool_status: None,
            session_link: None,
        }
    }
}

/// One session event, as reply delivery reads it.
#[derive(Debug, Clone, PartialEq)]
pub struct DeliveryEvent {
    /// Durable sequence; `None` for ephemeral deltas.
    pub sequence: Option<i64>,
    pub event_type: String,
    pub data: Value,
    /// The turn's input message, from the event context.
    pub input_message_id: Option<String>,
}

impl DeliveryEvent {
    /// Read a canonical event envelope (`type`, `data`, `context`, `sequence`).
    pub fn from_envelope(envelope: &Value) -> Option<Self> {
        Some(Self {
            sequence: envelope.get("sequence").and_then(Value::as_i64),
            event_type: envelope.get("type")?.as_str()?.to_string(),
            data: envelope.get("data").cloned().unwrap_or(Value::Null),
            input_message_id: envelope
                .get("context")
                .and_then(|context| context.get("input_message_id"))
                .and_then(Value::as_str)
                .map(str::to_string),
        })
    }

    /// Whether this event ends a turn.
    pub fn is_terminal(&self) -> bool {
        is_terminal_turn_event(&self.event_type)
    }
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

#[derive(Debug)]
struct Stream {
    handle: String,
    text: String,
    /// Bytes of `text` already sent; always a previous `text.len()`.
    sent: usize,
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
    /// Messages whose stream could not open; they post when complete.
    unstreamable: HashSet<String>,
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
            unstreamable: HashSet::new(),
        }
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
        self.options.stream
            && self.context.reply_mode == ChannelReplyMode::AllMessages
            && self.adapter.streaming().is_some()
    }

    /// Feed one event.
    pub async fn observe(&mut self, event: &DeliveryEvent) -> DeliveryStep {
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
            OUTPUT_MESSAGE_DELTA if self.streaming() => self.on_delta(&event.data).await,
            OUTPUT_MESSAGE_REPLACED if self.streaming() => self.on_replaced(&event.data).await,
            OUTPUT_MESSAGE_COMPLETED => self.on_completed(&event.data).await,
            TOOL_STARTED => {
                self.tools_running += 1;
                self.set_status(self.current_status()).await;
            }
            // `tool.completed` also reports failures (there is no
            // `tool.failed`), so the counter cannot strand above zero.
            TOOL_COMPLETED => {
                self.tools_running = self.tools_running.saturating_sub(1);
                if explicit_message_delivered(&event.data) {
                    self.delivered = true;
                }
                self.prompt_approval(&event.data).await;
                self.set_status(self.current_status()).await;
            }
            TASK_CREATED | TASK_UPDATED => self.on_task(&event.data),
            TURN_STARTED => self.set_status(self.options.thinking_status.clone()).await,
            SESSION_TITLE_UPDATED => {
                if let Some(title) = event.data.get("title").and_then(Value::as_str)
                    && !title.trim().is_empty()
                {
                    self.set_title(title).await;
                }
            }
            _ => {}
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
        let ids: Vec<String> = self
            .streams
            .iter()
            .filter(|(_, stream)| stream.sent < stream.text.len())
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            self.flush_stream(&id).await;
        }
        if self.progress.is_dirty() {
            self.push_progress(false).await;
        }
    }

    /// Wrap up: stop open streams, clear the status, and post the notice when
    /// nothing reached the platform. Called by [`observe`](Self::observe) on a
    /// terminal event; hosts call it directly when the event stream ends early.
    pub async fn finish(&mut self, event_type: &str) {
        if self.finished {
            return;
        }
        self.finished = true;
        let ids: Vec<String> = self.streams.keys().cloned().collect();
        for id in ids {
            self.close_stream(&id).await;
        }
        self.set_status(String::new()).await;
        // A summary frozen mid-flight reads as live forever: the final push
        // says where the fan-out got to.
        self.push_progress(true).await;

        let tool_only_failure = self.context.reply_mode == ChannelReplyMode::ToolOnly
            && matches!(event_type, TURN_FAILED | TURN_CANCELLED);
        if !self.delivered || tool_only_failure {
            let notice = self.notice(event_type);
            if let Err(error) = self.post(notice).await {
                warn!(session_id = %self.session_id, %error, "channel: could not post the end-of-turn notice");
            }
        }
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
            let started = match self.adapter.streaming() {
                Some(stream) => stream.start(&self.context).await,
                None => return,
            };
            match started {
                Ok(handle) => {
                    self.streams.insert(
                        message_id.to_string(),
                        Stream {
                            handle,
                            text: String::new(),
                            sent: 0,
                        },
                    );
                }
                Err(error) => {
                    warn!(session_id = %self.session_id, %error, "channel: could not open a stream, posting the message whole");
                    self.unstreamable.insert(message_id.to_string());
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
            self.flush_stream(&id).await;
        }
    }

    async fn on_replaced(&mut self, data: &Value) {
        let (Some(message_id), Some(replacement)) = (
            data.get("message_id").and_then(Value::as_str),
            data.get("replacement").and_then(Value::as_str),
        ) else {
            return;
        };
        let Some(stream) = self.streams.remove(message_id) else {
            return;
        };
        let Some(streaming) = self.adapter.streaming() else {
            return;
        };
        // `replace` closes the stream. When it fails the stream is still open,
        // and every start must end in a stop.
        if let DeliveryResult::TransientError(error) | DeliveryResult::PermanentError(error) =
            streaming
                .replace(&stream.handle, replacement, &self.context)
                .await
        {
            warn!(session_id = %self.session_id, %error, "channel: could not replace a streamed message");
            let _ = streaming.stop(&stream.handle, &self.context).await;
        }
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
            self.close_stream(id).await;
            return;
        }

        if self.context.reply_mode != ChannelReplyMode::AllMessages {
            return;
        }
        let Some(text) = text else { return };
        match self.post(text).await {
            Ok(()) => {
                if let Some(id) = message_id {
                    self.done.insert(id);
                }
            }
            Err(error) => {
                warn!(session_id = %self.session_id, %error, "channel: could not post a reply");
            }
        }
    }

    async fn flush_stream(&mut self, message_id: &str) {
        let Some(streaming) = self.adapter.streaming() else {
            return;
        };
        let Some(stream) = self.streams.get_mut(message_id) else {
            return;
        };
        if stream.sent >= stream.text.len() {
            return;
        }
        let pending = stream.text[stream.sent..].to_string();
        match streaming
            .append(&stream.handle, &pending, &self.context)
            .await
        {
            DeliveryResult::Ok => {
                stream.sent = stream.text.len();
                self.delivered = true;
            }
            DeliveryResult::TransientError(error) | DeliveryResult::PermanentError(error) => {
                warn!(session_id = %self.session_id, %error, "channel: could not append to a stream");
            }
        }
    }

    async fn close_stream(&mut self, message_id: &str) {
        self.flush_stream(message_id).await;
        let Some(stream) = self.streams.remove(message_id) else {
            return;
        };
        self.done.insert(message_id.to_string());
        if let Some(streaming) = self.adapter.streaming() {
            if let DeliveryResult::TransientError(error) | DeliveryResult::PermanentError(error) =
                streaming.stop(&stream.handle, &self.context).await
            {
                warn!(session_id = %self.session_id, %error, "channel: could not stop a stream");
            }
            if stream.sent > 0 {
                self.delivered = true;
            }
        }
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
        match surface.set_status(&status, &self.context).await {
            DeliveryResult::Ok => self.last_status = Some(status),
            DeliveryResult::TransientError(error) | DeliveryResult::PermanentError(error) => {
                warn!(session_id = %self.session_id, %error, "channel: could not set the status");
            }
        }
    }

    async fn set_title(&self, title: &str) {
        let Some(surface) = self.agent_surface() else {
            return;
        };
        if let DeliveryResult::TransientError(error) | DeliveryResult::PermanentError(error) =
            surface.set_title(title, &self.context).await
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
        match approvals
            .prompt(self.session_id, &prompt, &self.context)
            .await
        {
            DeliveryResult::Ok => self.delivered = true,
            DeliveryResult::TransientError(error) | DeliveryResult::PermanentError(error) => {
                warn!(session_id = %self.session_id, %error, "channel: could not show an approval prompt");
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
            Some(handle) => match surface.update(&handle, &text, &self.context).await {
                DeliveryResult::Ok => {
                    self.progress.mark_updated();
                    self.delivered = true;
                }
                DeliveryResult::TransientError(error) | DeliveryResult::PermanentError(error) => {
                    warn!(session_id = %self.session_id, %error, "channel: could not update task progress");
                }
            },
            None => match surface.post(&text, &self.context).await {
                Ok(handle) => {
                    self.progress.mark_posted(handle);
                    self.delivered = true;
                }
                Err(error) => {
                    warn!(session_id = %self.session_id, %error, "channel: could not post task progress");
                }
            },
        }
    }

    async fn post(&mut self, text: String) -> Result<(), String> {
        let message = OutboundChannelMessage {
            session_id: self.session_id,
            text,
            thread_ref: self.context.thread_ref.clone(),
            correlation_id: Some(self.input_message_id.clone()),
        };
        match self.adapter.deliver(&message, &self.context).await {
            DeliveryResult::Ok => {
                self.delivered = true;
                Ok(())
            }
            DeliveryResult::TransientError(error) | DeliveryResult::PermanentError(error) => {
                Err(error)
            }
        }
    }

    fn notice(&self, event_type: &str) -> String {
        let headline = match event_type {
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

/// Whether a `tool.completed` is a successful `channel_post_message` receipt.
fn explicit_message_delivered(data: &Value) -> bool {
    if data.get("tool_name").and_then(Value::as_str) != Some(CHANNEL_POST_MESSAGE_TOOL_NAME)
        || data.get("success").and_then(Value::as_bool) != Some(true)
    {
        return false;
    }
    let Some(text) = data
        .get("result")
        .and_then(Value::as_array)
        .and_then(|parts| {
            parts.iter().find_map(|part| {
                (part.get("type")?.as_str()? == "text").then(|| part.get("text")?.as_str())?
            })
        })
    else {
        return false;
    };
    serde_json::from_str::<Value>(text).is_ok_and(|receipt| {
        receipt["delivered"] == true
            && receipt["message_ref"]
                .as_str()
                .is_some_and(|reference| !reference.is_empty())
    })
}

#[cfg(test)]
#[path = "delivery_tests.rs"]
mod tests;

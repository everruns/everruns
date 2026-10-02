//! Projection of Everruns runtime events onto an AG-UI 1.0 run.
//!
//! A [`Projector`] follows one run: feed it the session's runtime events (in
//! order, already filtered to the run) and drain the AG-UI events it queues.
//! It owns the 1.0 sequencing rules, so callers only decide what to expose
//! through [`ProjectionPolicy`].
//!
//! ```
//! use everruns_ag_ui::projection::{ProjectionPolicy, Projector};
//! use everruns_ag_ui::Event;
//!
//! let mut projector = Projector::new("thread-1", "run-1", ProjectionPolicy::default());
//! projector.project("turn.cancelled", &serde_json::json!({}));
//! let events: Vec<Event> = projector.drain().collect();
//! assert!(matches!(events.last(), Some(Event::RunFinished(_))));
//! assert!(projector.is_finished());
//! ```

// Message ids are the runtime message's bare UUID, the same id the server's
// `MESSAGES_SNAPSHOT` uses, so a replayed transcript and a live one agree.
//
// Decision: the projector takes `(event_type, data)` as JSON, the canonical
// envelope every Everruns surface already has (the server's durable events,
// the framework's `SessionEvent::canonical_json`), and parses the payloads
// with `everruns-core`'s own data types. One state machine then serves the
// server endpoint, `serve`, and the framework.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

use everruns_core::events::{
    OutputMessageCompletedData, OutputMessageDeltaData, ReasonItemData, TurnFailedData,
};
use everruns_core::{ContentPart, RuntimeMessage};
use everruns_provider::execution_phase::ExecutionPhase;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::{
    Event, Interrupt, ReasoningMessageContentEvent, ReasoningMessageEndEvent,
    ReasoningMessageStartEvent, ReasoningSpanEvent, RunErrorEvent, RunFinishedEvent,
    RunFinishedOutcome, TextMessageContentEvent, TextMessageEndEvent, TextMessageStartEvent,
    TokenUsage, ToolCall, ToolCallArgsEvent, ToolCallEndEvent, ToolCallStartEvent,
};

/// A failed turn, as handed to [`ProjectionPolicy::error`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TurnFailure {
    /// The runtime's error message. May carry provider detail.
    pub message: String,
    /// The runtime's stable error code, when it has one.
    pub code: Option<String>,
}

/// Builds the `RUN_ERROR` a failure becomes.
pub type ErrorProjection = Arc<dyn Fn(&TurnFailure) -> RunErrorEvent + Send + Sync>;

/// What a projection exposes.
#[derive(Clone)]
pub struct ProjectionPolicy {
    /// Stream model reasoning (`reason.thinking.*`) and provider reasoning
    /// summaries (`reason.item`) on the reasoning channel.
    pub reasoning_visible: bool,
    /// Text shown on the reasoning channel while server-side tools run, or
    /// `None` to show nothing. Never derived from tool names or arguments.
    pub tool_activity_text: Option<String>,
    /// Report the run's token usage on its terminal event, summed per
    /// provider and model from `llm.generation`.
    pub usage_visible: bool,
    /// Maps a failed turn to its `RUN_ERROR`. Public channels sanitize here.
    pub error: ErrorProjection,
}

impl Default for ProjectionPolicy {
    /// Trusted defaults: reasoning visible, no tool activity text, and the
    /// runtime's error message and code passed through.
    fn default() -> Self {
        Self {
            reasoning_visible: true,
            tool_activity_text: None,
            usage_visible: true,
            error: Arc::new(|failure: &TurnFailure| RunErrorEvent {
                code: failure.code.clone(),
                ..RunErrorEvent::new(failure.message.clone())
            }),
        }
    }
}

impl std::fmt::Debug for ProjectionPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProjectionPolicy")
            .field("reasoning_visible", &self.reasoning_visible)
            .field("tool_activity_text", &self.tool_activity_text)
            .field("usage_visible", &self.usage_visible)
            .finish_non_exhaustive()
    }
}

/// Follows one AG-UI run.
#[derive(Debug)]
pub struct Projector {
    policy: ProjectionPolicy,
    thread_id: String,
    run_id: String,
    queue: VecDeque<Event>,
    /// The open assistant text message, if any.
    assistant_message_id: Option<String>,
    assistant_content_started: bool,
    assistant_emitted_delta: bool,
    /// The last assistant message that carried tool calls rather than an
    /// answer: the parent of any frontend tool call the run stops on.
    tool_call_message_id: Option<String>,
    /// The open reasoning span and reasoning message, if any.
    reasoning_span: Option<String>,
    reasoning_message: Option<String>,
    /// The span was opened by tool activity rather than model reasoning.
    span_opened_by_tools: bool,
    /// Model reasoning (`reason.thinking.*`) is streaming.
    thinking: bool,
    active_tools: usize,
    tool_activity_shown: bool,
    next_reasoning_id: usize,
    /// Usage per (provider, model), ordered by that key so output is stable.
    usage: BTreeMap<(Option<String>, Option<String>), TokenUsage>,
    finished: bool,
}

impl Projector {
    pub fn new(
        thread_id: impl Into<String>,
        run_id: impl Into<String>,
        policy: ProjectionPolicy,
    ) -> Self {
        Self {
            policy,
            thread_id: thread_id.into(),
            run_id: run_id.into(),
            queue: VecDeque::new(),
            assistant_message_id: None,
            assistant_content_started: false,
            assistant_emitted_delta: false,
            tool_call_message_id: None,
            reasoning_span: None,
            reasoning_message: None,
            span_opened_by_tools: false,
            thinking: false,
            active_tools: 0,
            tool_activity_shown: false,
            next_reasoning_id: 0,
            usage: BTreeMap::new(),
            finished: false,
        }
    }

    /// Whether the run has ended (`RUN_FINISHED` or `RUN_ERROR` queued).
    /// Later input is ignored.
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// Takes the queued events.
    pub fn drain(&mut self) -> impl Iterator<Item = Event> + '_ {
        self.queue.drain(..)
    }

    /// Takes the next queued event.
    pub fn pop(&mut self) -> Option<Event> {
        self.queue.pop_front()
    }

    /// Ends the run with an error that did not come from the runtime, such as
    /// the event stream closing early. Open messages are closed first.
    pub fn fail(&mut self, error: RunErrorEvent) {
        if self.finished {
            return;
        }
        self.close_all();
        let mut error = error;
        if error.usage.is_none() {
            error.usage = self.usage_entries();
        }
        self.queue.push_back(Event::RunError(error));
        self.finished = true;
    }

    /// Ends the run with the interrupt outcome: it stopped to ask for
    /// something, such as an answer or an approval, and the run that resumes
    /// it carries the answers. An empty list does nothing, because an
    /// interrupt outcome with nothing to answer is invalid.
    pub fn interrupt(&mut self, interrupts: Vec<Interrupt>) {
        self.park(Vec::new(), interrupts);
    }

    /// Ends the run where the turn parked on its consumer.
    ///
    /// Frontend tool calls stream as `TOOL_CALL_START`/`ARGS`/`END` under the
    /// assistant message that made them. With interrupts the run ends with
    /// the interrupt outcome; with only tool calls it is a success naming
    /// them in `pendingToolCallIds`, and the consumer's next run carries
    /// their results as `tool` messages. Both empty does nothing.
    ///
    /// ```
    /// use everruns_ag_ui::projection::{ProjectionPolicy, Projector};
    /// use everruns_ag_ui::{Event, RunFinishedOutcome, ToolCall};
    ///
    /// let mut projector = Projector::new("thread", "run", ProjectionPolicy::default());
    /// projector.park(vec![ToolCall::function("call-1", "confirm", "{}")], Vec::new());
    /// let events: Vec<Event> = projector.drain().collect();
    /// assert!(matches!(events[0], Event::ToolCallStart(_)));
    /// let Some(Event::RunFinished(finished)) = events.last() else { panic!() };
    /// assert_eq!(
    ///     finished.outcome,
    ///     Some(RunFinishedOutcome::Success {
    ///         pending_tool_call_ids: Some(vec!["call-1".to_string()]),
    ///     })
    /// );
    /// ```
    pub fn park(&mut self, tool_calls: Vec<ToolCall>, interrupts: Vec<Interrupt>) {
        if self.finished || (tool_calls.is_empty() && interrupts.is_empty()) {
            return;
        }
        self.close_all();
        let parent = self.tool_call_message_id.clone();
        let mut pending = Vec::with_capacity(tool_calls.len());
        for call in tool_calls {
            let mut start = ToolCallStartEvent::new(call.id.clone(), call.function.name);
            start.parent_message_id = parent.clone();
            self.queue.push_back(Event::ToolCallStart(start));
            if !call.function.arguments.is_empty() {
                self.queue
                    .push_back(Event::ToolCallArgs(ToolCallArgsEvent::new(
                        call.id.clone(),
                        call.function.arguments,
                    )));
            }
            self.queue
                .push_back(Event::ToolCallEnd(ToolCallEndEvent::new(call.id.clone())));
            pending.push(call.id);
        }
        self.finish(Some(if interrupts.is_empty() {
            RunFinishedOutcome::Success {
                pending_tool_call_ids: Some(pending),
            }
        } else {
            RunFinishedOutcome::Interrupt { interrupts }
        }));
    }

    /// Projects one runtime event, given its dotted type and JSON `data`.
    pub fn project(&mut self, event_type: &str, data: &Value) {
        if self.finished {
            return;
        }
        match event_type {
            "output.message.delta" => {
                if let Some(data) = parse::<OutputMessageDeltaData>(data) {
                    let message_id =
                        self.ensure_assistant_message(data.message_id.uuid().to_string());
                    self.open_assistant_text(&message_id);
                    self.queue
                        .push_back(Event::TextMessageContent(TextMessageContentEvent::new(
                            message_id, data.delta,
                        )));
                    self.assistant_emitted_delta = true;
                }
            }
            "output.message.completed" => {
                if let Some(data) = parse::<OutputMessageCompletedData>(data) {
                    self.output_completed(&data.message);
                }
            }
            "reason.thinking.started" if self.policy.reasoning_visible => {
                if self.reasoning_span.is_none() {
                    self.open_span(false);
                }
                // Model reasoning takes over a span tool activity opened.
                self.span_opened_by_tools = false;
                self.close_reasoning_message();
                self.thinking = true;
            }
            "reason.thinking.delta" if self.policy.reasoning_visible => {
                if !self.thinking {
                    self.project("reason.thinking.started", &Value::Null);
                }
                if let Some(delta) = data.get("delta").and_then(Value::as_str) {
                    let message_id = self.ensure_reasoning_message();
                    self.queue.push_back(Event::ReasoningMessageContent(
                        ReasoningMessageContentEvent::new(message_id, delta),
                    ));
                }
            }
            "reason.thinking.completed" if self.policy.reasoning_visible => {
                self.close_reasoning_message();
                self.close_span();
                self.thinking = false;
                self.span_opened_by_tools = false;
            }
            // A provider `reason.item` summary is a reasoning artifact: it
            // renders on the reasoning channel, never as assistant text, and
            // its opaque encrypted content is never emitted.
            "reason.item" if self.policy.reasoning_visible => {
                if let Some(data) = parse::<ReasonItemData>(data) {
                    self.reasoning_summary(&data.summary);
                }
            }
            "tool.started" => {
                self.active_tools += 1;
                if let Some(text) = self.policy.tool_activity_text.clone() {
                    self.tool_activity(&text);
                }
            }
            "tool.completed" => {
                self.active_tools = self.active_tools.saturating_sub(1);
                if self.active_tools == 0 && self.tool_activity_shown {
                    if self.span_opened_by_tools {
                        self.close_reasoning_message();
                        self.close_span();
                    }
                    self.tool_activity_shown = false;
                    self.span_opened_by_tools = false;
                }
            }
            "llm.generation" => self.record_usage(data),
            "turn.completed" | "session.idled" => self.finish(None),
            // Cancellation is a deliberate terminal state, typically client
            // initiated, not a fault: 1.0 names it with the cancelled outcome.
            "turn.cancelled" => self.finish(Some(RunFinishedOutcome::Cancelled)),
            "turn.failed" => {
                let failure = parse::<TurnFailedData>(data)
                    .map(|data| TurnFailure {
                        message: data.error,
                        code: data.error_code,
                    })
                    .unwrap_or_default();
                let error = (self.policy.error)(&failure);
                self.fail(error);
            }
            _ => {}
        }
    }

    fn finish(&mut self, outcome: Option<RunFinishedOutcome>) {
        self.close_all();
        self.queue.push_back(Event::RunFinished(RunFinishedEvent {
            outcome,
            usage: self.usage_entries(),
            ..RunFinishedEvent::new(self.thread_id.clone(), self.run_id.clone())
        }));
        self.finished = true;
    }

    /// Adds one generation's usage to its provider and model's entry.
    ///
    /// Everruns counts prompt buckets disjointly (non-cached input, cache
    /// reads, cache writes); AG-UI's `inputTokens` is their sum, with the
    /// cache counts as parts of it.
    fn record_usage(&mut self, data: &Value) {
        let metadata = &data["metadata"];
        let Some(usage) = metadata.get("usage").filter(|usage| usage.is_object()) else {
            return;
        };
        let count = |key: &str| usage.get(key).and_then(Value::as_u64);
        let text = |key: &str| {
            metadata
                .get(key)
                .and_then(Value::as_str)
                .map(str::to_string)
        };
        let model = text("response_model").or_else(|| text("model"));
        let entry = self
            .usage
            .entry((text("provider"), model.clone()))
            .or_insert_with(|| TokenUsage {
                provider: text("provider"),
                model,
                ..TokenUsage::default()
            });
        let cache_read = count("cache_read_tokens");
        let cache_write = count("cache_creation_tokens");
        let input = count("input_tokens")
            .map(|input| input + cache_read.unwrap_or_default() + cache_write.unwrap_or_default());
        let add = |total: &mut Option<u64>, value: Option<u64>| {
            if let Some(value) = value {
                *total = Some(total.unwrap_or_default().saturating_add(value));
            }
        };
        add(&mut entry.input_tokens, input);
        add(&mut entry.output_tokens, count("output_tokens"));
        add(&mut entry.cached_input_tokens, cache_read);
        add(&mut entry.cache_write_input_tokens, cache_write);
        entry.total_tokens = match (entry.input_tokens, entry.output_tokens) {
            (Some(input), Some(output)) => Some(input.saturating_add(output)),
            _ => None,
        };
    }

    fn usage_entries(&self) -> Option<Vec<TokenUsage>> {
        (self.policy.usage_visible && !self.usage.is_empty())
            .then(|| self.usage.values().cloned().collect())
    }

    fn output_completed(&mut self, message: &RuntimeMessage) {
        if !is_terminal_public_output(message, self.assistant_emitted_delta) {
            // Commentary before tools: close its text message so the final
            // answer can open its own without ending the run early.
            self.close_assistant_text();
            self.tool_call_message_id = Some(message.id.uuid().to_string());
            return;
        }
        let message_id = self.ensure_assistant_message(message.id.uuid().to_string());
        self.open_assistant_text(&message_id);
        let text = public_text(&message.content);
        if !self.assistant_emitted_delta && !text.is_empty() {
            self.queue
                .push_back(Event::TextMessageContent(TextMessageContentEvent::new(
                    message_id.clone(),
                    text,
                )));
        }
        self.close_assistant_text();
        self.finish(None);
    }

    /// Switches to `message_id`, closing a different open assistant message.
    fn ensure_assistant_message(&mut self, message_id: String) -> String {
        if self.assistant_message_id.as_deref() != Some(message_id.as_str()) {
            self.close_assistant_text();
            self.assistant_message_id = Some(message_id.clone());
        }
        message_id
    }

    fn open_assistant_text(&mut self, message_id: &str) {
        if !self.assistant_content_started {
            self.queue
                .push_back(Event::TextMessageStart(TextMessageStartEvent::assistant(
                    message_id,
                )));
            self.assistant_content_started = true;
        }
    }

    fn close_assistant_text(&mut self) {
        if self.assistant_content_started
            && let Some(message_id) = self.assistant_message_id.clone()
        {
            self.queue
                .push_back(Event::TextMessageEnd(TextMessageEndEvent::new(message_id)));
        }
        self.assistant_message_id = None;
        self.assistant_content_started = false;
        self.assistant_emitted_delta = false;
    }

    fn next_id(&mut self, kind: &str) -> String {
        self.next_reasoning_id += 1;
        format!("{}-{kind}-{}", self.run_id, self.next_reasoning_id)
    }

    fn open_span(&mut self, by_tools: bool) {
        let id = self.next_id("reasoning");
        self.queue
            .push_back(Event::ReasoningStart(ReasoningSpanEvent::new(id.clone())));
        self.reasoning_span = Some(id);
        self.span_opened_by_tools = by_tools;
    }

    fn close_span(&mut self) {
        if let Some(id) = self.reasoning_span.take() {
            self.queue
                .push_back(Event::ReasoningEnd(ReasoningSpanEvent::new(id)));
        }
    }

    fn ensure_reasoning_message(&mut self) -> String {
        if let Some(id) = &self.reasoning_message {
            return id.clone();
        }
        let id = self.next_id("reasoning-message");
        self.queue.push_back(Event::ReasoningMessageStart(
            ReasoningMessageStartEvent::new(id.clone()),
        ));
        self.reasoning_message = Some(id.clone());
        id
    }

    fn close_reasoning_message(&mut self) {
        if let Some(id) = self.reasoning_message.take() {
            self.queue
                .push_back(Event::ReasoningMessageEnd(ReasoningMessageEndEvent::new(
                    id,
                )));
        }
    }

    /// Emits a self-contained reasoning message, or appends to the one model
    /// reasoning is streaming.
    fn reasoning_text(&mut self, text: &str) {
        if self.thinking && self.reasoning_message.is_some() {
            let id = self.ensure_reasoning_message();
            self.queue.push_back(Event::ReasoningMessageContent(
                ReasoningMessageContentEvent::new(id, format!("\n{text}")),
            ));
            return;
        }
        let opened_span = self.reasoning_span.is_none();
        if opened_span {
            self.open_span(false);
        }
        self.close_reasoning_message();
        let id = self.ensure_reasoning_message();
        self.queue.push_back(Event::ReasoningMessageContent(
            ReasoningMessageContentEvent::new(id, text),
        ));
        self.close_reasoning_message();
        if opened_span {
            self.close_span();
        }
    }

    fn reasoning_summary(&mut self, summary: &[String]) {
        let text = summary
            .iter()
            .map(|segment| segment.trim())
            .filter(|segment| !segment.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        if !text.is_empty() {
            self.reasoning_text(&text);
        }
    }

    /// Tool activity shows the policy's fixed text on the reasoning channel.
    /// It opens a span when none is open and keeps it open until the last
    /// running tool completes.
    fn tool_activity(&mut self, text: &str) {
        if !self.tool_activity_shown {
            if self.reasoning_span.is_none() {
                self.open_span(true);
            }
            self.tool_activity_shown = true;
        }
        if self.thinking && self.reasoning_message.is_some() {
            let id = self.ensure_reasoning_message();
            self.queue.push_back(Event::ReasoningMessageContent(
                ReasoningMessageContentEvent::new(id, format!("\n{text}")),
            ));
            return;
        }
        self.close_reasoning_message();
        let id = self.ensure_reasoning_message();
        self.queue.push_back(Event::ReasoningMessageContent(
            ReasoningMessageContentEvent::new(id, text),
        ));
        self.close_reasoning_message();
    }

    /// 1.0 fails a run that finishes with a message or span still open.
    fn close_all(&mut self) {
        self.close_assistant_text();
        self.close_reasoning_message();
        self.close_span();
        self.thinking = false;
        self.tool_activity_shown = false;
        self.span_opened_by_tools = false;
    }
}

fn parse<T: DeserializeOwned>(data: &Value) -> Option<T> {
    serde_json::from_value(data.clone()).ok()
}

/// Whether a completed output message is the run's public answer: not
/// commentary, not a tool-call carrier, and with something to show.
pub fn is_terminal_public_output(message: &RuntimeMessage, emitted_delta: bool) -> bool {
    if matches!(message.phase, Some(ExecutionPhase::Commentary)) {
        return false;
    }
    if message
        .content
        .iter()
        .any(|part| matches!(part, ContentPart::ToolCall(_) | ContentPart::ToolResult(_)))
    {
        return false;
    }
    emitted_delta || !public_text(&message.content).is_empty()
}

/// The text of content parts as an AG-UI string. Tool calls, tool results
/// and reasoning are other channels' content and are left out; media becomes
/// a short placeholder.
pub fn public_text(parts: &[ContentPart]) -> String {
    parts
        .iter()
        .filter_map(public_part_text)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn public_part_text(part: &ContentPart) -> Option<String> {
    match part {
        ContentPart::Text(text) => Some(text.text.clone()),
        ContentPart::Image(image) => {
            Some(image.url.clone().unwrap_or_else(|| "[Image]".to_string()))
        }
        ContentPart::ImageFile(image) => Some(format!(
            "[Image file: {}]",
            image.filename.as_deref().unwrap_or("unnamed")
        )),
        ContentPart::File(file) => Some(format!(
            "[File: {}]",
            file.filename.as_deref().unwrap_or("unnamed")
        )),
        // Tool calls, tool results and reasoning belong to other channels;
        // `ContentPart` is non-exhaustive, and a part this build cannot
        // project is omitted rather than rendered as an unknown marker.
        _ => None,
    }
}

//! Step three of the consumer pipeline: folding verified events into a
//! [`RunResult`].

use std::collections::{HashMap, HashSet};

use serde_json::Value;

use super::{ProtocolError, Verifier, decode_event};
use crate::ag_ui::{
    Content, Event, Interrupt, Message, PROTOCOL_VERSION, RunAgentInput, RunFinishedOutcome,
    TextMessageRole, TokenUsage,
};

/// A text message assembled from a run's events.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssembledMessage {
    pub id: String,
    pub role: TextMessageRole,
    /// The deltas concatenated in arrival order.
    pub content: String,
    /// The subagent that produced it; `None` is the agent itself.
    pub subagent_run_id: Option<String>,
}

/// A tool call assembled from a run's events.
#[derive(Clone, Debug, PartialEq)]
pub struct AssembledToolCall {
    pub id: String,
    pub name: String,
    /// The argument deltas concatenated in arrival order (JSON text).
    pub arguments: String,
    pub parent_message_id: Option<String>,
    /// The result, when the stream carried one (`TOOL_CALL_RESULT`).
    pub result: Option<Content>,
}

/// How the last run in the stream ended.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum RunOutcome {
    /// No run has ended yet.
    #[default]
    Pending,
    /// The run completed. Frontend tool calls it left unanswered are listed.
    Success { pending_tool_call_ids: Vec<String> },
    /// The run stopped to ask for something; see [`RunResult::interrupts`].
    Interrupted,
    /// The run was stopped on purpose.
    Cancelled,
    /// The agent reported that the run failed (`RUN_ERROR`).
    Failed {
        message: String,
        code: Option<String>,
    },
}

/// Everything a consumer learned from one run's stream.
///
/// A stream may carry several runs in sequence. Messages and tool calls
/// accumulate across them and usage is summed; the outcome, interrupts and
/// result describe the last run.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RunResult {
    pub thread_id: Option<String>,
    pub run_id: Option<String>,
    /// Text messages the agent produced, in order. Messages the request
    /// itself carried (history) are not repeated here.
    pub messages: Vec<AssembledMessage>,
    pub tool_calls: Vec<AssembledToolCall>,
    pub outcome: RunOutcome,
    /// The open interrupts of an [`RunOutcome::Interrupted`] run. A resuming
    /// run must answer every one; see [`crate::ag_ui::ResumeBuilder`].
    pub interrupts: Vec<Interrupt>,
    /// Token usage summed per provider and model.
    pub usage: Vec<TokenUsage>,
    /// The `RUN_FINISHED` result value, when the agent sent one.
    pub result: Option<Value>,
    /// The protocol version the producer declared, if it did.
    pub protocol_version: Option<String>,
    /// What the pipeline dropped or stripped, and version notices.
    pub warnings: Vec<String>,
}

impl RunResult {
    /// The text of the agent's own assistant messages (not its subagents'),
    /// joined by blank lines. This is what a caller usually shows as "the
    /// answer".
    pub fn text(&self) -> String {
        self.messages
            .iter()
            .filter(|m| m.role == TextMessageRole::Assistant && m.subagent_run_id.is_none())
            .map(|m| m.content.as_str())
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    /// The failure the agent reported, when the last run failed.
    pub fn error(&self) -> Option<(&str, Option<&str>)> {
        match &self.outcome {
            RunOutcome::Failed { message, code } => Some((message, code.as_deref())),
            _ => None,
        }
    }
}

/// The whole consumer pipeline over one response stream: decode, verify,
/// accumulate.
///
/// Feed it wire values with [`push_value`](Self::push_value) (or typed events
/// with [`push`](Self::push)), then call [`finish`](Self::finish) when the
/// stream ends.
///
/// ```
/// use everruns_core::ag_ui::consumer::{RunConsumer, RunOutcome};
/// use serde_json::json;
///
/// let mut consumer = RunConsumer::new();
/// for event in [
///     json!({ "type": "RUN_STARTED", "threadId": "t", "runId": "r" }),
///     json!({ "type": "TEXT_MESSAGE_CHUNK", "messageId": "m", "delta": "Hello" }),
///     json!({ "type": "RUN_FINISHED", "threadId": "t", "runId": "r",
///             "usage": [{ "model": "m1", "inputTokens": 10, "outputTokens": 2 }] }),
/// ] {
///     consumer.push_value(event).unwrap();
/// }
/// let result = consumer.finish().unwrap();
/// assert_eq!(result.text(), "Hello");
/// assert!(matches!(result.outcome, RunOutcome::Success { .. }));
/// assert_eq!(result.usage[0].input_tokens, Some(10));
/// ```
#[derive(Debug, Default)]
pub struct RunConsumer {
    verifier: Verifier,
    result: RunResult,
    message_index: HashMap<String, usize>,
    tool_call_index: HashMap<String, usize>,
    history_ids: HashSet<String>,
}

impl RunConsumer {
    /// A consumer with no request context.
    pub fn new() -> Self {
        Self::default()
    }

    /// A consumer for the response to `input`: messages the request carried
    /// are history, so a `MESSAGES_SNAPSHOT` restating them does not report
    /// them as new output.
    pub fn for_input(input: &RunAgentInput) -> Self {
        Self {
            history_ids: input.messages.iter().map(|m| m.id().to_owned()).collect(),
            ..Self::default()
        }
    }

    /// Decodes, verifies and applies one wire value, returning the events
    /// it expanded to (empty when it was dropped as unrecognised).
    pub fn push_value(&mut self, value: Value) -> Result<Vec<Event>, ProtocolError> {
        let decoded = decode_event(value)?;
        self.result.warnings.extend(decoded.warnings);
        match decoded.event {
            Some(event) => self.push(event),
            None => Ok(Vec::new()),
        }
    }

    /// Verifies and applies one typed event.
    pub fn push(&mut self, event: Event) -> Result<Vec<Event>, ProtocolError> {
        let events = self.verifier.push(event)?;
        for event in &events {
            self.apply(event);
        }
        Ok(events)
    }

    /// The result so far, without ending the stream.
    pub fn result(&self) -> &RunResult {
        &self.result
    }

    /// Ends the stream and returns the result. A stream that stops inside a
    /// run (no `RUN_FINISHED` or `RUN_ERROR`) is an error.
    pub fn finish(mut self) -> Result<RunResult, ProtocolError> {
        self.verifier.finish()?;
        Ok(self.result)
    }

    fn message_mut(&mut self, id: &str) -> Option<&mut AssembledMessage> {
        let index = *self.message_index.get(id)?;
        self.result.messages.get_mut(index)
    }

    fn tool_call_mut(&mut self, id: &str) -> Option<&mut AssembledToolCall> {
        let index = *self.tool_call_index.get(id)?;
        self.result.tool_calls.get_mut(index)
    }

    fn add_message(&mut self, message: AssembledMessage) {
        self.message_index
            .insert(message.id.clone(), self.result.messages.len());
        self.result.messages.push(message);
    }

    fn add_tool_call(&mut self, call: AssembledToolCall) {
        self.tool_call_index
            .insert(call.id.clone(), self.result.tool_calls.len());
        self.result.tool_calls.push(call);
    }

    fn apply(&mut self, event: &Event) {
        match event {
            Event::RunStarted(e) => {
                self.result.thread_id = Some(e.thread_id.clone());
                self.result.run_id = Some(e.run_id.clone());
                self.result.outcome = RunOutcome::Pending;
                self.result.interrupts.clear();
                self.result.result = None;
                if let Some(version) = &e.protocol_version {
                    if version != PROTOCOL_VERSION {
                        self.result.warnings.push(format!(
                            "producer declared protocol version '{version}'; proceeding as {PROTOCOL_VERSION}"
                        ));
                    }
                    self.result.protocol_version = Some(version.clone());
                }
            }
            Event::TextMessageStart(e) => {
                if self.message_index.contains_key(&e.message_id) {
                    return; // A reopen continues the message.
                }
                self.add_message(AssembledMessage {
                    id: e.message_id.clone(),
                    role: e.role,
                    content: String::new(),
                    subagent_run_id: e.subagent_run_id.clone(),
                });
            }
            Event::TextMessageContent(e) => {
                if let Some(message) = self.message_mut(&e.message_id) {
                    message.content.push_str(&e.delta);
                }
            }
            Event::ToolCallStart(e) => {
                if self.tool_call_index.contains_key(&e.tool_call_id) {
                    return;
                }
                self.add_tool_call(AssembledToolCall {
                    id: e.tool_call_id.clone(),
                    name: e.tool_call_name.clone(),
                    arguments: String::new(),
                    parent_message_id: e.parent_message_id.clone(),
                    result: None,
                });
            }
            Event::ToolCallArgs(e) => {
                if let Some(call) = self.tool_call_mut(&e.tool_call_id) {
                    call.arguments.push_str(&e.delta);
                }
            }
            Event::ToolCallResult(e) => {
                if let Some(call) = self.tool_call_mut(&e.tool_call_id) {
                    call.result = Some(e.content.clone());
                }
            }
            Event::MessagesSnapshot(e) => self.apply_snapshot(&e.messages),
            Event::RunFinished(e) => {
                merge_usage(&mut self.result.usage, e.usage.iter().flatten());
                self.result.result = e.result.clone();
                self.result.interrupts.clear();
                self.result.outcome = match &e.outcome {
                    None => RunOutcome::Success {
                        pending_tool_call_ids: Vec::new(),
                    },
                    Some(RunFinishedOutcome::Success {
                        pending_tool_call_ids,
                    }) => RunOutcome::Success {
                        pending_tool_call_ids: pending_tool_call_ids.clone().unwrap_or_default(),
                    },
                    Some(RunFinishedOutcome::Interrupt { interrupts }) => {
                        self.result.interrupts = interrupts.clone();
                        RunOutcome::Interrupted
                    }
                    Some(RunFinishedOutcome::Cancelled) => RunOutcome::Cancelled,
                };
            }
            Event::RunError(e) => {
                merge_usage(&mut self.result.usage, e.usage.iter().flatten());
                self.result.interrupts.clear();
                self.result.outcome = RunOutcome::Failed {
                    message: e.message.clone(),
                    code: e.code.clone(),
                };
            }
            _ => {}
        }
    }

    /// A snapshot restates the conversation: the messages it carries replace
    /// ours, and ones it omits are gone.
    fn apply_snapshot(&mut self, messages: &[Message]) {
        let mut kept_messages = Vec::new();
        let mut kept_calls = Vec::new();
        let mut results: HashMap<&str, &Content> = HashMap::new();
        for message in messages {
            if let Message::Tool(tool) = message {
                results.insert(&tool.tool_call_id, &tool.content);
            }
        }
        for message in messages {
            if self.history_ids.contains(message.id()) {
                continue;
            }
            let (role, content, subagent_run_id) = match message {
                Message::Assistant(m) => {
                    for call in m.tool_calls.iter().flatten() {
                        kept_calls.push(AssembledToolCall {
                            id: call.id.clone(),
                            name: call.function.name.clone(),
                            arguments: call.function.arguments.clone(),
                            parent_message_id: Some(m.id.clone()),
                            result: results.get(call.id.as_str()).map(|c| (*c).clone()),
                        });
                    }
                    (
                        TextMessageRole::Assistant,
                        m.content.clone().unwrap_or_default(),
                        m.subagent_run_id.clone(),
                    )
                }
                Message::User(m) => (
                    TextMessageRole::User,
                    m.content.to_text(),
                    m.subagent_run_id.clone(),
                ),
                Message::System(m) => (
                    TextMessageRole::System,
                    m.content.clone(),
                    m.subagent_run_id.clone(),
                ),
                Message::Developer(m) => (
                    TextMessageRole::Developer,
                    m.content.clone(),
                    m.subagent_run_id.clone(),
                ),
                Message::Tool(_) | Message::Activity(_) | Message::Reasoning(_) => continue,
            };
            kept_messages.push(AssembledMessage {
                id: message.id().to_owned(),
                role,
                content,
                subagent_run_id,
            });
        }
        self.result.messages.clear();
        self.message_index.clear();
        self.result.tool_calls.clear();
        self.tool_call_index.clear();
        for message in kept_messages {
            self.add_message(message);
        }
        for call in kept_calls {
            self.add_tool_call(call);
        }
    }
}

/// Adds `entries` into `total`, one entry per provider and model.
pub fn merge_usage<'a>(
    total: &mut Vec<TokenUsage>,
    entries: impl IntoIterator<Item = &'a TokenUsage>,
) {
    fn add(a: &mut Option<u64>, b: Option<u64>) {
        if let Some(b) = b {
            *a = Some(a.unwrap_or(0).saturating_add(b));
        }
    }
    for entry in entries {
        let slot = match total
            .iter_mut()
            .position(|t| t.provider == entry.provider && t.model == entry.model)
        {
            Some(index) => &mut total[index],
            None => {
                total.push(TokenUsage {
                    provider: entry.provider.clone(),
                    model: entry.model.clone(),
                    ..TokenUsage::default()
                });
                let last = total.len() - 1;
                &mut total[last]
            }
        };
        add(&mut slot.input_tokens, entry.input_tokens);
        add(&mut slot.output_tokens, entry.output_tokens);
        add(&mut slot.total_tokens, entry.total_tokens);
        add(&mut slot.reasoning_tokens, entry.reasoning_tokens);
        add(&mut slot.cached_input_tokens, entry.cached_input_tokens);
        add(
            &mut slot.cache_write_input_tokens,
            entry.cache_write_input_tokens,
        );
    }
}

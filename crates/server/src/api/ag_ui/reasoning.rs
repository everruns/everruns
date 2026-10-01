//! Projection onto the AG-UI 1.0 reasoning channel (`REASONING_*`).
//!
//! Three sources share the channel: provider thinking (`reason.thinking.*`),
//! provider reasoning summaries (`reason.item`), and public tool activity text.
//! All three render inside a reasoning span (`REASONING_START` ..
//! `REASONING_END`) as reasoning messages (`REASONING_MESSAGE_START` ..
//! `REASONING_MESSAGE_END`). 1.0 clients verify the sequence — every content or
//! end event must name an open id, and `RUN_FINISHED` is rejected while any
//! span or message is still open — so this module owns the ids and always
//! closes what it opened (see [`close_open_reasoning`]).
//!
//! Design Decision (EVE-1135): tool activity stays on the reasoning channel
//! rather than moving to 1.0 `ACTIVITY_*`. It is the channel-configured
//! generic text, which pre-1.0 consumers already render as reasoning, and
//! moving it would change what existing integrations display.

use super::AgUiStreamState;
use everruns_ag_ui::{
    Event, ReasoningMessageContentEvent, ReasoningMessageEndEvent, ReasoningMessageStartEvent,
    ReasoningSpanEvent,
};

/// Open reasoning span and message ids for one AG-UI run.
#[derive(Debug, Default)]
pub(super) struct ReasoningState {
    /// The open span, whichever source opened it.
    span_id: Option<String>,
    /// The open span belongs to a provider thinking block.
    thinking_open: bool,
    /// The open streamed thinking message. Implies an open span.
    message_id: Option<String>,
    /// Tool activity is in progress on this channel.
    tool_activity_started: bool,
    /// Tool activity (not thinking) opened the current span.
    tool_activity_opened_span: bool,
}

impl ReasoningState {
    /// No span or message is open.
    #[cfg(test)]
    pub(super) fn is_idle(&self) -> bool {
        self.span_id.is_none() && self.message_id.is_none()
    }
}

fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn open_span(state: &mut AgUiStreamState) {
    let span_id = new_id();
    state
        .queue
        .push_back(Event::ReasoningStart(ReasoningSpanEvent::new(
            span_id.clone(),
        )));
    state.reasoning.span_id = Some(span_id);
}

fn close_span(state: &mut AgUiStreamState) {
    if let Some(span_id) = state.reasoning.span_id.take() {
        state
            .queue
            .push_back(Event::ReasoningEnd(ReasoningSpanEvent::new(span_id)));
    }
}

fn start_message(state: &mut AgUiStreamState) -> String {
    let message_id = new_id();
    state.queue.push_back(Event::ReasoningMessageStart(
        ReasoningMessageStartEvent::new(message_id.clone()),
    ));
    message_id
}

fn push_content(state: &mut AgUiStreamState, message_id: String, delta: String) {
    state.queue.push_back(Event::ReasoningMessageContent(
        ReasoningMessageContentEvent::new(message_id, delta),
    ));
}

fn end_message(state: &mut AgUiStreamState, message_id: String) {
    state
        .queue
        .push_back(Event::ReasoningMessageEnd(ReasoningMessageEndEvent::new(
            message_id,
        )));
}

/// A self-contained reasoning message inside the currently open span.
fn push_standalone_message(state: &mut AgUiStreamState, text: String) {
    let message_id = start_message(state);
    push_content(state, message_id.clone(), text);
    end_message(state, message_id);
}

/// `reason.thinking.started`: open a span unless tool activity already holds
/// one, which the thinking block then shares.
pub(super) fn thinking_started(state: &mut AgUiStreamState) {
    if let Some(message_id) = state.reasoning.message_id.take() {
        end_message(state, message_id);
    }
    if state.reasoning.span_id.is_none() {
        open_span(state);
    }
    state.reasoning.thinking_open = true;
}

/// `reason.thinking.delta`: stream into the open thinking message, opening the
/// span and message on first use so a delta that arrives without a `started`
/// still produces a well-formed sequence.
pub(super) fn thinking_delta(state: &mut AgUiStreamState, delta: String) {
    if state.reasoning.span_id.is_none() {
        open_span(state);
    }
    state.reasoning.thinking_open = true;
    let message_id = match state.reasoning.message_id.clone() {
        Some(message_id) => message_id,
        None => {
            let message_id = start_message(state);
            state.reasoning.message_id = Some(message_id.clone());
            message_id
        }
    };
    push_content(state, message_id, delta);
}

/// `reason.thinking.completed`: close the thinking message and the span it
/// holds, including a span tool activity opened and thinking then shared.
pub(super) fn thinking_completed(state: &mut AgUiStreamState) {
    if let Some(message_id) = state.reasoning.message_id.take() {
        end_message(state, message_id);
    }
    if state.reasoning.thinking_open {
        close_span(state);
        state.reasoning.tool_activity_opened_span = false;
    }
    state.reasoning.thinking_open = false;
}

/// Public tool activity text. Appends to an open thinking message; otherwise
/// renders as its own reasoning message inside a span, opening one when none
/// is open and keeping it open until the last active tool completes.
pub(super) fn push_tool_activity_start(state: &mut AgUiStreamState, text: String) {
    if state.reasoning.span_id.is_none() {
        open_span(state);
        state.reasoning.tool_activity_opened_span = true;
    }
    state.reasoning.tool_activity_started = true;
    if let Some(message_id) = state.reasoning.message_id.clone() {
        push_content(state, message_id, format!("\n{text}"));
        return;
    }
    push_standalone_message(state, text);
}

/// Close the tool-activity span once no tool is active, unless a thinking
/// block now shares it (thinking closes it on completion).
pub(super) fn push_tool_activity_end(state: &mut AgUiStreamState) {
    if state.reasoning.tool_activity_started && state.active_tool_activity_count == 0 {
        if state.reasoning.tool_activity_opened_span && !state.reasoning.thinking_open {
            close_span(state);
        }
        state.reasoning.tool_activity_started = false;
        state.reasoning.tool_activity_opened_span = false;
    }
}

/// Project a provider `reason.item` summary onto the reasoning channel. Per the
/// EVE-768 design note the provider-authored summary is a reasoning artifact:
/// it must render on the reasoning channel and is never relabeled as an
/// assistant answer. Only the curated `summary` segments are surfaced here —
/// opaque/encrypted reasoning content is never emitted (no
/// `REASONING_ENCRYPTED_VALUE`).
///
/// An open thinking message absorbs the summary rather than gaining a nested
/// block; otherwise the summary is its own message, in a span of its own when
/// none is open.
pub(super) fn push_reasoning_summary(state: &mut AgUiStreamState, summary: &[String]) {
    let text = summary
        .iter()
        .map(|segment| segment.trim())
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    if text.is_empty() {
        return;
    }

    if let Some(message_id) = state.reasoning.message_id.clone() {
        push_content(state, message_id, format!("\n{text}"));
        return;
    }

    let opened_span = state.reasoning.span_id.is_none();
    if opened_span {
        open_span(state);
    }
    push_standalone_message(state, text);
    if opened_span {
        close_span(state);
    }
}

/// Close any open reasoning message and span. Called before `RUN_FINISHED`,
/// which 1.0 clients reject while reasoning is still open — a turn can end
/// without the `reason.thinking.completed` / `tool.completed` that would
/// otherwise close them.
pub(super) fn close_open_reasoning(state: &mut AgUiStreamState) {
    if let Some(message_id) = state.reasoning.message_id.take() {
        end_message(state, message_id);
    }
    close_span(state);
    state.reasoning = ReasoningState::default();
}

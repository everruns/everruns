// OpenTelemetry Event Listener
//
// Turns the agentic event stream into OpenTelemetry spans that follow the
// Gen-AI agent and inference conventions
// (https://github.com/open-telemetry/semantic-conventions-genai) and, on the
// same spans, the OpenInference conventions read by Arize Phoenix. See
// knowledge/operations/observability.md for the contract.
//
// One trace per turn:
//
//   invoke_agent {agent}      INTERNAL  turn.started → turn.completed/failed/cancelled
//   ├── reason                INTERNAL  reason.started → reason.completed (phase span)
//   │   └── chat {model}      CLIENT    llm.generation (real duration, see below)
//   │       └── thinking      INTERNAL  reason.thinking.started → completed
//   ├── act                   INTERNAL  act.started → act.completed (phase span)
//   │   ├── execute_tool {n}  INTERNAL  tool.started → tool.completed
//   │   └── execute_tool {n}
//   └── ...
//
// Spans are built with the OpenTelemetry API directly rather than through
// `tracing` spans: every event carries the timestamp of the fact it records,
// and only the API lets a span start (and end) at those timestamps rather than
// at listener time. `llm.generation` is one event emitted after the call
// returns, so the chat span is backdated by the call's duration; when extended
// thinking is on, the chat span opens at `reason.thinking.started` instead so
// the thinking span can nest inside the call it belongs to.
//
// Parenting follows the ids the engine already computes: `parent_span_id`
// first, then the phase that owns the event's `exec_id` (thinking events carry
// no span ids), then the turn. Content (instructions, messages, tool
// arguments and results, reasoning text) is opt-in via
// OTEL_INSTRUMENTATION_GENAI_CAPTURE_MESSAGE_CONTENT=true.

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use everruns_core::event_listeners::EventListener;
use everruns_core::events::{
    ACT_COMPLETED, ACT_STARTED, ActCompletedData, ActStartedData, Event, EventData, LLM_GENERATION,
    LlmGenerationData, REASON_COMPLETED, REASON_STARTED, REASON_THINKING_COMPLETED,
    REASON_THINKING_STARTED, ReasonCompletedData, ReasonStartedData, ReasonThinkingCompletedData,
    ReasonThinkingStartedData, TOOL_COMPLETED, TOOL_STARTED, TURN_CANCELLED, TURN_COMPLETED,
    TURN_FAILED, TURN_STARTED, TokenUsage, ToolCompletedData, ToolStartedData, TurnCancelledData,
    TurnCompletedData, TurnFailedData, TurnStartedData,
};
use everruns_core::message::ContentPart;
use everruns_core::telemetry::{
    chat_span_name, content, error_type, gen_ai, invoke_agent_span_name, tool_span_name,
};
use opentelemetry::global::{BoxedSpan, BoxedTracer};
use opentelemetry::trace::{Span, SpanKind, Status, TraceContextExt, Tracer};
use opentelemetry::{Context, KeyValue, StringValue, Value};

use super::openinference as oi;

// ============================================================================
// Configuration
// ============================================================================

/// Which attribute vocabularies the listener writes on its spans.
///
/// Both are on by default so one OTLP stream renders in Gen-AI-aware
/// backends and in Phoenix. Narrow with `EVERRUNS_TRACE_CONVENTIONS`
/// (`gen_ai`, `openinference`, or both, comma-separated).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TraceConventions {
    /// OpenTelemetry Gen-AI semantic conventions (`gen_ai.*`).
    pub gen_ai: bool,
    /// OpenInference conventions (`openinference.span.kind`, `llm.*`, ...).
    pub openinference: bool,
}

impl TraceConventions {
    pub const ALL: Self = Self {
        gen_ai: true,
        openinference: true,
    };
    pub const GEN_AI: Self = Self {
        gen_ai: true,
        openinference: false,
    };
    pub const OPENINFERENCE: Self = Self {
        gen_ai: false,
        openinference: true,
    };

    /// Environment variable selecting the conventions.
    pub const ENV: &'static str = "EVERRUNS_TRACE_CONVENTIONS";

    pub fn from_env() -> Self {
        std::env::var(Self::ENV)
            .ok()
            .map(|value| Self::parse(&value))
            .unwrap_or(Self::ALL)
    }

    /// Parse a comma-separated list. Unknown or empty selections fall back
    /// to every convention, so a typo widens rather than silences telemetry.
    pub fn parse(value: &str) -> Self {
        let mut selected = Self {
            gen_ai: false,
            openinference: false,
        };
        for token in value.split(',').map(|t| t.trim().to_ascii_lowercase()) {
            match token.as_str() {
                "gen_ai" | "genai" | "otel" | "opentelemetry" => selected.gen_ai = true,
                "openinference" | "oi" | "phoenix" => selected.openinference = true,
                "all" | "both" => return Self::ALL,
                "" => {}
                other => tracing::warn!(value = other, "Unknown {} entry, ignoring", Self::ENV),
            }
        }
        if selected
            == (Self {
                gen_ai: false,
                openinference: false,
            })
        {
            Self::ALL
        } else {
            selected
        }
    }
}

impl Default for TraceConventions {
    fn default() -> Self {
        Self::ALL
    }
}

/// Read the content-capture opt-in. Accepts the boolean spelling and the
/// mode names used by the reference Python instrumentation.
///
/// THREAT[TM-OBS-010]: prompts, completions, reasoning, and tool payloads only
/// reach the OTLP endpoint when the operator turns this on; every content
/// attribute below is gated on the resulting `record_content` flag.
fn record_content_from_env() -> bool {
    let value = std::env::var("OTEL_INSTRUMENTATION_GENAI_CAPTURE_MESSAGE_CONTENT")
        .or_else(|_| std::env::var("OTEL_RECORD_CONTENT"))
        .unwrap_or_default();
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "true" | "1" | "yes" | "on" | "span_only" | "span_and_event" | "event_only"
    )
}

// ============================================================================
// Internal state
// ============================================================================

/// Instrumentation scope name of every span the listener produces.
const TRACER_NAME: &str = "everruns";

/// Everruns-specific attributes, namespaced so they never collide with a
/// convention.
mod everruns_attr {
    pub const TURN_ID: &str = "everruns.turn.id";
    pub const EXEC_ID: &str = "everruns.exec.id";
    pub const INPUT_MESSAGE_ID: &str = "everruns.input_message.id";
    pub const HARNESS_ID: &str = "everruns.harness.id";
    pub const PHASE: &str = "everruns.phase";
    pub const TURN_ITERATIONS: &str = "everruns.turn.iterations";
    pub const TURN_TOOL_CALL_COUNT: &str = "everruns.turn.tool_call_count";
    pub const TURN_LLM_CALL_COUNT: &str = "everruns.turn.llm_call_count";
    pub const TURN_STATUS: &str = "everruns.turn.status";
    pub const REASON_TOOL_CALL_COUNT: &str = "everruns.reason.tool_call_count";
    pub const ACT_TOOL_CALL_COUNT: &str = "everruns.act.tool_call_count";
    pub const ACT_SUCCESS_COUNT: &str = "everruns.act.success_count";
    pub const ACT_ERROR_COUNT: &str = "everruns.act.error_count";
    pub const TOOL_STATUS: &str = "everruns.tool.status";
    pub const TOOL_CAPABILITY_ID: &str = "everruns.tool.capability.id";
    pub const TOOL_CAPABILITY_NAME: &str = "everruns.tool.capability.name";
    pub const LLM_RETRY_ATTEMPTS: &str = "everruns.llm.retry.attempts";
    pub const LLM_RETRY_WAIT_MS: &str = "everruns.llm.retry.total_wait_ms";
    pub const USAGE_COST_USD: &str = "everruns.usage.cost_usd";
    /// Set on a span the listener had to close because its turn ended first.
    pub const SPAN_UNTERMINATED: &str = "everruns.span.unterminated";
    /// Set on a span reconstructed from a terminal event whose start was
    /// never seen.
    pub const SPAN_ORPHANED: &str = "everruns.span.orphaned";
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SpanRole {
    Turn,
    Reason,
    Act,
    Thinking,
    Tool,
}

/// A chat span opened at `reason.thinking.started`, before the generation
/// record exists, so thinking nests inside the call.
struct PendingChat {
    span: BoxedSpan,
    cx: Context,
}

/// Per-reason state that bridges the thinking and generation events.
#[derive(Default)]
struct ReasonState {
    pending_chat: Option<PendingChat>,
    thinking_text: Option<String>,
}

struct ActiveSpan {
    span: BoxedSpan,
    /// Parent context for children of this span.
    cx: Context,
    role: SpanRole,
    turn_key: Option<String>,
    /// Alias keys (exec ids) registered for this span, dropped with it.
    aliases: Vec<String>,
    /// The reason span this thinking span belongs to.
    reason_key: Option<String>,
    reason: ReasonState,
}

#[derive(Default, Clone)]
struct AgentIdentity {
    id: Option<String>,
    name: Option<String>,
    description: Option<String>,
}

/// Per-turn state shared by the turn's spans.
#[derive(Default)]
struct TurnState {
    /// Keys of the turn's descendant spans that are still open.
    children: Vec<String>,
    agent: AgentIdentity,
    /// Tool descriptions seen on `llm.generation`, so `execute_tool` spans can
    /// carry `gen_ai.tool.description`.
    tool_descriptions: HashMap<String, String>,
}

#[derive(Default)]
struct ListenerState {
    spans: HashMap<String, ActiveSpan>,
    /// `exec:{id}` → span key of the phase that owns that exec.
    aliases: HashMap<String, String>,
    /// `turn:{id}` → per-turn state.
    turns: HashMap<String, TurnState>,
}

// ============================================================================
// OtelEventListener
// ============================================================================

/// OpenTelemetry event listener producing Gen-AI and OpenInference spans.
pub struct OtelEventListener {
    tracer: BoxedTracer,
    record_content: bool,
    conventions: TraceConventions,
    state: Mutex<ListenerState>,
}

impl Default for OtelEventListener {
    fn default() -> Self {
        Self::new()
    }
}

impl OtelEventListener {
    /// Listener on the globally installed tracer provider (see
    /// `init_telemetry`), configured from the environment.
    pub fn new() -> Self {
        Self::with_tracer(
            opentelemetry::global::tracer(TRACER_NAME),
            record_content_from_env(),
            TraceConventions::from_env(),
        )
    }

    /// Listener with an explicit content-recording setting, reading the
    /// tracer and conventions from the environment.
    pub fn with_record_content(record_content: bool) -> Self {
        Self::with_tracer(
            opentelemetry::global::tracer(TRACER_NAME),
            record_content,
            TraceConventions::from_env(),
        )
    }

    /// Listener on a specific tracer with explicit settings.
    pub fn with_tracer(
        tracer: BoxedTracer,
        record_content: bool,
        conventions: TraceConventions,
    ) -> Self {
        Self {
            tracer,
            record_content,
            conventions,
            state: Mutex::new(ListenerState::default()),
        }
    }

    pub fn record_content(&self) -> bool {
        self.record_content
    }

    pub fn conventions(&self) -> TraceConventions {
        self.conventions
    }

    // ------------------------------------------------------------------
    // Span construction helpers
    // ------------------------------------------------------------------

    fn start_span(
        &self,
        name: String,
        kind: SpanKind,
        start: SystemTime,
        attributes: Vec<KeyValue>,
        parent: Option<&Context>,
    ) -> (BoxedSpan, Context) {
        let builder = self
            .tracer
            .span_builder(name)
            .with_kind(kind)
            .with_start_time(start)
            .with_attributes(attributes);
        let root = Context::new();
        let span = self
            .tracer
            .build_with_context(builder, parent.unwrap_or(&root));
        let cx = Context::new().with_remote_span_context(span.span_context().clone());
        (span, cx)
    }

    /// Attributes every span carries: conversation/session correlation plus
    /// Everruns ids.
    fn common_attributes(&self, event: &Event, oi_kind: &'static str) -> Vec<KeyValue> {
        let session = event.session_id.to_string();
        let mut attrs = Vec::new();
        if self.conventions.gen_ai {
            attrs.push(KeyValue::new(gen_ai::CONVERSATION_ID, session.clone()));
        }
        if self.conventions.openinference {
            attrs.push(KeyValue::new(oi::SPAN_KIND, oi_kind));
            attrs.push(KeyValue::new(oi::SESSION_ID, session));
        }
        if let Some(turn_id) = &event.context.turn_id {
            attrs.push(KeyValue::new(everruns_attr::TURN_ID, turn_id.to_string()));
        }
        if let Some(exec_id) = &event.context.exec_id {
            attrs.push(KeyValue::new(everruns_attr::EXEC_ID, exec_id.to_string()));
        }
        if let Some(id) = &event.context.input_message_id {
            attrs.push(KeyValue::new(
                everruns_attr::INPUT_MESSAGE_ID,
                id.to_string(),
            ));
        }
        attrs
    }

    /// Mark a span failed per the conventions: `error.type`, error status
    /// with the message, and an `exception` event for backends that render
    /// those.
    fn record_failure(
        &self,
        span: &mut BoxedSpan,
        ts: SystemTime,
        code: Option<&str>,
        message: &str,
    ) {
        let kind = error_type(code, message);
        span.set_attribute(KeyValue::new(gen_ai::ERROR_TYPE, kind.clone()));
        span.set_status(Status::error(message.to_string()));
        span.add_event_with_timestamp(
            "exception",
            ts,
            vec![
                KeyValue::new(oi::EXCEPTION_TYPE, kind),
                KeyValue::new(oi::EXCEPTION_MESSAGE, message.to_string()),
            ],
        );
    }

    // ------------------------------------------------------------------
    // Key and parent resolution
    // ------------------------------------------------------------------

    fn turn_key(event: &Event) -> Option<String> {
        event.context.turn_id.map(|id| format!("turn:{id}"))
    }

    /// Key for a started/completed pair: the engine's span id when present,
    /// else the family scoped to the turn.
    fn phase_key(event: &Event, family: &str) -> String {
        if let Some(span_id) = &event.context.span_id {
            format!("span:{span_id}")
        } else if let Some(turn_id) = &event.context.turn_id {
            format!("{family}:{turn_id}")
        } else {
            format!("{family}:{}", event.id)
        }
    }

    /// The open span an event nests under: its `parent_span_id`, else the
    /// phase owning its `exec_id`, else its turn.
    fn parent_key(state: &ListenerState, event: &Event) -> Option<String> {
        if let Some(parent) = &event.context.parent_span_id {
            let key = format!("span:{parent}");
            if state.spans.contains_key(&key) {
                return Some(key);
            }
            // The engine names the turn root by its turn id.
            let turn_key = format!("turn:{parent}");
            if state.spans.contains_key(&turn_key) {
                return Some(turn_key);
            }
        }
        if let Some(exec_id) = &event.context.exec_id
            && let Some(key) = state.aliases.get(&format!("exec:{exec_id}"))
            && state.spans.contains_key(key)
        {
            return Some(key.clone());
        }
        Self::turn_key(event).filter(|key| state.spans.contains_key(key))
    }

    fn parent_cx(state: &ListenerState, key: Option<&String>) -> Option<Context> {
        key.and_then(|k| state.spans.get(k)).map(|s| s.cx.clone())
    }

    fn register(
        state: &mut ListenerState,
        key: String,
        mut active: ActiveSpan,
        exec_alias: Option<String>,
    ) {
        if let Some(alias) = exec_alias {
            state.aliases.insert(alias.clone(), key.clone());
            active.aliases.push(alias);
        }
        if let Some(turn_key) = &active.turn_key
            && let Some(turn) = state.turns.get_mut(turn_key)
        {
            turn.children.push(key.clone());
        }
        state.spans.insert(key, active);
    }

    fn take(state: &mut ListenerState, key: &str) -> Option<ActiveSpan> {
        let active = state.spans.remove(key)?;
        for alias in &active.aliases {
            state.aliases.remove(alias);
        }
        if let Some(turn_key) = &active.turn_key
            && let Some(turn) = state.turns.get_mut(turn_key)
        {
            turn.children.retain(|k| k != key);
        }
        Some(active)
    }

    // ------------------------------------------------------------------
    // Turn lifecycle → invoke_agent
    // ------------------------------------------------------------------

    fn handle_turn_started(&self, event: &Event, data: &TurnStartedData) {
        let ts = event_time(event);
        let turn_key = format!("turn:{}", data.turn_id);
        let agent = AgentIdentity {
            id: data.agent_id.map(|id| id.to_string()),
            name: data.agent_name.clone(),
            description: data.agent_description.clone(),
        };

        let mut attrs = self.common_attributes(event, oi::span_kind::AGENT);
        if self.conventions.gen_ai {
            attrs.push(KeyValue::new(
                gen_ai::OPERATION_NAME,
                gen_ai::operation::INVOKE_AGENT,
            ));
            if let Some(id) = &agent.id {
                attrs.push(KeyValue::new(gen_ai::AGENT_ID, id.clone()));
            }
            if let Some(name) = &agent.name {
                attrs.push(KeyValue::new(gen_ai::AGENT_NAME, name.clone()));
            }
            if let Some(description) = &agent.description {
                attrs.push(KeyValue::new(
                    gen_ai::AGENT_DESCRIPTION,
                    description.clone(),
                ));
            }
            if self.record_content
                && let Some(input) = &data.input_content
            {
                let messages = serde_json::json!([{
                    "role": gen_ai::role::USER,
                    "parts": [{ "type": gen_ai::part_type::TEXT, "content": input }],
                }]);
                attrs.push(KeyValue::new(gen_ai::INPUT_MESSAGES, messages.to_string()));
            }
        }
        if self.conventions.openinference {
            if let Some(name) = &agent.name {
                attrs.push(KeyValue::new(oi::AGENT_NAME, name.clone()));
            }
            if self.record_content
                && let Some(input) = &data.input_content
            {
                attrs.push(KeyValue::new(oi::INPUT_VALUE, input.clone()));
                attrs.push(KeyValue::new(oi::INPUT_MIME_TYPE, oi::mime::TEXT));
            }
            let metadata = serde_json::json!({
                "turn_id": data.turn_id.to_string(),
                "input_message_id": data.input_message_id.to_string(),
                "agent_id": agent.id,
            });
            attrs.push(KeyValue::new(oi::METADATA, metadata.to_string()));
        }
        if event.context.turn_id.is_none() {
            attrs.push(KeyValue::new(
                everruns_attr::TURN_ID,
                data.turn_id.to_string(),
            ));
        }

        let (span, cx) = self.start_span(
            invoke_agent_span_name(agent.name.as_deref()),
            SpanKind::Internal,
            ts,
            attrs,
            None,
        );

        let mut state = self.state.lock().unwrap();
        state.turns.insert(
            turn_key.clone(),
            TurnState {
                agent,
                ..TurnState::default()
            },
        );
        state.spans.insert(
            turn_key,
            ActiveSpan {
                span,
                cx,
                role: SpanRole::Turn,
                turn_key: None,
                aliases: Vec::new(),
                reason_key: None,
                reason: ReasonState::default(),
            },
        );
    }

    fn turn_usage_attributes(&self, usage: &TokenUsage) -> Vec<KeyValue> {
        let mut attrs = Vec::new();
        if self.conventions.gen_ai {
            attrs.push(KeyValue::new(
                gen_ai::USAGE_INPUT_TOKENS,
                i64::from(usage.input_tokens),
            ));
            attrs.push(KeyValue::new(
                gen_ai::USAGE_OUTPUT_TOKENS,
                i64::from(usage.output_tokens),
            ));
            if let Some(read) = usage.cache_read_tokens {
                attrs.push(KeyValue::new(
                    gen_ai::USAGE_CACHE_READ_INPUT_TOKENS,
                    i64::from(read),
                ));
            }
            if let Some(write) = usage.cache_creation_tokens {
                attrs.push(KeyValue::new(
                    gen_ai::USAGE_CACHE_WRITE_INPUT_TOKENS,
                    i64::from(write),
                ));
            }
        }
        if self.conventions.openinference {
            attrs.extend(oi_token_attributes(usage));
        }
        if let Some(cost) = cost_usd(usage) {
            attrs.push(KeyValue::new(everruns_attr::USAGE_COST_USD, cost));
        }
        attrs
    }

    /// Close the turn root and anything still open under it.
    fn finish_turn(
        &self,
        event: &Event,
        turn_id: &str,
        mut attrs: Vec<KeyValue>,
        failure: Option<(Option<&str>, &str)>,
        orphan_name: &str,
    ) {
        let ts = event_time(event);
        let turn_key = format!("turn:{turn_id}");
        let mut state = self.state.lock().unwrap();
        let turn_state = state.turns.remove(&turn_key).unwrap_or_default();
        for child_key in turn_state.children {
            if let Some(mut child) = Self::take(&mut state, &child_key) {
                if let Some(pending) = child.reason.pending_chat.take() {
                    let mut chat = pending.span;
                    chat.set_attribute(KeyValue::new(everruns_attr::SPAN_UNTERMINATED, true));
                    chat.end_with_timestamp(ts);
                }
                child
                    .span
                    .set_attribute(KeyValue::new(everruns_attr::SPAN_UNTERMINATED, true));
                child.span.end_with_timestamp(ts);
            }
        }
        match state.spans.remove(&turn_key) {
            Some(mut active) => {
                active.span.set_attributes(attrs);
                if let Some((code, message)) = failure {
                    self.record_failure(&mut active.span, ts, code, message);
                }
                active.span.end_with_timestamp(ts);
            }
            None => {
                // Terminal event without a start: reconstruct a point span so
                // the outcome is still visible.
                attrs.push(KeyValue::new(everruns_attr::SPAN_ORPHANED, true));
                attrs.extend(self.common_attributes(event, oi::span_kind::AGENT));
                if self.conventions.gen_ai {
                    attrs.push(KeyValue::new(
                        gen_ai::OPERATION_NAME,
                        gen_ai::operation::INVOKE_AGENT,
                    ));
                }
                let (mut span, _) =
                    self.start_span(orphan_name.to_string(), SpanKind::Internal, ts, attrs, None);
                if let Some((code, message)) = failure {
                    self.record_failure(&mut span, ts, code, message);
                }
                span.end_with_timestamp(ts);
            }
        }
    }

    fn handle_turn_completed(&self, event: &Event, data: &TurnCompletedData) {
        let mut attrs = vec![KeyValue::new(
            everruns_attr::TURN_ITERATIONS,
            i64::from(data.iterations),
        )];
        if let Some(count) = data.tool_call_count {
            attrs.push(KeyValue::new(
                everruns_attr::TURN_TOOL_CALL_COUNT,
                i64::from(count),
            ));
        }
        if let Some(count) = data.llm_call_count {
            attrs.push(KeyValue::new(
                everruns_attr::TURN_LLM_CALL_COUNT,
                i64::from(count),
            ));
        }
        if let Some(status) = &data.status {
            attrs.push(KeyValue::new(everruns_attr::TURN_STATUS, status.clone()));
        }
        if let Some(usage) = &data.usage {
            attrs.extend(self.turn_usage_attributes(usage));
        }
        if self.record_content
            && let Some(answer) = &data.final_answer_preview
        {
            if self.conventions.gen_ai {
                let messages = content::output_messages(Some(answer), &[], None, None);
                attrs.push(KeyValue::new(gen_ai::OUTPUT_MESSAGES, messages.to_string()));
            }
            if self.conventions.openinference {
                attrs.push(KeyValue::new(oi::OUTPUT_VALUE, answer.clone()));
                attrs.push(KeyValue::new(oi::OUTPUT_MIME_TYPE, oi::mime::TEXT));
            }
        }
        self.finish_turn(
            event,
            &data.turn_id.to_string(),
            attrs,
            None,
            &invoke_agent_span_name(None),
        );
    }

    fn handle_turn_failed(&self, event: &Event, data: &TurnFailedData) {
        self.finish_turn(
            event,
            &data.turn_id.to_string(),
            vec![KeyValue::new(everruns_attr::TURN_STATUS, "failed")],
            Some((data.error_code.as_deref(), &data.error)),
            &invoke_agent_span_name(None),
        );
    }

    fn handle_turn_cancelled(&self, event: &Event, data: &TurnCancelledData) {
        let mut attrs = vec![KeyValue::new(everruns_attr::TURN_STATUS, "cancelled")];
        if let Some(usage) = &data.usage {
            attrs.extend(self.turn_usage_attributes(usage));
        }
        let message = data
            .reason
            .clone()
            .unwrap_or_else(|| "cancelled".to_string());
        self.finish_turn(
            event,
            &data.turn_id.to_string(),
            attrs,
            Some((Some("cancelled"), &message)),
            &invoke_agent_span_name(None),
        );
    }

    // ------------------------------------------------------------------
    // Phase spans: reason / act
    // ------------------------------------------------------------------

    fn start_phase(&self, event: &Event, role: SpanRole, mut attrs: Vec<KeyValue>) {
        let ts = event_time(event);
        let family = match role {
            SpanRole::Reason => "reason",
            SpanRole::Act => "act",
            _ => unreachable!("phase spans are reason or act"),
        };
        let key = Self::phase_key(event, family);
        attrs.extend(self.common_attributes(event, oi::span_kind::CHAIN));
        attrs.push(KeyValue::new(everruns_attr::PHASE, family));

        let mut state = self.state.lock().unwrap();
        let turn_key = Self::turn_key(event).filter(|k| state.spans.contains_key(k));
        let parent_cx = Self::parent_cx(&state, turn_key.as_ref());
        let (span, cx) = self.start_span(
            family.to_string(),
            SpanKind::Internal,
            ts,
            attrs,
            parent_cx.as_ref(),
        );
        let exec_alias = event.context.exec_id.map(|id| format!("exec:{id}"));
        Self::register(
            &mut state,
            key,
            ActiveSpan {
                span,
                cx,
                role,
                turn_key,
                aliases: Vec::new(),
                reason_key: None,
                reason: ReasonState::default(),
            },
            exec_alias,
        );
    }

    fn handle_reason_started(&self, event: &Event, data: &ReasonStartedData) {
        let mut attrs = vec![KeyValue::new(
            everruns_attr::HARNESS_ID,
            data.harness_id.to_string(),
        )];
        if self.conventions.gen_ai
            && let Some(agent_id) = &data.agent_id
        {
            attrs.push(KeyValue::new(gen_ai::AGENT_ID, agent_id.to_string()));
        }
        self.start_phase(event, SpanRole::Reason, attrs);
    }

    fn handle_reason_completed(&self, event: &Event, data: &ReasonCompletedData) {
        let ts = event_time(event);
        let key = Self::phase_key(event, "reason");
        let mut state = self.state.lock().unwrap();
        let Some(mut active) = Self::take(&mut state, &key) else {
            drop(state);
            self.orphan_phase(event, "reason", data.duration_ms, data.error.as_deref());
            return;
        };
        drop(state);

        // A chat span opened for thinking but never finalized by a generation
        // record: close it with the phase so the trace stays well-formed.
        if let Some(pending) = active.reason.pending_chat.take() {
            let mut chat = pending.span;
            let message = data
                .error
                .as_deref()
                .unwrap_or("reason phase ended without a generation record");
            self.record_failure(&mut chat, ts, None, message);
            chat.end_with_timestamp(ts);
        }

        active.span.set_attribute(KeyValue::new(
            everruns_attr::REASON_TOOL_CALL_COUNT,
            i64::from(data.tool_call_count),
        ));
        if !data.success {
            let message = data.error.as_deref().unwrap_or("reason phase failed");
            self.record_failure(&mut active.span, ts, None, message);
        }
        active.span.end_with_timestamp(ts);
    }

    fn handle_act_started(&self, event: &Event, data: &ActStartedData) {
        let attrs = vec![KeyValue::new(
            everruns_attr::ACT_TOOL_CALL_COUNT,
            data.tool_calls.len() as i64,
        )];
        self.start_phase(event, SpanRole::Act, attrs);
    }

    fn handle_act_completed(&self, event: &Event, data: &ActCompletedData) {
        let ts = event_time(event);
        let key = Self::phase_key(event, "act");
        let mut state = self.state.lock().unwrap();
        let Some(mut active) = Self::take(&mut state, &key) else {
            drop(state);
            self.orphan_phase(event, "act", data.duration_ms, None);
            return;
        };
        drop(state);
        active.span.set_attributes(vec![
            KeyValue::new(
                everruns_attr::ACT_SUCCESS_COUNT,
                i64::from(data.success_count),
            ),
            KeyValue::new(everruns_attr::ACT_ERROR_COUNT, i64::from(data.error_count)),
        ]);
        if !data.completed {
            self.record_failure(&mut active.span, ts, None, "act phase interrupted");
        }
        active.span.end_with_timestamp(ts);
    }

    /// A phase terminal event without a start: a span reconstructed from the
    /// reported duration.
    fn orphan_phase(
        &self,
        event: &Event,
        family: &'static str,
        duration_ms: Option<u64>,
        error: Option<&str>,
    ) {
        let ts = event_time(event);
        let start = backdate(ts, duration_ms);
        let mut attrs = self.common_attributes(event, oi::span_kind::CHAIN);
        attrs.push(KeyValue::new(everruns_attr::PHASE, family));
        attrs.push(KeyValue::new(everruns_attr::SPAN_ORPHANED, true));
        let state = self.state.lock().unwrap();
        let turn_key = Self::turn_key(event);
        let parent_cx = Self::parent_cx(&state, turn_key.as_ref());
        drop(state);
        let (mut span, _) = self.start_span(
            family.to_string(),
            SpanKind::Internal,
            start,
            attrs,
            parent_cx.as_ref(),
        );
        if let Some(error) = error {
            self.record_failure(&mut span, ts, None, error);
        }
        span.end_with_timestamp(ts);
    }

    // ------------------------------------------------------------------
    // Thinking (nested in the chat span it belongs to)
    // ------------------------------------------------------------------

    fn handle_thinking_started(&self, event: &Event, data: &ReasonThinkingStartedData) {
        let ts = event_time(event);
        let key = Self::phase_key(event, "thinking");
        let mut state = self.state.lock().unwrap();
        let reason_key = Self::parent_key(&state, event)
            .filter(|k| state.spans.get(k).map(|s| s.role) == Some(SpanRole::Reason));
        let turn_key = Self::turn_key(event).filter(|k| state.spans.contains_key(k));

        // Open the chat span now so thinking nests inside the model call.
        let parent_cx = match reason_key.as_ref().and_then(|k| state.spans.get_mut(k)) {
            Some(reason) => {
                if reason.reason.pending_chat.is_none() {
                    let model = data.model.as_deref().unwrap_or("unknown");
                    let attrs = self.chat_base_attributes(event);
                    let (span, cx) = self.start_span(
                        chat_span_name(model),
                        SpanKind::Client,
                        ts,
                        attrs,
                        Some(&reason.cx),
                    );
                    reason.reason.pending_chat = Some(PendingChat { span, cx });
                }
                reason
                    .reason
                    .pending_chat
                    .as_ref()
                    .map(|pending| pending.cx.clone())
            }
            None => Self::parent_cx(&state, turn_key.as_ref()),
        };

        let mut attrs = self.common_attributes(event, oi::span_kind::CHAIN);
        attrs.push(KeyValue::new(everruns_attr::PHASE, "thinking"));
        let (span, cx) = self.start_span(
            "thinking".to_string(),
            SpanKind::Internal,
            ts,
            attrs,
            parent_cx.as_ref(),
        );
        Self::register(
            &mut state,
            key,
            ActiveSpan {
                span,
                cx,
                role: SpanRole::Thinking,
                turn_key,
                aliases: Vec::new(),
                reason_key,
                reason: ReasonState::default(),
            },
            None,
        );
    }

    fn handle_thinking_completed(&self, event: &Event, data: &ReasonThinkingCompletedData) {
        let ts = event_time(event);
        let key = Self::phase_key(event, "thinking");
        let mut state = self.state.lock().unwrap();
        let Some(mut active) = Self::take(&mut state, &key) else {
            return;
        };
        // Keep the text for the chat span's output message (reasoning part).
        if let Some(reason_key) = &active.reason_key
            && let Some(reason) = state.spans.get_mut(reason_key)
        {
            reason.reason.thinking_text = Some(data.thinking.clone());
        }
        drop(state);
        if self.record_content && self.conventions.openinference {
            active.span.set_attributes(vec![
                KeyValue::new(oi::OUTPUT_VALUE, data.thinking.clone()),
                KeyValue::new(oi::OUTPUT_MIME_TYPE, oi::mime::TEXT),
            ]);
        }
        active.span.end_with_timestamp(ts);
    }

    // ------------------------------------------------------------------
    // LLM generation → chat
    // ------------------------------------------------------------------

    /// Attributes known before the generation record exists.
    fn chat_base_attributes(&self, event: &Event) -> Vec<KeyValue> {
        let mut attrs = self.common_attributes(event, oi::span_kind::LLM);
        if self.conventions.gen_ai {
            attrs.push(KeyValue::new(
                gen_ai::OPERATION_NAME,
                gen_ai::operation::CHAT,
            ));
        }
        attrs
    }

    /// Everything the generation record tells us about the call.
    fn chat_detail_attributes(
        &self,
        data: &LlmGenerationData,
        reasoning_text: Option<&str>,
    ) -> Vec<KeyValue> {
        let meta = &data.metadata;
        let driver_id = meta.provider.as_deref().unwrap_or("unknown");
        let mut attrs = Vec::new();

        if self.conventions.gen_ai {
            let provider = gen_ai::provider::from_driver_id(driver_id).to_string();
            attrs.push(KeyValue::new(gen_ai::PROVIDER_NAME, provider.clone()));
            attrs.push(KeyValue::new(gen_ai::SYSTEM, provider));
            attrs.push(KeyValue::new(gen_ai::REQUEST_MODEL, meta.model.clone()));
            attrs.push(KeyValue::new(gen_ai::RESPONSE_MODEL, meta.model.clone()));
            if let Some(id) = &meta.response_id {
                attrs.push(KeyValue::new(gen_ai::RESPONSE_ID, id.clone()));
            }
            if let Some(reasons) = &meta.finish_reasons
                && !reasons.is_empty()
            {
                let reasons: Vec<StringValue> = reasons
                    .iter()
                    .map(|r| StringValue::from(r.clone()))
                    .collect();
                attrs.push(KeyValue::new(
                    gen_ai::RESPONSE_FINISH_REASONS,
                    Value::Array(reasons.into()),
                ));
            }
            if let Some(usage) = &meta.usage {
                attrs.push(KeyValue::new(
                    gen_ai::USAGE_INPUT_TOKENS,
                    i64::from(usage.input_tokens),
                ));
                attrs.push(KeyValue::new(
                    gen_ai::USAGE_OUTPUT_TOKENS,
                    i64::from(usage.output_tokens),
                ));
                if let Some(read) = usage.cache_read_tokens {
                    attrs.push(KeyValue::new(
                        gen_ai::USAGE_CACHE_READ_INPUT_TOKENS,
                        i64::from(read),
                    ));
                }
                if let Some(write) = usage.cache_creation_tokens {
                    attrs.push(KeyValue::new(
                        gen_ai::USAGE_CACHE_WRITE_INPUT_TOKENS,
                        i64::from(write),
                    ));
                }
            }
            if let Some(options) = &meta.request_options {
                if let Some(temperature) = options.temperature {
                    attrs.push(KeyValue::new(
                        gen_ai::REQUEST_TEMPERATURE,
                        f64::from(temperature),
                    ));
                }
                if let Some(max_tokens) = options.max_tokens {
                    attrs.push(KeyValue::new(
                        gen_ai::REQUEST_MAX_TOKENS,
                        i64::from(max_tokens),
                    ));
                }
                if let Some(level) = &options.reasoning_effort {
                    attrs.push(KeyValue::new(
                        gen_ai::REQUEST_REASONING_LEVEL,
                        level.clone(),
                    ));
                }
                if let Some(stream) = options.stream {
                    attrs.push(KeyValue::new(gen_ai::REQUEST_STREAM, stream));
                }
            }
            if let Some(ttft) = meta.time_to_first_token_ms {
                attrs.push(KeyValue::new(
                    gen_ai::RESPONSE_TIME_TO_FIRST_CHUNK,
                    ttft as f64 / 1000.0,
                ));
            }
            if meta.compaction.is_some() {
                attrs.push(KeyValue::new(gen_ai::CONVERSATION_COMPACTED, true));
            }
            if self.record_content {
                if let Some(instructions) = content::system_instructions(&data.messages) {
                    attrs.push(KeyValue::new(
                        gen_ai::SYSTEM_INSTRUCTIONS,
                        instructions.to_string(),
                    ));
                }
                attrs.push(KeyValue::new(
                    gen_ai::INPUT_MESSAGES,
                    content::input_messages(&data.messages).to_string(),
                ));
                if meta.success {
                    let finish_reason = meta
                        .finish_reasons
                        .as_ref()
                        .and_then(|r| r.first())
                        .map(String::as_str);
                    attrs.push(KeyValue::new(
                        gen_ai::OUTPUT_MESSAGES,
                        content::output_messages(
                            data.output.text.as_deref(),
                            &data.output.tool_calls,
                            reasoning_text,
                            finish_reason,
                        )
                        .to_string(),
                    ));
                }
                if !data.tools.is_empty() {
                    attrs.push(KeyValue::new(
                        gen_ai::TOOL_DEFINITIONS,
                        content::tool_definitions(&data.tools).to_string(),
                    ));
                }
            }
        }

        if self.conventions.openinference {
            let (provider, system) = oi::provider_and_system(driver_id);
            attrs.push(KeyValue::new(oi::LLM_MODEL_NAME, meta.model.clone()));
            attrs.push(KeyValue::new(oi::LLM_PROVIDER, provider.to_string()));
            if let Some(system) = system {
                attrs.push(KeyValue::new(oi::LLM_SYSTEM, system));
            }
            if let Some(usage) = &meta.usage {
                attrs.extend(oi_token_attributes(usage));
                if let Some(cost) = cost_usd(usage) {
                    attrs.push(KeyValue::new(oi::LLM_COST_TOTAL, cost));
                }
            }
            let mut invocation = serde_json::Map::new();
            invocation.insert("model".to_string(), serde_json::json!(meta.model));
            if let Some(options) = &meta.request_options {
                if let Some(t) = options.temperature {
                    invocation.insert("temperature".to_string(), serde_json::json!(t));
                }
                if let Some(m) = options.max_tokens {
                    invocation.insert("max_tokens".to_string(), serde_json::json!(m));
                }
                if let Some(effort) = &options.reasoning_effort {
                    invocation.insert("reasoning_effort".to_string(), serde_json::json!(effort));
                }
                if let Some(stream) = options.stream {
                    invocation.insert("stream".to_string(), serde_json::json!(stream));
                }
            }
            attrs.push(KeyValue::new(
                oi::LLM_INVOCATION_PARAMETERS,
                serde_json::Value::Object(invocation).to_string(),
            ));
            for (i, tool) in data.tools.iter().enumerate() {
                attrs.push(oi::tool_attributes(i, &tool.name, &tool.description));
            }
            if self.record_content {
                let input = serde_json::json!({
                    "messages": data
                        .messages
                        .iter()
                        .map(content::message_json)
                        .collect::<Vec<_>>(),
                });
                attrs.push(KeyValue::new(oi::INPUT_VALUE, input.to_string()));
                attrs.push(KeyValue::new(oi::INPUT_MIME_TYPE, oi::mime::JSON));
                for (i, message) in data.messages.iter().enumerate() {
                    attrs.extend(oi::input_message_attributes(i, message));
                }
                if meta.success {
                    let output = content::output_messages(
                        data.output.text.as_deref(),
                        &data.output.tool_calls,
                        reasoning_text,
                        None,
                    );
                    attrs.push(KeyValue::new(oi::OUTPUT_VALUE, output.to_string()));
                    attrs.push(KeyValue::new(oi::OUTPUT_MIME_TYPE, oi::mime::JSON));
                    attrs.extend(oi::output_message_attributes(
                        data.output.text.as_deref(),
                        &data.output.tool_calls,
                    ));
                }
            }
        }

        if let Some(retry) = &meta.retry {
            attrs.push(KeyValue::new(
                everruns_attr::LLM_RETRY_ATTEMPTS,
                i64::from(retry.attempts),
            ));
            attrs.push(KeyValue::new(
                everruns_attr::LLM_RETRY_WAIT_MS,
                retry.total_wait_ms as i64,
            ));
        }
        if let Some(cost) = meta.usage.as_ref().and_then(cost_usd) {
            attrs.push(KeyValue::new(everruns_attr::USAGE_COST_USD, cost));
        }
        attrs
    }

    fn handle_llm_generation(&self, event: &Event, data: &LlmGenerationData) {
        let ts = event_time(event);
        let mut state = self.state.lock().unwrap();

        // Remember tool descriptions for the turn's execute_tool spans.
        if let Some(turn_key) = Self::turn_key(event)
            && let Some(turn) = state.turns.get_mut(&turn_key)
        {
            for tool in &data.tools {
                turn.tool_descriptions
                    .insert(tool.name.clone(), tool.description.clone());
            }
        }

        let parent_key = Self::parent_key(&state, event);
        let (pending, reasoning_text) =
            match parent_key.as_ref().and_then(|k| state.spans.get_mut(k)) {
                Some(parent) if parent.role == SpanRole::Reason => (
                    parent.reason.pending_chat.take(),
                    parent.reason.thinking_text.take(),
                ),
                _ => (None, None),
            };
        let parent_cx = Self::parent_cx(&state, parent_key.as_ref());
        drop(state);

        let name = chat_span_name(&data.metadata.model);
        let detail = self.chat_detail_attributes(data, reasoning_text.as_deref());
        let mut span = match pending {
            Some(pending) => {
                let mut span = pending.span;
                span.update_name(name);
                span.set_attributes(detail);
                span
            }
            None => {
                let mut attrs = self.chat_base_attributes(event);
                attrs.extend(detail);
                let start = backdate(ts, data.metadata.duration_ms);
                let (span, _) =
                    self.start_span(name, SpanKind::Client, start, attrs, parent_cx.as_ref());
                span
            }
        };
        if !data.metadata.success {
            let message = data
                .metadata
                .error
                .as_deref()
                .unwrap_or("LLM generation failed");
            self.record_failure(&mut span, ts, None, message);
        }
        span.end_with_timestamp(ts);
    }

    // ------------------------------------------------------------------
    // Tool lifecycle → execute_tool
    // ------------------------------------------------------------------

    fn tool_attributes(
        &self,
        event: &Event,
        name: &str,
        call_id: &str,
        description: Option<&str>,
        agent_name: Option<&str>,
    ) -> Vec<KeyValue> {
        let mut attrs = self.common_attributes(event, oi::span_kind::TOOL);
        if self.conventions.gen_ai {
            attrs.push(KeyValue::new(
                gen_ai::OPERATION_NAME,
                gen_ai::operation::EXECUTE_TOOL,
            ));
            attrs.push(KeyValue::new(gen_ai::TOOL_NAME, name.to_string()));
            attrs.push(KeyValue::new(
                gen_ai::TOOL_TYPE,
                gen_ai::tool_type::FUNCTION,
            ));
            attrs.push(KeyValue::new(gen_ai::TOOL_CALL_ID, call_id.to_string()));
            if let Some(description) = description {
                attrs.push(KeyValue::new(
                    gen_ai::TOOL_DESCRIPTION,
                    description.to_string(),
                ));
            }
            if let Some(agent_name) = agent_name {
                attrs.push(KeyValue::new(gen_ai::AGENT_NAME, agent_name.to_string()));
            }
        }
        if self.conventions.openinference {
            attrs.push(KeyValue::new(oi::TOOL_NAME, name.to_string()));
            if let Some(description) = description {
                attrs.push(KeyValue::new(oi::TOOL_DESCRIPTION, description.to_string()));
            }
        }
        attrs
    }

    fn handle_tool_started(&self, event: &Event, data: &ToolStartedData) {
        let ts = event_time(event);
        let key = format!("tool:{}", data.tool_call.id);
        let mut state = self.state.lock().unwrap();
        let turn_key = Self::turn_key(event).filter(|k| state.spans.contains_key(k));
        let (description, agent_name) = turn_key
            .as_ref()
            .and_then(|k| state.turns.get(k))
            .map(|turn| {
                (
                    turn.tool_descriptions.get(&data.tool_call.name).cloned(),
                    turn.agent.name.clone(),
                )
            })
            .unwrap_or_default();
        let mut attrs = self.tool_attributes(
            event,
            &data.tool_call.name,
            &data.tool_call.id,
            description.as_deref(),
            agent_name.as_deref(),
        );
        if self.record_content {
            let arguments = data.tool_call.arguments.to_string();
            if self.conventions.gen_ai {
                attrs.push(KeyValue::new(
                    gen_ai::TOOL_CALL_ARGUMENTS,
                    arguments.clone(),
                ));
            }
            if self.conventions.openinference {
                attrs.push(KeyValue::new(oi::INPUT_VALUE, arguments));
                attrs.push(KeyValue::new(oi::INPUT_MIME_TYPE, oi::mime::JSON));
            }
        }
        let parent_key = Self::parent_key(&state, event);
        let parent_cx = Self::parent_cx(&state, parent_key.as_ref());
        let (span, cx) = self.start_span(
            tool_span_name(&data.tool_call.name),
            SpanKind::Internal,
            ts,
            attrs,
            parent_cx.as_ref(),
        );
        Self::register(
            &mut state,
            key,
            ActiveSpan {
                span,
                cx,
                role: SpanRole::Tool,
                turn_key,
                aliases: Vec::new(),
                reason_key: None,
                reason: ReasonState::default(),
            },
            None,
        );
    }

    fn handle_tool_completed(&self, event: &Event, data: &ToolCompletedData) {
        let ts = event_time(event);
        let key = format!("tool:{}", data.tool_call_id);
        let mut state = self.state.lock().unwrap();
        let mut span = match Self::take(&mut state, &key) {
            Some(active) => {
                drop(state);
                active.span
            }
            None => {
                // Completed without a start: reconstruct from the duration.
                let turn_key = Self::turn_key(event).filter(|k| state.spans.contains_key(k));
                let (description, agent_name) = turn_key
                    .as_ref()
                    .and_then(|k| state.turns.get(k))
                    .map(|turn| {
                        (
                            turn.tool_descriptions.get(&data.tool_name).cloned(),
                            turn.agent.name.clone(),
                        )
                    })
                    .unwrap_or_default();
                let mut attrs = self.tool_attributes(
                    event,
                    &data.tool_name,
                    &data.tool_call_id,
                    description.as_deref(),
                    agent_name.as_deref(),
                );
                attrs.push(KeyValue::new(everruns_attr::SPAN_ORPHANED, true));
                let parent_key = Self::parent_key(&state, event);
                let parent_cx = Self::parent_cx(&state, parent_key.as_ref());
                drop(state);
                let (span, _) = self.start_span(
                    tool_span_name(&data.tool_name),
                    SpanKind::Internal,
                    backdate(ts, data.duration_ms),
                    attrs,
                    parent_cx.as_ref(),
                );
                span
            }
        };

        let mut attrs = vec![KeyValue::new(
            everruns_attr::TOOL_STATUS,
            data.status.clone(),
        )];
        if let Some(id) = &data.capability_id {
            attrs.push(KeyValue::new(everruns_attr::TOOL_CAPABILITY_ID, id.clone()));
        }
        if let Some(name) = &data.capability_name {
            attrs.push(KeyValue::new(
                everruns_attr::TOOL_CAPABILITY_NAME,
                name.clone(),
            ));
        }
        if self.record_content
            && let Some(result) = &data.result
        {
            let (value, mime) = tool_result_value(result);
            if self.conventions.gen_ai {
                attrs.push(KeyValue::new(gen_ai::TOOL_CALL_RESULT, value.clone()));
            }
            if self.conventions.openinference {
                attrs.push(KeyValue::new(oi::OUTPUT_VALUE, value));
                attrs.push(KeyValue::new(oi::OUTPUT_MIME_TYPE, mime));
            }
        }
        span.set_attributes(attrs);
        if !data.success {
            let message = data.error.as_deref().unwrap_or("tool call failed");
            self.record_failure(&mut span, ts, Some(&data.status), message);
        }
        span.end_with_timestamp(ts);
    }

    /// Number of tracked in-flight spans (for tests).
    #[cfg(test)]
    fn active_span_count(&self) -> usize {
        self.state.lock().unwrap().spans.len()
    }
}

// ============================================================================
// Free helpers
// ============================================================================

fn event_time(event: &Event) -> SystemTime {
    SystemTime::from(event.ts)
}

fn backdate(ts: SystemTime, duration_ms: Option<u64>) -> SystemTime {
    duration_ms
        .and_then(|ms| ts.checked_sub(Duration::from_millis(ms)))
        .unwrap_or(ts)
}

fn cost_usd(usage: &TokenUsage) -> Option<f64> {
    usage
        .effective_cost_usd
        .or(usage.actual_cost_usd)
        .or(usage.estimated_cost_usd)
}

fn oi_token_attributes(usage: &TokenUsage) -> Vec<KeyValue> {
    let prompt = i64::from(usage.input_tokens);
    let completion = i64::from(usage.output_tokens);
    let mut attrs = vec![
        KeyValue::new(oi::LLM_TOKEN_COUNT_PROMPT, prompt),
        KeyValue::new(oi::LLM_TOKEN_COUNT_COMPLETION, completion),
        KeyValue::new(oi::LLM_TOKEN_COUNT_TOTAL, prompt + completion),
    ];
    if let Some(read) = usage.cache_read_tokens {
        attrs.push(KeyValue::new(
            oi::LLM_TOKEN_COUNT_PROMPT_CACHE_READ,
            i64::from(read),
        ));
    }
    if let Some(write) = usage.cache_creation_tokens {
        attrs.push(KeyValue::new(
            oi::LLM_TOKEN_COUNT_PROMPT_CACHE_WRITE,
            i64::from(write),
        ));
    }
    attrs
}

/// A tool result as one attribute value: a lone text part stays plain text,
/// anything else is the JSON of the spec-shaped parts.
fn tool_result_value(result: &[ContentPart]) -> (String, &'static str) {
    if let [ContentPart::Text(text)] = result {
        return (text.text.clone(), oi::mime::TEXT);
    }
    let parts: Vec<serde_json::Value> = result.iter().filter_map(content::part_json).collect();
    (serde_json::Value::Array(parts).to_string(), oi::mime::JSON)
}

#[async_trait]
impl EventListener for OtelEventListener {
    async fn on_event(&self, event: &Event) {
        match &event.data {
            EventData::TurnStarted(data) => self.handle_turn_started(event, data),
            EventData::TurnCompleted(data) => self.handle_turn_completed(event, data),
            EventData::TurnFailed(data) => self.handle_turn_failed(event, data),
            EventData::TurnCancelled(data) => self.handle_turn_cancelled(event, data),
            EventData::ReasonStarted(data) => self.handle_reason_started(event, data),
            EventData::ReasonCompleted(data) => self.handle_reason_completed(event, data),
            EventData::ReasonThinkingStarted(data) => self.handle_thinking_started(event, data),
            EventData::ReasonThinkingCompleted(data) => {
                self.handle_thinking_completed(event, data);
            }
            EventData::LlmGeneration(data) => self.handle_llm_generation(event, data),
            EventData::ActStarted(data) => self.handle_act_started(event, data),
            EventData::ActCompleted(data) => self.handle_act_completed(event, data),
            EventData::ToolStarted(data) => self.handle_tool_started(event, data),
            EventData::ToolCompleted(data) => self.handle_tool_completed(event, data),
            _ => {}
        }
    }

    fn event_types(&self) -> Option<Vec<&'static str>> {
        Some(vec![
            TURN_STARTED,
            TURN_COMPLETED,
            TURN_FAILED,
            TURN_CANCELLED,
            REASON_STARTED,
            REASON_COMPLETED,
            REASON_THINKING_STARTED,
            REASON_THINKING_COMPLETED,
            LLM_GENERATION,
            ACT_STARTED,
            ACT_COMPLETED,
            TOOL_STARTED,
            TOOL_COMPLETED,
        ])
    }

    fn name(&self) -> &'static str {
        "OtelEventListener"
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Duration as ChronoDuration, Utc};
    use everruns_core::events::{
        EventContext, LlmGenerationMetadata, LlmGenerationOutput, LlmRequestOptions,
        ToolDefinitionSummary,
    };
    use everruns_core::message::Message;
    use everruns_provider::tool_types::ToolCall;
    use everruns_provider::typed_id::{AgentId, ExecId, HarnessId, MessageId, SessionId, TurnId};
    use opentelemetry::trace::{SpanId, TracerProvider as _};
    use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider, SpanData};
    use serde_json::json;

    struct Harness {
        listener: OtelEventListener,
        exporter: InMemorySpanExporter,
        _provider: SdkTracerProvider,
        session: SessionId,
        turn: TurnId,
        input_message: MessageId,
        t0: DateTime<Utc>,
    }

    impl Harness {
        fn new(record_content: bool, conventions: TraceConventions) -> Self {
            let exporter = InMemorySpanExporter::default();
            let provider = SdkTracerProvider::builder()
                .with_simple_exporter(exporter.clone())
                .build();
            let tracer = BoxedTracer::new(Box::new(provider.tracer("test")));
            Self {
                listener: OtelEventListener::with_tracer(tracer, record_content, conventions),
                exporter,
                _provider: provider,
                session: SessionId::new(),
                turn: TurnId::new(),
                input_message: MessageId::new(),
                t0: Utc::now(),
            }
        }

        fn at(&self, ms: i64) -> DateTime<Utc> {
            self.t0 + ChronoDuration::milliseconds(ms)
        }

        fn context(
            &self,
            exec: Option<ExecId>,
            span: Option<&str>,
            parent: Option<&str>,
        ) -> EventContext {
            EventContext {
                turn_id: Some(self.turn),
                input_message_id: Some(self.input_message),
                exec_id: exec,
                trace_id: Some(self.turn.to_string()),
                span_id: span.map(str::to_string),
                parent_span_id: parent.map(str::to_string),
            }
        }

        async fn emit(&self, ms: i64, ctx: EventContext, data: impl Into<EventData>) {
            let mut event = Event::new(self.session, ctx, data);
            event.ts = self.at(ms);
            self.listener.on_event(&event).await;
        }

        fn spans(&self) -> Vec<SpanData> {
            self.exporter.get_finished_spans().unwrap()
        }

        fn turn_started(&self) -> TurnStartedData {
            TurnStartedData {
                turn_id: self.turn,
                input_message_id: self.input_message,
                input_content: Some("What is the weather in Paris?".to_string()),
                agent_id: Some(AgentId::new()),
                agent_name: Some("Weather Helper".to_string()),
                agent_description: Some("Answers weather questions".to_string()),
            }
        }
    }

    fn attr<'a>(span: &'a SpanData, key: &str) -> Option<&'a Value> {
        span.attributes
            .iter()
            .find(|kv| kv.key.as_str() == key)
            .map(|kv| &kv.value)
    }

    fn attr_str(span: &SpanData, key: &str) -> Option<String> {
        attr(span, key).map(|v| v.to_string())
    }

    fn by_name<'a>(spans: &'a [SpanData], name: &str) -> &'a SpanData {
        spans
            .iter()
            .find(|s| s.name == name)
            .unwrap_or_else(|| panic!("no span named {name}"))
    }

    fn id(span: &SpanData) -> SpanId {
        span.span_context.span_id()
    }

    fn usage(input: u32, output: u32) -> TokenUsage {
        TokenUsage {
            input_tokens: input,
            output_tokens: output,
            cache_read_tokens: Some(40),
            cache_creation_tokens: Some(8),
            actual_cost_usd: Some(0.0125),
            estimated_cost_usd: None,
            effective_cost_usd: None,
        }
    }

    fn generation(success: bool) -> LlmGenerationData {
        let call = ToolCall {
            id: "call_1".to_string(),
            name: "get_weather".to_string(),
            arguments: json!({ "city": "Paris" }),
        };
        LlmGenerationData {
            messages: vec![
                Message::system("You are a weather assistant."),
                Message::user("What is the weather in Paris?"),
            ],
            tools: vec![ToolDefinitionSummary {
                name: "get_weather".to_string(),
                display_name: None,
                category: None,
                capability_id: None,
                capability_name: None,
                description: "Look up current weather".to_string(),
            }],
            output: LlmGenerationOutput {
                text: if success {
                    Some("Let me check.".to_string())
                } else {
                    None
                },
                tool_calls: if success { vec![call] } else { vec![] },
            },
            metadata: LlmGenerationMetadata {
                model: "claude-sonnet-4-5".to_string(),
                provider: Some("anthropic".to_string()),
                usage: Some(usage(120, 30)),
                duration_ms: Some(500),
                time_to_first_token_ms: Some(250),
                success,
                error: (!success).then(|| "provider returned 503".to_string()),
                finish_reasons: Some(vec!["tool_calls".to_string()]),
                response_id: Some("msg_01".to_string()),
                retry: None,
                compaction: None,
                request_options: Some(LlmRequestOptions {
                    temperature: Some(0.2),
                    max_tokens: Some(1024),
                    reasoning_effort: Some("high".to_string()),
                    stream: Some(true),
                    ..LlmRequestOptions::default()
                }),
            },
        }
    }

    fn tool_started() -> ToolStartedData {
        ToolStartedData {
            tool_call: ToolCall {
                id: "call_1".to_string(),
                name: "get_weather".to_string(),
                arguments: json!({ "city": "Paris" }),
            },
            tool_call_fingerprint: None,
            display_name: None,
            narration: None,
        }
    }

    /// The full agentic loop with extended thinking, as the engine emits it.
    async fn run_full_turn(h: &Harness) {
        let reason_exec = ExecId::new();
        let act_exec = ExecId::new();
        h.emit(0, h.context(None, None, None), h.turn_started())
            .await;
        h.emit(
            10,
            h.context(Some(reason_exec), Some("r1"), Some(&h.turn.to_string())),
            ReasonStartedData {
                harness_id: HarnessId::new(),
                agent_id: None,
                metadata: None,
            },
        )
        .await;
        // Thinking events carry exec context only, no span ids.
        h.emit(
            100,
            h.context(Some(reason_exec), None, None),
            ReasonThinkingStartedData {
                turn_id: h.turn,
                model: Some("claude-sonnet-4-5".to_string()),
            },
        )
        .await;
        h.emit(
            300,
            h.context(Some(reason_exec), None, None),
            ReasonThinkingCompletedData {
                turn_id: h.turn,
                thinking: "Paris needs a lookup.".to_string(),
            },
        )
        .await;
        h.emit(
            600,
            h.context(Some(reason_exec), Some("g1"), Some("r1")),
            generation(true),
        )
        .await;
        h.emit(
            610,
            h.context(Some(reason_exec), Some("r1"), Some(&h.turn.to_string())),
            ReasonCompletedData {
                success: true,
                text_preview: Some("Let me check.".to_string()),
                has_tool_calls: true,
                tool_call_count: 1,
                error: None,
                duration_ms: Some(600),
                usage: Some(usage(120, 30)),
            },
        )
        .await;
        h.emit(
            620,
            h.context(Some(act_exec), Some("a1"), Some(&h.turn.to_string())),
            ActStartedData {
                tool_calls: vec![],
                headline: None,
            },
        )
        .await;
        h.emit(
            630,
            h.context(Some(act_exec), Some("t1"), Some("a1")),
            tool_started(),
        )
        .await;
        h.emit(
            700,
            h.context(Some(act_exec), Some("t1"), Some("a1")),
            ToolCompletedData::success(
                "call_1".to_string(),
                "get_weather".to_string(),
                vec![ContentPart::text("rainy, 14C")],
                Some(70),
            ),
        )
        .await;
        h.emit(
            710,
            h.context(Some(act_exec), Some("a1"), Some(&h.turn.to_string())),
            ActCompletedData {
                completed: true,
                success_count: 1,
                error_count: 0,
                duration_ms: Some(90),
                headline: None,
            },
        )
        .await;
        h.emit(
            800,
            h.context(None, None, None),
            TurnCompletedData {
                turn_id: h.turn,
                iterations: 1,
                duration_ms: Some(800),
                usage: Some(usage(120, 30)),
                input_content: None,
                final_message_id: None,
                final_answer_preview: Some("It is rainy in Paris.".to_string()),
                time_to_first_token_ms: Some(250),
                tool_call_count: Some(1),
                llm_call_count: Some(1),
                status: Some("completed".to_string()),
            },
        )
        .await;
    }

    #[tokio::test]
    async fn full_turn_nests_spans_per_the_conventions() {
        let h = Harness::new(false, TraceConventions::ALL);
        run_full_turn(&h).await;
        let spans = h.spans();
        assert_eq!(h.listener.active_span_count(), 0);
        assert_eq!(
            spans.len(),
            6,
            "{:?}",
            spans.iter().map(|s| s.name.clone()).collect::<Vec<_>>()
        );

        let turn = by_name(&spans, "invoke_agent Weather Helper");
        let reason = by_name(&spans, "reason");
        let chat = by_name(&spans, "chat claude-sonnet-4-5");
        let thinking = by_name(&spans, "thinking");
        let act = by_name(&spans, "act");
        let tool = by_name(&spans, "execute_tool get_weather");

        // Parenting: turn → reason → chat → thinking; turn → act → tool.
        assert_eq!(turn.parent_span_id, SpanId::INVALID);
        assert_eq!(reason.parent_span_id, id(turn));
        assert_eq!(chat.parent_span_id, id(reason));
        assert_eq!(thinking.parent_span_id, id(chat));
        assert_eq!(act.parent_span_id, id(turn));
        assert_eq!(tool.parent_span_id, id(act));
        for span in &spans {
            assert_eq!(span.span_context.trace_id(), turn.span_context.trace_id());
        }

        // Kinds.
        assert_eq!(turn.span_kind, SpanKind::Internal);
        assert_eq!(chat.span_kind, SpanKind::Client);
        assert_eq!(tool.span_kind, SpanKind::Internal);

        // Timing comes from event timestamps: the chat span opened with
        // thinking and closed with the generation record.
        assert_eq!(chat.start_time, SystemTime::from(h.at(100)));
        assert_eq!(chat.end_time, SystemTime::from(h.at(600)));
        assert_eq!(turn.start_time, SystemTime::from(h.at(0)));
        assert_eq!(turn.end_time, SystemTime::from(h.at(800)));
        assert_eq!(tool.start_time, SystemTime::from(h.at(630)));

        // invoke_agent attributes.
        assert_eq!(
            attr_str(turn, "gen_ai.operation.name").as_deref(),
            Some("invoke_agent")
        );
        assert_eq!(
            attr_str(turn, "gen_ai.agent.name").as_deref(),
            Some("Weather Helper")
        );
        assert_eq!(
            attr_str(turn, "gen_ai.agent.description").as_deref(),
            Some("Answers weather questions")
        );
        assert!(attr(turn, "gen_ai.agent.id").is_some());
        assert_eq!(
            attr_str(turn, "gen_ai.conversation.id"),
            Some(h.session.to_string())
        );
        assert_eq!(
            attr(turn, "gen_ai.usage.input_tokens"),
            Some(&Value::I64(120))
        );
        assert_eq!(
            attr(turn, "gen_ai.usage.output_tokens"),
            Some(&Value::I64(30))
        );
        assert_eq!(attr(turn, "everruns.turn.iterations"), Some(&Value::I64(1)));
        assert_eq!(
            attr_str(turn, "openinference.span.kind").as_deref(),
            Some("AGENT")
        );
        assert_eq!(
            attr_str(turn, "agent.name").as_deref(),
            Some("Weather Helper")
        );
        assert_eq!(attr_str(turn, "session.id"), Some(h.session.to_string()));
        assert_eq!(attr(turn, "llm.token_count.total"), Some(&Value::I64(150)));

        // chat attributes.
        assert_eq!(
            attr_str(chat, "gen_ai.operation.name").as_deref(),
            Some("chat")
        );
        assert_eq!(
            attr_str(chat, "gen_ai.provider.name").as_deref(),
            Some("anthropic")
        );
        assert_eq!(
            attr_str(chat, "gen_ai.request.model").as_deref(),
            Some("claude-sonnet-4-5")
        );
        assert_eq!(
            attr_str(chat, "gen_ai.response.id").as_deref(),
            Some("msg_01")
        );
        assert_eq!(
            attr(chat, "gen_ai.response.finish_reasons"),
            Some(&Value::Array(vec![StringValue::from("tool_calls")].into()))
        );
        assert_eq!(
            attr(chat, "gen_ai.usage.input_tokens"),
            Some(&Value::I64(120))
        );
        assert_eq!(
            attr(chat, "gen_ai.usage.cache_read.input_tokens"),
            Some(&Value::I64(40))
        );
        assert_eq!(
            attr(chat, "gen_ai.usage.cache_write.input_tokens"),
            Some(&Value::I64(8))
        );
        assert_eq!(
            attr(chat, "gen_ai.request.temperature"),
            Some(&Value::F64(0.2f32 as f64))
        );
        assert_eq!(
            attr(chat, "gen_ai.request.max_tokens"),
            Some(&Value::I64(1024))
        );
        assert_eq!(
            attr_str(chat, "gen_ai.request.reasoning.level").as_deref(),
            Some("high")
        );
        assert_eq!(
            attr(chat, "gen_ai.request.stream"),
            Some(&Value::Bool(true))
        );
        assert_eq!(
            attr(chat, "gen_ai.response.time_to_first_chunk"),
            Some(&Value::F64(0.25))
        );
        assert!(attr(chat, "gen_ai.output.type").is_none());
        assert!(attr(chat, "gen_ai.usage.cache_read_tokens").is_none());
        assert_eq!(
            attr_str(chat, "openinference.span.kind").as_deref(),
            Some("LLM")
        );
        assert_eq!(
            attr_str(chat, "llm.model_name").as_deref(),
            Some("claude-sonnet-4-5")
        );
        assert_eq!(attr_str(chat, "llm.provider").as_deref(), Some("anthropic"));
        assert_eq!(attr(chat, "llm.token_count.prompt"), Some(&Value::I64(120)));
        assert_eq!(
            attr(chat, "llm.token_count.prompt_details.cache_read"),
            Some(&Value::I64(40))
        );
        assert_eq!(attr(chat, "llm.cost.total"), Some(&Value::F64(0.0125)));
        assert!(
            attr_str(chat, "llm.tools.0.tool.json_schema")
                .unwrap()
                .contains("get_weather")
        );
        assert!(
            attr_str(chat, "llm.invocation_parameters")
                .unwrap()
                .contains("\"temperature\":0.2")
        );
        assert_eq!(chat.status, Status::Unset);

        // Phase spans are plain internal spans, not Gen-AI operations.
        assert!(attr(reason, "gen_ai.operation.name").is_none());
        assert_eq!(
            attr_str(reason, "everruns.phase").as_deref(),
            Some("reason")
        );
        assert_eq!(
            attr_str(reason, "openinference.span.kind").as_deref(),
            Some("CHAIN")
        );
        assert_eq!(
            attr_str(thinking, "everruns.phase").as_deref(),
            Some("thinking")
        );
        assert_eq!(attr_str(act, "everruns.phase").as_deref(), Some("act"));

        // execute_tool attributes, with the description learned from the
        // generation record.
        assert_eq!(
            attr_str(tool, "gen_ai.operation.name").as_deref(),
            Some("execute_tool")
        );
        assert_eq!(
            attr_str(tool, "gen_ai.tool.name").as_deref(),
            Some("get_weather")
        );
        assert_eq!(
            attr_str(tool, "gen_ai.tool.type").as_deref(),
            Some("function")
        );
        assert_eq!(
            attr_str(tool, "gen_ai.tool.call.id").as_deref(),
            Some("call_1")
        );
        assert_eq!(
            attr_str(tool, "gen_ai.tool.description").as_deref(),
            Some("Look up current weather")
        );
        assert_eq!(
            attr_str(tool, "gen_ai.agent.name").as_deref(),
            Some("Weather Helper")
        );
        assert_eq!(
            attr_str(tool, "openinference.span.kind").as_deref(),
            Some("TOOL")
        );
        assert_eq!(attr_str(tool, "tool.name").as_deref(), Some("get_weather"));
        assert_eq!(
            attr_str(tool, "everruns.tool.status").as_deref(),
            Some("success")
        );
        assert_eq!(tool.status, Status::Unset);
    }

    #[tokio::test]
    async fn content_stays_out_of_spans_by_default() {
        let h = Harness::new(false, TraceConventions::ALL);
        run_full_turn(&h).await;
        for span in h.spans() {
            for key in [
                "gen_ai.input.messages",
                "gen_ai.output.messages",
                "gen_ai.system_instructions",
                "gen_ai.tool.definitions",
                "gen_ai.tool.call.arguments",
                "gen_ai.tool.call.result",
                "input.value",
                "output.value",
                "llm.input_messages.0.message.content",
            ] {
                assert!(attr(&span, key).is_none(), "{} leaked {key}", span.name);
            }
        }
    }

    #[tokio::test]
    async fn content_capture_records_spec_shaped_messages() {
        let h = Harness::new(true, TraceConventions::ALL);
        run_full_turn(&h).await;
        let spans = h.spans();
        let turn = by_name(&spans, "invoke_agent Weather Helper");
        let chat = by_name(&spans, "chat claude-sonnet-4-5");
        let thinking = by_name(&spans, "thinking");
        let tool = by_name(&spans, "execute_tool get_weather");

        let instructions: serde_json::Value =
            serde_json::from_str(&attr_str(chat, "gen_ai.system_instructions").unwrap()).unwrap();
        assert_eq!(
            instructions,
            json!([{ "type": "text", "content": "You are a weather assistant." }])
        );
        let input: serde_json::Value =
            serde_json::from_str(&attr_str(chat, "gen_ai.input.messages").unwrap()).unwrap();
        assert_eq!(input[0]["role"], "user");
        assert_eq!(
            input.as_array().unwrap().len(),
            1,
            "system prompt is not chat history"
        );
        let output: serde_json::Value =
            serde_json::from_str(&attr_str(chat, "gen_ai.output.messages").unwrap()).unwrap();
        assert_eq!(output[0]["role"], "assistant");
        assert_eq!(output[0]["finish_reason"], "tool_calls");
        assert_eq!(output[0]["parts"][0]["type"], "reasoning");
        assert_eq!(output[0]["parts"][0]["content"], "Paris needs a lookup.");
        assert_eq!(output[0]["parts"][1]["type"], "text");
        assert_eq!(output[0]["parts"][2]["type"], "tool_call");
        assert_eq!(output[0]["parts"][2]["name"], "get_weather");
        let tools: serde_json::Value =
            serde_json::from_str(&attr_str(chat, "gen_ai.tool.definitions").unwrap()).unwrap();
        assert_eq!(tools[0]["name"], "get_weather");

        assert_eq!(
            attr_str(chat, "input.mime_type").as_deref(),
            Some("application/json")
        );
        assert_eq!(
            attr_str(chat, "llm.input_messages.0.message.role").as_deref(),
            Some("system")
        );
        assert_eq!(
            attr_str(chat, "llm.input_messages.1.message.role").as_deref(),
            Some("user")
        );
        assert_eq!(
            attr_str(
                chat,
                "llm.output_messages.0.message.tool_calls.0.tool_call.function.name"
            )
            .as_deref(),
            Some("get_weather")
        );

        assert_eq!(
            attr_str(turn, "input.value").as_deref(),
            Some("What is the weather in Paris?")
        );
        assert_eq!(
            attr_str(turn, "output.value").as_deref(),
            Some("It is rainy in Paris.")
        );
        assert!(
            attr_str(turn, "gen_ai.input.messages")
                .unwrap()
                .contains("\"role\":\"user\"")
        );
        assert_eq!(
            attr_str(thinking, "output.value").as_deref(),
            Some("Paris needs a lookup.")
        );

        assert_eq!(
            attr_str(tool, "gen_ai.tool.call.arguments").as_deref(),
            Some(r#"{"city":"Paris"}"#)
        );
        assert_eq!(
            attr_str(tool, "gen_ai.tool.call.result").as_deref(),
            Some("rainy, 14C")
        );
        assert_eq!(
            attr_str(tool, "output.mime_type").as_deref(),
            Some("text/plain")
        );
    }

    #[tokio::test]
    async fn chat_without_thinking_is_backdated_by_its_duration() {
        let h = Harness::new(false, TraceConventions::ALL);
        let exec = ExecId::new();
        h.emit(0, h.context(None, None, None), h.turn_started())
            .await;
        h.emit(
            10,
            h.context(Some(exec), Some("r1"), Some(&h.turn.to_string())),
            ReasonStartedData {
                harness_id: HarnessId::new(),
                agent_id: None,
                metadata: None,
            },
        )
        .await;
        h.emit(
            600,
            h.context(Some(exec), Some("g1"), Some("r1")),
            generation(true),
        )
        .await;
        let spans = h.spans();
        let chat = by_name(&spans, "chat claude-sonnet-4-5");
        assert_eq!(chat.start_time, SystemTime::from(h.at(100)));
        assert_eq!(chat.end_time, SystemTime::from(h.at(600)));
        assert_eq!(chat.span_kind, SpanKind::Client);
    }

    #[tokio::test]
    async fn failures_carry_error_type_status_and_exception_event() {
        let h = Harness::new(false, TraceConventions::ALL);
        let exec = ExecId::new();
        h.emit(0, h.context(None, None, None), h.turn_started())
            .await;
        h.emit(
            10,
            h.context(Some(exec), Some("r1"), Some(&h.turn.to_string())),
            ReasonStartedData {
                harness_id: HarnessId::new(),
                agent_id: None,
                metadata: None,
            },
        )
        .await;
        h.emit(
            600,
            h.context(Some(exec), Some("g1"), Some("r1")),
            generation(false),
        )
        .await;
        h.emit(
            620,
            h.context(Some(exec), Some("t1"), Some("r1")),
            tool_started(),
        )
        .await;
        h.emit(
            700,
            h.context(Some(exec), Some("t1"), Some("r1")),
            ToolCompletedData::failure(
                "call_1".to_string(),
                "get_weather".to_string(),
                "timeout".to_string(),
                "tool timed out after 60s".to_string(),
                Some(80),
            ),
        )
        .await;
        h.emit(
            800,
            h.context(None, None, None),
            TurnFailedData {
                turn_id: h.turn,
                error: "budget exhausted".to_string(),
                error_code: Some("budget_exhausted".to_string()),
                error_fields: None,
                error_disclosure: None,
            },
        )
        .await;

        let spans = h.spans();
        let chat = by_name(&spans, "chat claude-sonnet-4-5");
        assert_eq!(attr_str(chat, "error.type").as_deref(), Some("503"));
        assert!(
            matches!(&chat.status, Status::Error { description } if description == "provider returned 503")
        );
        let exception = chat.events.iter().find(|e| e.name == "exception").unwrap();
        assert!(
            exception
                .attributes
                .iter()
                .any(|kv| kv.key.as_str() == "exception.message")
        );
        assert!(attr(chat, "gen_ai.output.messages").is_none());

        let tool = by_name(&spans, "execute_tool get_weather");
        assert_eq!(attr_str(tool, "error.type").as_deref(), Some("timeout"));
        assert_eq!(
            attr_str(tool, "everruns.tool.status").as_deref(),
            Some("timeout")
        );
        assert!(matches!(&tool.status, Status::Error { .. }));

        let turn = by_name(&spans, "invoke_agent Weather Helper");
        assert_eq!(
            attr_str(turn, "error.type").as_deref(),
            Some("budget_exhausted")
        );
        assert!(
            matches!(&turn.status, Status::Error { description } if description == "budget exhausted")
        );

        // The reason span never completed: the turn end closed it.
        let reason = by_name(&spans, "reason");
        assert_eq!(
            attr(reason, "everruns.span.unterminated"),
            Some(&Value::Bool(true))
        );
        assert_eq!(reason.end_time, SystemTime::from(h.at(800)));
        assert_eq!(h.listener.active_span_count(), 0);
    }

    #[tokio::test]
    async fn cancelled_turn_closes_everything_under_it() {
        let h = Harness::new(false, TraceConventions::ALL);
        let exec = ExecId::new();
        h.emit(0, h.context(None, None, None), h.turn_started())
            .await;
        h.emit(
            10,
            h.context(Some(exec), Some("r1"), Some(&h.turn.to_string())),
            ReasonStartedData {
                harness_id: HarnessId::new(),
                agent_id: None,
                metadata: None,
            },
        )
        .await;
        h.emit(
            100,
            h.context(Some(exec), None, None),
            ReasonThinkingStartedData {
                turn_id: h.turn,
                model: Some("claude-sonnet-4-5".to_string()),
            },
        )
        .await;
        h.emit(
            200,
            h.context(None, None, None),
            TurnCancelledData {
                turn_id: h.turn,
                reason: Some("user stop".to_string()),
                usage: None,
            },
        )
        .await;
        let spans = h.spans();
        assert_eq!(
            spans.len(),
            4,
            "{:?}",
            spans.iter().map(|s| s.name.clone()).collect::<Vec<_>>()
        );
        for name in ["reason", "thinking", "chat claude-sonnet-4-5"] {
            let span = by_name(&spans, name);
            assert_eq!(
                attr(span, "everruns.span.unterminated"),
                Some(&Value::Bool(true))
            );
            assert_eq!(span.end_time, SystemTime::from(h.at(200)));
        }
        let turn = by_name(&spans, "invoke_agent Weather Helper");
        assert_eq!(attr_str(turn, "error.type").as_deref(), Some("cancelled"));
        assert_eq!(
            attr_str(turn, "everruns.turn.status").as_deref(),
            Some("cancelled")
        );
        assert_eq!(h.listener.active_span_count(), 0);
    }

    #[tokio::test]
    async fn pending_chat_is_closed_when_no_generation_record_arrives() {
        let h = Harness::new(false, TraceConventions::ALL);
        let exec = ExecId::new();
        h.emit(0, h.context(None, None, None), h.turn_started())
            .await;
        h.emit(
            10,
            h.context(Some(exec), Some("r1"), Some(&h.turn.to_string())),
            ReasonStartedData {
                harness_id: HarnessId::new(),
                agent_id: None,
                metadata: None,
            },
        )
        .await;
        h.emit(
            100,
            h.context(Some(exec), None, None),
            ReasonThinkingStartedData {
                turn_id: h.turn,
                model: Some("claude-sonnet-4-5".to_string()),
            },
        )
        .await;
        h.emit(
            300,
            h.context(Some(exec), None, None),
            ReasonThinkingCompletedData {
                turn_id: h.turn,
                thinking: "partial".to_string(),
            },
        )
        .await;
        h.emit(
            500,
            h.context(Some(exec), Some("r1"), Some(&h.turn.to_string())),
            ReasonCompletedData::failure("stream dropped".to_string(), Some(490)),
        )
        .await;
        let spans = h.spans();
        let chat = by_name(&spans, "chat claude-sonnet-4-5");
        assert_eq!(chat.end_time, SystemTime::from(h.at(500)));
        assert!(
            matches!(&chat.status, Status::Error { description } if description == "stream dropped")
        );
        let reason = by_name(&spans, "reason");
        assert!(matches!(&reason.status, Status::Error { .. }));
        let thinking = by_name(&spans, "thinking");
        assert_eq!(thinking.parent_span_id, id(chat));
    }

    #[tokio::test]
    async fn orphan_completions_are_reconstructed_from_their_duration() {
        let h = Harness::new(false, TraceConventions::ALL);
        h.emit(0, h.context(None, None, None), h.turn_started())
            .await;
        h.emit(
            700,
            h.context(None, Some("t1"), None),
            ToolCompletedData::success(
                "call_x".to_string(),
                "read_file".to_string(),
                vec![],
                Some(70),
            ),
        )
        .await;
        h.emit(
            800,
            h.context(None, None, None),
            TurnCompletedData {
                turn_id: h.turn,
                iterations: 1,
                duration_ms: Some(800),
                usage: None,
                input_content: None,
                final_message_id: None,
                final_answer_preview: None,
                time_to_first_token_ms: None,
                tool_call_count: None,
                llm_call_count: None,
                status: None,
            },
        )
        .await;
        let spans = h.spans();
        let tool = by_name(&spans, "execute_tool read_file");
        assert_eq!(
            attr(tool, "everruns.span.orphaned"),
            Some(&Value::Bool(true))
        );
        assert_eq!(tool.start_time, SystemTime::from(h.at(630)));
        assert_eq!(tool.end_time, SystemTime::from(h.at(700)));
        let turn = by_name(&spans, "invoke_agent Weather Helper");
        assert_eq!(tool.parent_span_id, id(turn));

        // A turn completing with no start still leaves a record.
        let other = Harness::new(false, TraceConventions::ALL);
        other
            .emit(
                50,
                other.context(None, None, None),
                TurnCompletedData {
                    turn_id: other.turn,
                    iterations: 2,
                    duration_ms: Some(50),
                    usage: None,
                    input_content: None,
                    final_message_id: None,
                    final_answer_preview: None,
                    time_to_first_token_ms: None,
                    tool_call_count: None,
                    llm_call_count: None,
                    status: None,
                },
            )
            .await;
        let spans = other.spans();
        let turn = by_name(&spans, "invoke_agent");
        assert_eq!(
            attr(turn, "everruns.span.orphaned"),
            Some(&Value::Bool(true))
        );
        assert_eq!(attr(turn, "everruns.turn.iterations"), Some(&Value::I64(2)));
    }

    #[tokio::test]
    async fn conventions_can_be_narrowed() {
        let h = Harness::new(false, TraceConventions::GEN_AI);
        run_full_turn(&h).await;
        for span in h.spans() {
            assert!(
                attr(&span, "openinference.span.kind").is_none(),
                "{}",
                span.name
            );
            assert!(attr(&span, "session.id").is_none());
            assert!(attr(&span, "llm.model_name").is_none());
        }
        let h = Harness::new(false, TraceConventions::OPENINFERENCE);
        run_full_turn(&h).await;
        let spans = h.spans();
        for span in &spans {
            assert!(
                !span
                    .attributes
                    .iter()
                    .any(|kv| kv.key.as_str().starts_with("gen_ai.")),
                "{}",
                span.name
            );
        }
        // Span names, kinds, and hierarchy do not depend on the vocabulary.
        let chat = by_name(&spans, "chat claude-sonnet-4-5");
        assert_eq!(
            attr_str(chat, "openinference.span.kind").as_deref(),
            Some("LLM")
        );
        assert_eq!(chat.parent_span_id, id(by_name(&spans, "reason")));
    }

    #[test]
    fn conventions_parse_from_env_syntax() {
        assert_eq!(TraceConventions::parse("gen_ai"), TraceConventions::GEN_AI);
        assert_eq!(
            TraceConventions::parse("openinference"),
            TraceConventions::OPENINFERENCE
        );
        assert_eq!(
            TraceConventions::parse("gen_ai, openinference"),
            TraceConventions::ALL
        );
        assert_eq!(TraceConventions::parse("all"), TraceConventions::ALL);
        assert_eq!(TraceConventions::parse(""), TraceConventions::ALL);
        assert_eq!(TraceConventions::parse("bogus"), TraceConventions::ALL);
        assert_eq!(TraceConventions::parse("OTEL"), TraceConventions::GEN_AI);
    }

    #[tokio::test]
    async fn listener_subscribes_to_the_thirteen_lifecycle_events() {
        let h = Harness::new(false, TraceConventions::ALL);
        let types = h.listener.event_types().unwrap();
        assert_eq!(types.len(), 13);
        assert!(types.contains(&TURN_STARTED));
        assert!(types.contains(&LLM_GENERATION));
        assert!(types.contains(&TOOL_COMPLETED));
        assert_eq!(h.listener.name(), "OtelEventListener");
        assert!(!h.listener.record_content());
        assert_eq!(h.listener.conventions(), TraceConventions::ALL);
    }
}

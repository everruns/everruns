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
            // Fall back to the requested model rather than dropping the
            // attribute: every existing consumer expects it to be present, and
            // for a provider that echoes no model the request is the best
            // answer available.
            attrs.push(KeyValue::new(
                gen_ai::RESPONSE_MODEL,
                meta.response_model
                    .clone()
                    .unwrap_or_else(|| meta.model.clone()),
            ));
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
#[path = "otel_tests.rs"]
mod tests;

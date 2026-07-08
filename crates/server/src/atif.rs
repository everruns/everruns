// ATIF (Agent Trajectory Interchange Format) adoption. See specs/atif-adoption.md.
//
// This module is the single place that folds a session's event log into an
// ATIF-v1.7 trajectory JSON document (Harbor RFC 0001). It is pure over
// `&[Event]` so every export path (session export today; eval dataset export
// as a follow-up) and the tests share one folding implementation.
//
// Folding rules (see the mapping table in specs/atif-adoption.md):
// - `input.message`             → one "user" step
// - `output.message.completed`  → opens one "agent" step per reasoning
//   iteration (everruns emits it once per LLM call); text content becomes the
//   step `message`, tool-call content parts become `tool_calls`, message
//   thinking becomes `reasoning_content`, usage becomes `metrics`
// - `reason.completed`          → annotates the open agent step (duration,
//   fallback usage); a failed reason with no message opens an error step
// - `tool.completed`            → appended to the open agent step's
//   `observation.results` with `source_call_id` = tool_call_id
// - `turn.*`                    → boundaries, not steps; durations/iteration
//   counts are recorded in root `extra.turns`
//
// Secret scrubbing (the same always-on scrubber used by the dataset export)
// is applied to every produced trajectory before it leaves this module.

use chrono::{DateTime, Utc};
use everruns_core::events::{
    Event, EventData, TokenUsage, TurnCancelledData, TurnCompletedData, TurnFailedData,
    TurnSealedData,
};
use everruns_core::message::{ContentPart, Message};
use serde_json::{Map, Value, json};

use crate::domains::evals::dataset::{REDACTED, sanitize_value};

/// Pinned ATIF schema version produced by exports.
pub const ATIF_SCHEMA_VERSION: &str = "ATIF-v1.7";

// ============================================================================
// Trajectory building (export)
// ============================================================================

/// Options for building a trajectory.
#[derive(Debug, Clone, Copy, Default)]
pub struct AtifOptions {
    /// Replace content-bearing strings (messages, reasoning, tool arguments,
    /// observation contents) with a placeholder, preserving structure. Secret
    /// scrubbing is applied regardless.
    pub redact_content: bool,
}

/// Fold a session's events into one ATIF trajectory document.
///
/// `root_extra` lets callers attach provenance/reward at the ATIF extension
/// point (`extra`); ATIF itself has no reward field, so callers that carry
/// reward (the eval dataset export) put it there.
pub fn build_trajectory(
    session_id: Option<&str>,
    events: &[Event],
    mut root_extra: Map<String, Value>,
    options: AtifOptions,
) -> Value {
    let mut fold = Fold::new(options.redact_content);
    for event in events {
        fold.push(event);
    }
    let (steps, turns, totals, model_name) = fold.finish();

    if !turns.is_empty() {
        root_extra.insert("turns".to_string(), Value::Array(turns));
    }

    let mut agent = Map::new();
    agent.insert("name".to_string(), json!("everruns"));
    agent.insert("version".to_string(), json!(env!("CARGO_PKG_VERSION")));
    if let Some(model) = model_name {
        agent.insert("model_name".to_string(), json!(model));
    }

    let mut root = Map::new();
    root.insert("schema_version".to_string(), json!(ATIF_SCHEMA_VERSION));
    if let Some(sid) = session_id {
        root.insert("session_id".to_string(), json!(sid));
    }
    root.insert("agent".to_string(), Value::Object(agent));
    root.insert(
        "final_metrics".to_string(),
        totals.to_final_metrics(steps.len()),
    );
    root.insert("steps".to_string(), Value::Array(steps));
    if !root_extra.is_empty() {
        root.insert("extra".to_string(), Value::Object(root_extra));
    }

    let mut value = Value::Object(root);
    // Always-on secret scrubbing over every exported string. Content redaction
    // was already applied structurally during the fold, so scrub-only here.
    sanitize_value(&mut value, false);
    value
}

/// Aggregated token totals across agent steps.
#[derive(Debug, Default)]
struct Totals {
    prompt: u64,
    completion: u64,
    cached: u64,
    cost_usd: Option<f64>,
    any: bool,
}

impl Totals {
    fn add(&mut self, usage: &TokenUsage) {
        self.any = true;
        self.prompt += prompt_tokens(usage);
        self.completion += usage.output_tokens as u64;
        self.cached += usage.cache_read_tokens.unwrap_or(0) as u64;
        if let Some(cost) = usage.effective_cost_usd() {
            *self.cost_usd.get_or_insert(0.0) += cost;
        }
    }

    fn to_final_metrics(&self, total_steps: usize) -> Value {
        let mut m = Map::new();
        if self.any {
            m.insert("total_prompt_tokens".to_string(), json!(self.prompt));
            m.insert(
                "total_completion_tokens".to_string(),
                json!(self.completion),
            );
            m.insert("total_cached_tokens".to_string(), json!(self.cached));
            if let Some(cost) = self.cost_usd {
                m.insert("total_cost_usd".to_string(), json!(cost));
            }
        }
        m.insert("total_steps".to_string(), json!(total_steps));
        Value::Object(m)
    }
}

/// Total prompt tokens for ATIF `prompt_tokens`: everruns keeps the prompt
/// buckets disjoint (see `TokenUsage`), while ATIF expects the inclusive
/// prompt count with `cached_tokens` as the cached subset.
fn prompt_tokens(usage: &TokenUsage) -> u64 {
    usage.input_tokens as u64
        + usage.cache_read_tokens.unwrap_or(0) as u64
        + usage.cache_creation_tokens.unwrap_or(0) as u64
}

fn usage_to_metrics(usage: &TokenUsage) -> Value {
    let mut m = Map::new();
    m.insert("prompt_tokens".to_string(), json!(prompt_tokens(usage)));
    m.insert(
        "completion_tokens".to_string(),
        json!(usage.output_tokens as u64),
    );
    if let Some(cached) = usage.cache_read_tokens {
        m.insert("cached_tokens".to_string(), json!(cached as u64));
    }
    if let Some(cost) = usage.effective_cost_usd() {
        m.insert("cost_usd".to_string(), json!(cost));
    }
    Value::Object(m)
}

/// An in-progress agent step (one reasoning iteration).
struct AgentStepAcc {
    timestamp: DateTime<Utc>,
    model_name: Option<String>,
    message: String,
    reasoning: Option<String>,
    tool_calls: Vec<Value>,
    observations: Vec<Value>,
    usage: Option<TokenUsage>,
    extra: Map<String, Value>,
}

impl AgentStepAcc {
    fn new(timestamp: DateTime<Utc>) -> Self {
        Self {
            timestamp,
            model_name: None,
            message: String::new(),
            reasoning: None,
            tool_calls: Vec::new(),
            observations: Vec::new(),
            usage: None,
            extra: Map::new(),
        }
    }

    fn has_tool_call(&self, id: &str) -> bool {
        self.tool_calls
            .iter()
            .any(|tc| tc.get("tool_call_id").and_then(Value::as_str) == Some(id))
    }
}

struct Fold {
    redact: bool,
    steps: Vec<Value>,
    turns: Vec<Value>,
    totals: Totals,
    open: Option<AgentStepAcc>,
    pending_thinking: Option<String>,
    last_model: Option<String>,
}

impl Fold {
    fn new(redact: bool) -> Self {
        Self {
            redact,
            steps: Vec::new(),
            turns: Vec::new(),
            totals: Totals::default(),
            open: None,
            pending_thinking: None,
            last_model: None,
        }
    }

    fn content_or_redacted(&self, content: String) -> String {
        if self.redact {
            REDACTED.to_string()
        } else {
            content
        }
    }

    fn push(&mut self, event: &Event) {
        match &event.data {
            EventData::InputMessage(data) => {
                self.flush();
                let text = self.content_or_redacted(flatten_message_text(&data.message));
                self.steps.push(json!({
                    "timestamp": timestamp(event),
                    "source": "user",
                    "message": text,
                }));
            }
            EventData::OutputMessageCompleted(data) => {
                self.flush();
                let mut acc = AgentStepAcc::new(event.ts);
                acc.message = self.content_or_redacted(flatten_message_text(&data.message));
                acc.reasoning = data
                    .message
                    .thinking
                    .clone()
                    .or_else(|| self.pending_thinking.take())
                    .map(|t| self.content_or_redacted(t));
                for part in &data.message.content {
                    if let ContentPart::ToolCall(call) = part {
                        acc.tool_calls.push(json!({
                            "tool_call_id": call.id,
                            "function_name": call.name,
                            "arguments": self.arguments_or_redacted(&call.arguments),
                        }));
                    }
                }
                if let Some(meta) = &data.metadata {
                    acc.model_name = Some(meta.model.clone());
                    self.last_model = Some(meta.model.clone());
                }
                acc.usage = data.usage.clone();
                if let Some(code) = &data.error_code {
                    acc.extra.insert("error_code".to_string(), json!(code));
                }
                self.pending_thinking = None;
                self.open = Some(acc);
            }
            EventData::ReasonThinkingCompleted(data) => {
                self.pending_thinking = Some(data.thinking.clone());
            }
            EventData::ReasonCompleted(data) => {
                if data.success {
                    if let Some(acc) = self.open.as_mut() {
                        if acc.usage.is_none() {
                            acc.usage = data.usage.clone();
                        }
                        if let Some(d) = data.duration_ms {
                            acc.extra.insert("reason_duration_ms".to_string(), json!(d));
                        }
                    }
                } else {
                    // A failed LLM call with no assistant message still counts
                    // as an agent step so the failure is visible in sequence.
                    self.flush();
                    let mut acc = AgentStepAcc::new(event.ts);
                    if let Some(err) = &data.error {
                        acc.extra.insert(
                            "error".to_string(),
                            json!(self.content_or_redacted(err.clone())),
                        );
                    }
                    if let Some(d) = data.duration_ms {
                        acc.extra.insert("reason_duration_ms".to_string(), json!(d));
                    }
                    self.open = Some(acc);
                    self.flush();
                }
            }
            EventData::ToolStarted(data) => {
                let arguments = self.arguments_or_redacted(&data.tool_call.arguments);
                let acc = self.open.get_or_insert_with(|| AgentStepAcc::new(event.ts));
                // Assistant messages normally carry tool-call parts already;
                // this covers paths that only emit `tool.started`.
                if !acc.has_tool_call(&data.tool_call.id) {
                    acc.tool_calls.push(json!({
                        "tool_call_id": data.tool_call.id,
                        "function_name": data.tool_call.name,
                        "arguments": arguments,
                    }));
                }
            }
            EventData::ToolCallRequested(data) => {
                let calls: Vec<(String, String, Value)> = data
                    .tool_calls
                    .iter()
                    .map(|call| {
                        (
                            call.id.clone(),
                            call.name.clone(),
                            self.arguments_or_redacted(&call.arguments),
                        )
                    })
                    .collect();
                let acc = self.open.get_or_insert_with(|| AgentStepAcc::new(event.ts));
                for (id, name, arguments) in calls {
                    if !acc.has_tool_call(&id) {
                        acc.tool_calls.push(json!({
                            "tool_call_id": id,
                            "function_name": name,
                            "arguments": arguments,
                        }));
                    }
                }
            }
            EventData::ToolCompleted(data) => {
                let content = if let Some(err) = &data.error {
                    format!("[error] {err}")
                } else {
                    data.result
                        .as_ref()
                        .map(|parts| flatten_content_parts(parts))
                        .unwrap_or_default()
                };
                let content = self.content_or_redacted(content);
                let mut extra = Map::new();
                extra.insert("tool_name".to_string(), json!(data.tool_name));
                extra.insert("status".to_string(), json!(data.status));
                if let Some(d) = data.duration_ms {
                    extra.insert("duration_ms".to_string(), json!(d));
                }
                let acc = self.open.get_or_insert_with(|| AgentStepAcc::new(event.ts));
                acc.observations.push(json!({
                    "source_call_id": data.tool_call_id,
                    "content": content,
                    "extra": extra,
                }));
            }
            // Turn events are boundaries, not steps: close the open step and
            // record turn stats at the root extension point.
            EventData::TurnCompleted(TurnCompletedData {
                turn_id,
                iterations,
                duration_ms,
                ..
            }) => {
                self.flush();
                let mut t = Map::new();
                t.insert("turn_id".to_string(), json!(turn_id.to_string()));
                t.insert("status".to_string(), json!("completed"));
                t.insert("iterations".to_string(), json!(iterations));
                if let Some(d) = duration_ms {
                    t.insert("duration_ms".to_string(), json!(d));
                }
                self.turns.push(Value::Object(t));
            }
            EventData::TurnFailed(TurnFailedData { turn_id, error, .. }) => {
                self.flush();
                self.turns.push(json!({
                    "turn_id": turn_id.to_string(),
                    "status": "failed",
                    "error": self.content_or_redacted(error.clone()),
                }));
            }
            EventData::TurnSealed(TurnSealedData {
                turn_id,
                reason,
                iterations,
                ..
            }) => {
                self.flush();
                let mut t = Map::new();
                t.insert("turn_id".to_string(), json!(turn_id.to_string()));
                t.insert("status".to_string(), json!("sealed"));
                t.insert("reason".to_string(), json!(reason));
                if let Some(i) = iterations {
                    t.insert("iterations".to_string(), json!(i));
                }
                self.turns.push(Value::Object(t));
            }
            EventData::TurnCancelled(TurnCancelledData {
                turn_id, reason, ..
            }) => {
                self.flush();
                let mut t = Map::new();
                t.insert("turn_id".to_string(), json!(turn_id.to_string()));
                t.insert("status".to_string(), json!("cancelled"));
                if let Some(r) = reason {
                    t.insert("reason".to_string(), json!(r));
                }
                self.turns.push(Value::Object(t));
            }
            // Everything else (deltas, lifecycle, budget, voice, ...) is
            // observability detail with no ATIF equivalent.
            _ => {}
        }
    }

    fn arguments_or_redacted(&self, arguments: &Value) -> Value {
        if self.redact {
            Value::String(REDACTED.to_string())
        } else {
            arguments.clone()
        }
    }

    fn flush(&mut self) {
        let Some(acc) = self.open.take() else {
            return;
        };
        if let Some(usage) = &acc.usage {
            self.totals.add(usage);
        }
        let mut step = Map::new();
        step.insert("timestamp".to_string(), json!(acc.timestamp.to_rfc3339()));
        step.insert("source".to_string(), json!("agent"));
        if let Some(model) = acc.model_name.or_else(|| self.last_model.clone()) {
            step.insert("model_name".to_string(), json!(model));
        }
        step.insert("message".to_string(), json!(acc.message));
        if let Some(reasoning) = acc.reasoning {
            step.insert("reasoning_content".to_string(), json!(reasoning));
        }
        if !acc.tool_calls.is_empty() {
            step.insert("tool_calls".to_string(), Value::Array(acc.tool_calls));
        }
        if !acc.observations.is_empty() {
            step.insert(
                "observation".to_string(),
                json!({ "results": acc.observations }),
            );
        }
        if let Some(usage) = &acc.usage {
            step.insert("metrics".to_string(), usage_to_metrics(usage));
        }
        if !acc.extra.is_empty() {
            step.insert("extra".to_string(), Value::Object(acc.extra));
        }
        self.steps.push(Value::Object(step));
    }

    /// Finish the fold: flush the trailing step and assign 1-based step ids.
    fn finish(mut self) -> (Vec<Value>, Vec<Value>, Totals, Option<String>) {
        self.flush();
        for (i, step) in self.steps.iter_mut().enumerate() {
            if let Value::Object(map) = step {
                map.insert("step_id".to_string(), json!(i + 1));
            }
        }
        (self.steps, self.turns, self.totals, self.last_model)
    }
}

fn timestamp(event: &Event) -> String {
    event.ts.to_rfc3339()
}

/// Flatten a message's text-bearing content to one string. Tool calls and
/// results are represented elsewhere in the step; images become a marker.
fn flatten_message_text(message: &Message) -> String {
    let mut parts: Vec<String> = Vec::new();
    for part in &message.content {
        match part {
            ContentPart::Text(t) => parts.push(t.text.clone()),
            ContentPart::Image(_) | ContentPart::ImageFile(_) => parts.push("[image]".to_string()),
            ContentPart::ToolCall(_) | ContentPart::ToolResult(_) => {}
        }
    }
    parts.join("\n")
}

/// Flatten tool-result content parts to a single observation string.
fn flatten_content_parts(parts: &[ContentPart]) -> String {
    let mut out: Vec<String> = Vec::new();
    for part in parts {
        match part {
            ContentPart::Text(t) => out.push(t.text.clone()),
            ContentPart::Image(_) | ContentPart::ImageFile(_) => out.push("[image]".to_string()),
            other => out.push(serde_json::to_string(other).unwrap_or_default()),
        }
    }
    out.join("\n")
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_core::events::{
        EventContext, InputMessageData, ModelMetadata, OutputMessageCompletedData,
        ReasonCompletedData, ToolCompletedData, TurnCompletedData, TurnFailedData,
    };
    use everruns_core::tool_types::ToolCall;
    use everruns_core::typed_id::{SessionId, TurnId};

    fn event(session: SessionId, data: impl Into<EventData>) -> Event {
        Event::new(session, EventContext::empty(), data)
    }

    /// Synthetic session: user msg → iteration 1 (tool call + result) →
    /// iteration 2 (final text) → turn.completed.
    fn sample_events(session: SessionId) -> Vec<Event> {
        let turn_id = TurnId::new();
        let tool_call = ToolCall {
            id: "call_1".to_string(),
            name: "search".to_string(),
            arguments: json!({"query": "weather"}),
        };

        let mut first = Message::assistant_with_tools("checking", vec![tool_call]);
        first.thinking = Some("let me look this up".to_string());

        vec![
            event(session, InputMessageData::new(Message::user("hi there"))),
            event(
                session,
                OutputMessageCompletedData::new(first)
                    .with_metadata(ModelMetadata {
                        model: "test-model".to_string(),
                        model_id: None,
                        provider_id: None,
                    })
                    .with_usage(TokenUsage::with_cache(100, 20, Some(30), None)),
            ),
            event(
                session,
                ReasonCompletedData::success(
                    "checking",
                    true,
                    1,
                    Some(250),
                    Some(TokenUsage::new(100, 20)),
                ),
            ),
            event(
                session,
                ToolCompletedData::success(
                    "call_1".to_string(),
                    "search".to_string(),
                    vec![ContentPart::text("sunny, 21C")],
                    Some(40),
                ),
            ),
            event(
                session,
                OutputMessageCompletedData::new(Message::assistant(
                    "It is sunny. token sk-abcdef0123456789ABCDEF",
                ))
                .with_usage(TokenUsage::new(150, 10)),
            ),
            event(
                session,
                ReasonCompletedData::success("It is sunny.", false, 0, Some(120), None),
            ),
            event(
                session,
                TurnCompletedData {
                    turn_id,
                    iterations: 2,
                    duration_ms: Some(1500),
                    usage: None,
                    input_content: None,
                    final_message_id: None,
                    final_answer_preview: None,
                    time_to_first_token_ms: None,
                    tool_call_count: Some(1),
                    llm_call_count: Some(2),
                    status: None,
                },
            ),
        ]
    }

    #[test]
    fn folds_events_into_atif_steps() {
        let session = SessionId::new();
        let value = build_trajectory(
            Some("session_x"),
            &sample_events(session),
            Map::new(),
            AtifOptions::default(),
        );

        assert_eq!(value["schema_version"], json!(ATIF_SCHEMA_VERSION));
        assert_eq!(value["session_id"], json!("session_x"));
        assert_eq!(value["agent"]["name"], json!("everruns"));
        assert_eq!(value["agent"]["model_name"], json!("test-model"));

        let steps = value["steps"].as_array().unwrap();
        assert_eq!(steps.len(), 3);

        // Step ids are 1-based and sequential.
        for (i, step) in steps.iter().enumerate() {
            assert_eq!(step["step_id"], json!(i + 1));
        }

        assert_eq!(steps[0]["source"], json!("user"));
        assert_eq!(steps[0]["message"], json!("hi there"));

        // Iteration 1: tool call, reasoning, observation, metrics.
        let s1 = &steps[1];
        assert_eq!(s1["source"], json!("agent"));
        assert_eq!(s1["message"], json!("checking"));
        assert_eq!(s1["reasoning_content"], json!("let me look this up"));
        assert_eq!(s1["model_name"], json!("test-model"));
        assert_eq!(s1["tool_calls"][0]["tool_call_id"], json!("call_1"));
        assert_eq!(s1["tool_calls"][0]["function_name"], json!("search"));
        assert_eq!(
            s1["tool_calls"][0]["arguments"],
            json!({"query": "weather"})
        );
        assert_eq!(
            s1["observation"]["results"][0]["source_call_id"],
            json!("call_1")
        );
        assert_eq!(
            s1["observation"]["results"][0]["content"],
            json!("sunny, 21C")
        );
        // prompt = input(100) + cache_read(30); cached = 30.
        assert_eq!(s1["metrics"]["prompt_tokens"], json!(130));
        assert_eq!(s1["metrics"]["completion_tokens"], json!(20));
        assert_eq!(s1["metrics"]["cached_tokens"], json!(30));
        assert_eq!(s1["extra"]["reason_duration_ms"], json!(250));

        // Iteration 2: final message, secret scrubbed.
        let s2 = &steps[2];
        assert_eq!(s2["source"], json!("agent"));
        let msg = s2["message"].as_str().unwrap();
        assert!(msg.contains("It is sunny."));
        assert!(!msg.contains("sk-abcdef0123456789ABCDEF"));
        assert!(msg.contains(REDACTED));
        assert!(s2.get("tool_calls").is_none());
        assert!(s2.get("observation").is_none());

        // Turn boundary recorded in extra, not as a step.
        assert_eq!(value["extra"]["turns"][0]["status"], json!("completed"));
        assert_eq!(value["extra"]["turns"][0]["iterations"], json!(2));
        assert_eq!(value["extra"]["turns"][0]["duration_ms"], json!(1500));

        // Aggregates: 130 + 150 prompt, 20 + 10 completion.
        assert_eq!(value["final_metrics"]["total_prompt_tokens"], json!(280));
        assert_eq!(value["final_metrics"]["total_completion_tokens"], json!(30));
        assert_eq!(value["final_metrics"]["total_steps"], json!(3));
    }

    #[test]
    fn failed_reason_and_turn_produce_error_step_and_turn_record() {
        let session = SessionId::new();
        let turn_id = TurnId::new();
        let events = vec![
            event(session, InputMessageData::new(Message::user("hi"))),
            event(
                session,
                ReasonCompletedData::failure("provider exploded".to_string(), Some(50)),
            ),
            event(
                session,
                TurnFailedData {
                    turn_id,
                    error: "provider exploded".to_string(),
                    error_code: None,
                    error_fields: None,
                    error_disclosure: None,
                },
            ),
        ];
        let value = build_trajectory(None, &events, Map::new(), AtifOptions::default());
        let steps = value["steps"].as_array().unwrap();
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[1]["source"], json!("agent"));
        assert_eq!(steps[1]["extra"]["error"], json!("provider exploded"));
        assert_eq!(value["extra"]["turns"][0]["status"], json!("failed"));
        assert_eq!(
            value["extra"]["turns"][0]["error"],
            json!("provider exploded")
        );
    }

    #[test]
    fn redact_content_blanks_content_but_keeps_structure() {
        let session = SessionId::new();
        let value = build_trajectory(
            Some("session_x"),
            &sample_events(session),
            Map::new(),
            AtifOptions {
                redact_content: true,
            },
        );
        let serialized = serde_json::to_string(&value).unwrap();
        assert!(!serialized.contains("hi there"));
        assert!(!serialized.contains("sunny"));
        assert!(!serialized.contains("weather"));
        assert!(!serialized.contains("let me look this up"));
        // Structure preserved.
        let steps = value["steps"].as_array().unwrap();
        assert_eq!(steps[1]["tool_calls"][0]["function_name"], json!("search"));
        assert_eq!(
            steps[1]["observation"]["results"][0]["source_call_id"],
            json!("call_1")
        );
        assert_eq!(steps[0]["message"], json!(REDACTED));
    }
}

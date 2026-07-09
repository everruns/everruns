// ATIF (Agent Trajectory Interchange Format) adoption. See specs/atif-adoption.md.
//
// This module is the single place that folds a session's event log into an
// ATIF-v1.7 trajectory JSON document (Harbor RFC 0001), and the inverse parser
// used by the eval-case importer. It is pure over `&[Event]` so both export
// paths (session export, eval dataset export) and the tests share one folding
// implementation.
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

/// Pinned ATIF schema version produced by exports. Imports tolerate any
/// `ATIF-*` version (unknown fields are ignored by construction).
pub const ATIF_SCHEMA_VERSION: &str = "ATIF-v1.7";

/// Hard cap on a single serialized ATIF session-export document, in bytes.
///
/// The session export (`?format=atif`) is a synchronous single-document
/// response, so an unbounded fold of a very large event log could pin server
/// memory and saturate the response path (TM-DOS-026). Documents over this
/// cap are rejected with HTTP 413 at the export route. Segmented export via
/// ATIF `continued_trajectory_ref` is the planned follow-up for sessions
/// that exceed it.
pub const ATIF_EXPORT_MAX_BYTES: usize = 50 * 1024 * 1024;

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

/// A built trajectory document plus fold facts that callers surface
/// out-of-band (e.g. the `X-Atif-Images-Omitted` header on session export).
#[derive(Debug)]
pub struct BuiltTrajectory {
    pub document: Value,
    /// Image content parts flattened to `"[image]"` markers across the whole
    /// document (step messages and observation content). Also recorded inside
    /// the document as root `extra.images_omitted` when non-zero.
    pub images_omitted: usize,
}

/// Fold a session's events into one ATIF trajectory document.
///
/// `root_extra` lets callers attach provenance/reward at the ATIF extension
/// point (`extra`); ATIF itself has no reward field, so the dataset export
/// puts reward there (see `build_case_record`).
pub fn build_trajectory(
    session_id: Option<&str>,
    events: &[Event],
    mut root_extra: Map<String, Value>,
    options: AtifOptions,
) -> BuiltTrajectory {
    let mut fold = Fold::new(options.redact_content);
    for event in events {
        fold.push(event);
    }
    let (steps, turns, totals, model_name, images_omitted) = fold.finish();

    if !turns.is_empty() {
        root_extra.insert("turns".to_string(), Value::Array(turns));
    }
    if images_omitted > 0 {
        root_extra.insert("images_omitted".to_string(), json!(images_omitted));
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
    BuiltTrajectory {
        document: value,
        images_omitted,
    }
}

/// Build one dataset NDJSON record for an eval case: a complete ATIF
/// trajectory with reward and case identity in root `extra`.
pub fn build_case_record(
    run: &everruns_core::eval::EvalRun,
    result: &everruns_core::eval::EvalCaseResult,
    events: &[Event],
    options: AtifOptions,
) -> Value {
    let mut extra = Map::new();
    extra.insert(
        "reward".to_string(),
        crate::domains::evals::dataset::reward(result),
    );
    extra.insert(
        "source_key".to_string(),
        json!(crate::domains::evals::dataset::source_key(run, result)),
    );
    extra.insert("eval_run_id".to_string(), json!(run.public_id.to_string()));
    extra.insert(
        "case_id".to_string(),
        json!(result.eval_case_id.to_string()),
    );
    extra.insert("case_name".to_string(), json!(result.case_name));
    let session_id = result.session_id.map(|s| s.to_string());
    build_trajectory(session_id.as_deref(), events, extra, options).document
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
    /// Image content parts flattened to `"[image]"` markers so far.
    images_omitted: usize,
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
            images_omitted: 0,
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
                let mut omitted = Vec::new();
                let text = self.content_or_redacted(flatten_message_text(
                    &data.message,
                    self.redact,
                    &mut omitted,
                ));
                self.images_omitted += omitted.len();
                let mut step = Map::new();
                step.insert("timestamp".to_string(), json!(timestamp(event)));
                step.insert("source".to_string(), json!("user"));
                step.insert("message".to_string(), json!(text));
                let mut extra = Map::new();
                append_omitted_images(&mut extra, omitted);
                if !extra.is_empty() {
                    step.insert("extra".to_string(), Value::Object(extra));
                }
                self.steps.push(Value::Object(step));
            }
            EventData::OutputMessageCompleted(data) => {
                self.flush();
                let mut acc = AgentStepAcc::new(event.ts);
                let mut omitted = Vec::new();
                acc.message = self.content_or_redacted(flatten_message_text(
                    &data.message,
                    self.redact,
                    &mut omitted,
                ));
                self.images_omitted += omitted.len();
                append_omitted_images(&mut acc.extra, omitted);
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
                let mut omitted = Vec::new();
                let content = if let Some(err) = &data.error {
                    format!("[error] {err}")
                } else {
                    data.result
                        .as_ref()
                        .map(|parts| flatten_content_parts(parts, self.redact, &mut omitted))
                        .unwrap_or_default()
                };
                let content = self.content_or_redacted(content);
                self.images_omitted += omitted.len();
                let mut extra = Map::new();
                extra.insert("tool_name".to_string(), json!(data.tool_name));
                extra.insert("status".to_string(), json!(data.status));
                if let Some(d) = data.duration_ms {
                    extra.insert("duration_ms".to_string(), json!(d));
                }
                let acc = self.open.get_or_insert_with(|| AgentStepAcc::new(event.ts));
                append_omitted_images(&mut acc.extra, omitted);
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
    fn finish(mut self) -> (Vec<Value>, Vec<Value>, Totals, Option<String>, usize) {
        self.flush();
        for (i, step) in self.steps.iter_mut().enumerate() {
            if let Value::Object(map) = step {
                map.insert("step_id".to_string(), json!(i + 1));
            }
        }
        (
            self.steps,
            self.turns,
            self.totals,
            self.last_model,
            self.images_omitted,
        )
    }
}

fn timestamp(event: &Event) -> String {
    event.ts.to_rfc3339()
}

/// Flatten a message's text-bearing content to one string. Tool calls and
/// results are represented elsewhere in the step; images become a marker and
/// their locator record is appended to `omitted`.
fn flatten_message_text(message: &Message, redact: bool, omitted: &mut Vec<Value>) -> String {
    let mut parts: Vec<String> = Vec::new();
    for part in &message.content {
        match part {
            ContentPart::Text(t) => parts.push(t.text.clone()),
            ContentPart::Image(_) | ContentPart::ImageFile(_) => {
                parts.push("[image]".to_string());
                omitted.extend(omitted_image_ref(part, redact));
            }
            ContentPart::ToolCall(_) | ContentPart::ToolResult(_) => {}
        }
    }
    parts.join("\n")
}

/// Flatten tool-result content parts to a single observation string. Images
/// become a marker and their locator record is appended to `omitted`.
fn flatten_content_parts(parts: &[ContentPart], redact: bool, omitted: &mut Vec<Value>) -> String {
    let mut out: Vec<String> = Vec::new();
    for part in parts {
        match part {
            ContentPart::Text(t) => out.push(t.text.clone()),
            ContentPart::Image(_) | ContentPart::ImageFile(_) => {
                out.push("[image]".to_string());
                omitted.extend(omitted_image_ref(part, redact));
            }
            other => out.push(serde_json::to_string(other).unwrap_or_default()),
        }
    }
    out.join("\n")
}

/// Locator record for an image content part flattened to a `"[image]"`
/// marker: url / file_id / media_type / filename, as present on the part.
/// Never raw bytes — base64 payloads are dropped entirely. `redact` blanks
/// the content-bearing locators (url, filename) while keeping the structural
/// ones (file_id, media_type), mirroring `AtifOptions::redact_content`.
fn omitted_image_ref(part: &ContentPart, redact: bool) -> Option<Value> {
    let redacted = |s: &str| {
        if redact {
            REDACTED.to_string()
        } else {
            s.to_string()
        }
    };
    match part {
        ContentPart::Image(image) => {
            let mut m = Map::new();
            if let Some(url) = &image.url {
                m.insert("url".to_string(), json!(redacted(url)));
            }
            if let Some(media_type) = &image.media_type {
                m.insert("media_type".to_string(), json!(media_type));
            }
            Some(Value::Object(m))
        }
        ContentPart::ImageFile(file) => {
            let mut m = Map::new();
            m.insert("file_id".to_string(), json!(file.image_id.to_string()));
            if let Some(filename) = &file.filename {
                m.insert("filename".to_string(), json!(redacted(filename)));
            }
            Some(Value::Object(m))
        }
        _ => None,
    }
}

/// Append omitted-image locator records to a step's `extra.omitted_images`,
/// creating or extending the array. No-op (and no key) when `omitted` is
/// empty, so image-free steps carry no extra keys.
fn append_omitted_images(extra: &mut Map<String, Value>, omitted: Vec<Value>) {
    if omitted.is_empty() {
        return;
    }
    match extra.get_mut("omitted_images") {
        Some(Value::Array(existing)) => existing.extend(omitted),
        _ => {
            extra.insert("omitted_images".to_string(), Value::Array(omitted));
        }
    }
}

// ============================================================================
// Import (ATIF → eval case drafts)
// ============================================================================

/// Import body cap (NDJSON or JSON). Sized like other untrusted-import caps
/// (see specs/okf-adoption.md security notes).
pub const MAX_IMPORT_BYTES: usize = 4 * 1024 * 1024;
/// Max trajectories accepted per import call.
pub const MAX_IMPORT_TRAJECTORIES: usize = 200;
/// Per-string content cap applied to imported messages.
const MAX_IMPORT_CONTENT_CHARS: usize = 64 * 1024;
/// Reference excerpt length carried into the case description.
const MAX_REFERENCE_CHARS: usize = 2000;

/// A parsed, validated case draft derived from one ATIF trajectory.
#[derive(Debug, Clone)]
pub struct AtifCaseDraft {
    /// Natural key within the target eval (used for idempotent upsert).
    pub name: String,
    pub description: String,
    /// User steps, in order — one input message each.
    pub conversation: Vec<everruns_core::eval::EvalInputMessage>,
}

/// Parse an import body that is either a JSON array of trajectories, a single
/// trajectory object, an object with a `trajectories` array, or NDJSON with
/// one trajectory per line.
pub fn parse_import_body(body: &str) -> Result<Vec<Value>, String> {
    if body.len() > MAX_IMPORT_BYTES {
        return Err(format!(
            "import body exceeds the {MAX_IMPORT_BYTES}-byte limit"
        ));
    }
    let trajectories: Vec<Value> = match serde_json::from_str::<Value>(body) {
        Ok(Value::Array(items)) => items,
        Ok(Value::Object(mut obj)) => match obj.remove("trajectories") {
            Some(Value::Array(items)) => items,
            Some(_) => return Err("`trajectories` must be an array".to_string()),
            None => vec![Value::Object(obj)],
        },
        Ok(_) => return Err("import body must be a JSON object, array, or NDJSON".to_string()),
        // Not a single JSON document — treat as NDJSON, one object per line.
        Err(_) => {
            let mut items = Vec::new();
            for (i, line) in body.lines().enumerate() {
                if line.trim().is_empty() {
                    continue;
                }
                let value: Value = serde_json::from_str(line)
                    .map_err(|e| format!("invalid JSON on line {}: {e}", i + 1))?;
                items.push(value);
            }
            items
        }
    };
    if trajectories.is_empty() {
        return Err("import contains no trajectories".to_string());
    }
    if trajectories.len() > MAX_IMPORT_TRAJECTORIES {
        return Err(format!(
            "import exceeds the {MAX_IMPORT_TRAJECTORIES}-trajectory limit"
        ));
    }
    Ok(trajectories)
}

/// Map one ATIF trajectory to an eval-case draft.
///
/// Tolerates unknown fields and any `ATIF-*` schema version; requires
/// `schema_version`, a non-empty `steps` array, and at least one user step.
pub fn trajectory_to_case_draft(trajectory: &Value, index: usize) -> Result<AtifCaseDraft, String> {
    let at = |msg: &str| format!("trajectory {}: {msg}", index + 1);

    let obj = trajectory
        .as_object()
        .ok_or_else(|| at("must be a JSON object"))?;
    let version = obj
        .get("schema_version")
        .and_then(Value::as_str)
        .ok_or_else(|| at("missing schema_version"))?;
    if !version.starts_with("ATIF-") {
        return Err(at(&format!("unsupported schema_version '{version}'")));
    }
    let steps = obj
        .get("steps")
        .and_then(Value::as_array)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| at("missing or empty steps"))?;

    let mut conversation = Vec::new();
    let mut reference: Option<String> = None;
    for step in steps {
        let source = step.get("source").and_then(Value::as_str).unwrap_or("");
        let text = step_message_text(step);
        match source {
            "user" if !text.trim().is_empty() => {
                conversation.push(everruns_core::eval::EvalInputMessage {
                    content: truncate_chars(&text, MAX_IMPORT_CONTENT_CHARS),
                });
            }
            "agent" if !text.trim().is_empty() => reference = Some(text),
            _ => {}
        }
    }
    if conversation.is_empty() {
        return Err(at("has no user steps to build a conversation from"));
    }

    let extra = obj.get("extra").and_then(Value::as_object);
    let name = extra
        .and_then(|e| e.get("case_name").and_then(Value::as_str))
        .or_else(|| extra.and_then(|e| e.get("source_key").and_then(Value::as_str)))
        .or_else(|| obj.get("trajectory_id").and_then(Value::as_str))
        .or_else(|| obj.get("session_id").and_then(Value::as_str))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| truncate_chars(s, 200))
        .unwrap_or_else(|| format!("atif-case-{}", index + 1));

    let agent_label = obj.get("agent").and_then(Value::as_object).map(|a| {
        format!(
            "{} {}",
            a.get("name").and_then(Value::as_str).unwrap_or("unknown"),
            a.get("version").and_then(Value::as_str).unwrap_or(""),
        )
    });
    let mut description = format!(
        "Imported from ATIF trajectory{}.",
        agent_label
            .map(|l| format!(" produced by {}", l.trim_end()))
            .unwrap_or_default()
    );
    if let Some(reward) = extra.and_then(|e| e.get("reward")) {
        description.push_str(&format!(
            "\nSource reward: {}",
            serde_json::to_string(reward).unwrap_or_default()
        ));
    }
    // ATIF carries no assertion semantics, so the final agent message is kept
    // as a human reference rather than auto-synthesized into a brittle scorer.
    if let Some(reference) = reference {
        description.push_str("\n\nReference final agent message:\n");
        description.push_str(&truncate_chars(&reference, MAX_REFERENCE_CHARS));
    }

    Ok(AtifCaseDraft {
        name,
        description,
        conversation,
    })
}

/// Extract the text of an ATIF step `message` (string or ContentPart array).
fn step_message_text(step: &Value) -> String {
    match step.get("message") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|p| {
                if p.get("type").and_then(Value::as_str) == Some("text") {
                    p.get("text").and_then(Value::as_str).map(str::to_string)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max).collect()
    }
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
        let built = build_trajectory(
            Some("session_x"),
            &sample_events(session),
            Map::new(),
            AtifOptions::default(),
        );
        let value = built.document;

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
        let value = build_trajectory(None, &events, Map::new(), AtifOptions::default()).document;
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
        )
        .document;
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

    #[test]
    fn images_flatten_to_markers_and_record_refs() {
        use everruns_core::message::{ImageContentPart, ImageFileContentPart};
        use everruns_core::typed_id::ImageId;

        let session = SessionId::new();
        let image_id = ImageId::new();
        let mut user = Message::user("look at this");
        user.content
            .push(ContentPart::Image(ImageContentPart::from_url(
                "https://example.com/cat.png",
            )));
        user.content
            .push(ContentPart::ImageFile(ImageFileContentPart::with_filename(
                image_id, "cat.png",
            )));

        let tool_call = ToolCall {
            id: "call_img".to_string(),
            name: "screenshot".to_string(),
            arguments: json!({}),
        };
        let events = vec![
            event(session, InputMessageData::new(user)),
            event(
                session,
                OutputMessageCompletedData::new(Message::assistant_with_tools(
                    "taking a screenshot",
                    vec![tool_call],
                )),
            ),
            event(
                session,
                ToolCompletedData::success(
                    "call_img".to_string(),
                    "screenshot".to_string(),
                    vec![
                        ContentPart::text("done"),
                        ContentPart::Image(ImageContentPart::from_base64("aGVsbG8=", "image/png")),
                    ],
                    Some(5),
                ),
            ),
        ];

        let built = build_trajectory(
            Some("session_x"),
            &events,
            Map::new(),
            AtifOptions::default(),
        );
        assert_eq!(built.images_omitted, 3);
        let value = built.document;
        assert_eq!(value["extra"]["images_omitted"], json!(3));

        // User step: markers stay in the text, locator refs land in step extra.
        let steps = value["steps"].as_array().unwrap();
        assert_eq!(steps[0]["message"], json!("look at this\n[image]\n[image]"));
        let user_refs = steps[0]["extra"]["omitted_images"].as_array().unwrap();
        assert_eq!(user_refs.len(), 2);
        assert_eq!(user_refs[0], json!({"url": "https://example.com/cat.png"}));
        assert_eq!(user_refs[1]["file_id"], json!(image_id.to_string()));
        assert_eq!(user_refs[1]["filename"], json!("cat.png"));

        // Observation content keeps the marker; the base64 payload is dropped
        // and the ref is recorded on the owning agent step.
        let s1 = &steps[1];
        assert_eq!(
            s1["observation"]["results"][0]["content"],
            json!("done\n[image]")
        );
        assert_eq!(
            s1["extra"]["omitted_images"],
            json!([{"media_type": "image/png"}])
        );
        let serialized = serde_json::to_string(&value).unwrap();
        assert!(
            !serialized.contains("aGVsbG8="),
            "raw image bytes must never be exported"
        );

        // Redaction blanks content-bearing locators, keeps structural ones.
        let redacted = build_trajectory(
            Some("session_x"),
            &events,
            Map::new(),
            AtifOptions {
                redact_content: true,
            },
        )
        .document;
        let refs = &redacted["steps"][0]["extra"]["omitted_images"];
        assert_eq!(refs[0]["url"], json!(REDACTED));
        assert_eq!(refs[1]["file_id"], json!(image_id.to_string()));
        assert_eq!(refs[1]["filename"], json!(REDACTED));
    }

    #[test]
    fn zero_image_session_carries_no_omission_keys() {
        let session = SessionId::new();
        let built = build_trajectory(
            Some("session_x"),
            &sample_events(session),
            Map::new(),
            AtifOptions::default(),
        );
        assert_eq!(built.images_omitted, 0);
        let serialized = serde_json::to_string(&built.document).unwrap();
        assert!(!serialized.contains("omitted_images"));
        assert!(!serialized.contains("images_omitted"));
    }

    #[test]
    fn case_record_carries_reward_and_identity_in_extra() {
        use everruns_core::eval::{
            CaseResultStatus, EvalCaseResult, EvalRun, EvalRunSource, EvalRunStatus,
        };
        use everruns_core::typed_id::{EvalCaseId, EvalResultId, EvalRunId};

        let session = SessionId::new();
        let run = EvalRun {
            public_id: EvalRunId::new(),
            internal_id: uuid::Uuid::nil(),
            org_id: 1,
            target: None,
            model_override: None,
            filter_tags: None,
            status: EvalRunStatus::Completed,
            source: EvalRunSource::Internal,
            attribution: None,
            triggered_by: "test".to_string(),
            started_at: None,
            completed_at: None,
            summary: None,
            results: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let result = EvalCaseResult {
            public_id: EvalResultId::new(),
            internal_id: uuid::Uuid::nil(),
            eval_case_id: EvalCaseId::new(),
            case_name: Some("case-one".to_string()),
            session_id: Some(session),
            target: None,
            target_snapshot: None,
            status: CaseResultStatus::Passed,
            scores: Some(json!([{"pass": true, "value": 1.0, "reason": "ok"}])),
            metadata: None,
            turns: Some(1),
            latency_ms: Some(10),
            input_tokens: Some(1),
            output_tokens: Some(2),
            error_message: None,
            artifacts: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let record = build_case_record(
            &run,
            &result,
            &sample_events(session),
            AtifOptions::default(),
        );
        assert_eq!(record["schema_version"], json!(ATIF_SCHEMA_VERSION));
        assert_eq!(record["extra"]["reward"]["pass"], json!(true));
        assert_eq!(record["extra"]["reward"]["score"], json!(1.0));
        assert_eq!(record["extra"]["case_name"], json!("case-one"));
        assert!(
            record["extra"]["source_key"]
                .as_str()
                .unwrap()
                .contains('/')
        );
        assert_eq!(record["session_id"], json!(session.to_string()));
    }

    // ------------------------------------------------------------------
    // Import parsing
    // ------------------------------------------------------------------

    fn sample_trajectory() -> Value {
        json!({
            "schema_version": "ATIF-v1.7",
            "session_id": "session_abc",
            "agent": {"name": "yolop", "version": "0.3.0"},
            "steps": [
                {"step_id": 1, "source": "user", "message": "What is 2+2?"},
                {"step_id": 2, "source": "agent", "message": "4"},
            ],
            "extra": {"reward": {"pass": true, "score": 1.0}},
            "unknown_future_field": {"ignored": true},
        })
    }

    #[test]
    fn parse_accepts_array_object_and_ndjson() {
        let t = sample_trajectory();

        let array_body = serde_json::to_string(&json!([t])).unwrap();
        assert_eq!(parse_import_body(&array_body).unwrap().len(), 1);

        let single_body = serde_json::to_string(&t).unwrap();
        assert_eq!(parse_import_body(&single_body).unwrap().len(), 1);

        let wrapped = serde_json::to_string(&json!({"trajectories": [t, t]})).unwrap();
        assert_eq!(parse_import_body(&wrapped).unwrap().len(), 2);

        let ndjson = format!(
            "{}\n{}\n",
            serde_json::to_string(&t).unwrap(),
            serde_json::to_string(&t).unwrap()
        );
        assert_eq!(parse_import_body(&ndjson).unwrap().len(), 2);
    }

    #[test]
    fn parse_rejects_empty_and_oversized() {
        assert!(parse_import_body("[]").is_err());
        assert!(parse_import_body("not json at all").is_err());
        let big = "x".repeat(MAX_IMPORT_BYTES + 1);
        assert!(parse_import_body(&big).is_err());
    }

    #[test]
    fn draft_maps_user_steps_and_reference() {
        let draft = trajectory_to_case_draft(&sample_trajectory(), 0).unwrap();
        assert_eq!(draft.name, "session_abc");
        assert_eq!(draft.conversation.len(), 1);
        assert_eq!(draft.conversation[0].content, "What is 2+2?");
        assert!(draft.description.contains("Reference final agent message"));
        assert!(draft.description.contains('4'));
        assert!(draft.description.contains("yolop"));
        assert!(draft.description.contains("reward"));
    }

    #[test]
    fn draft_prefers_extra_case_name_and_tolerates_content_parts() {
        let t = json!({
            "schema_version": "ATIF-v1.9",
            "agent": {"name": "x", "version": "1"},
            "steps": [
                {"step_id": 1, "source": "user",
                 "message": [{"type": "text", "text": "part one"}, {"type": "text", "text": "part two"}]},
            ],
            "extra": {"case_name": "named-case"},
        });
        let draft = trajectory_to_case_draft(&t, 3).unwrap();
        assert_eq!(draft.name, "named-case");
        assert_eq!(draft.conversation[0].content, "part one\npart two");
    }

    #[test]
    fn draft_rejects_missing_version_and_missing_user_steps() {
        assert!(trajectory_to_case_draft(&json!({"steps": []}), 0).is_err());
        let no_user = json!({
            "schema_version": "ATIF-v1.7",
            "steps": [{"step_id": 1, "source": "agent", "message": "hello"}],
        });
        assert!(trajectory_to_case_draft(&no_user, 0).is_err());
        let bad_version = json!({
            "schema_version": "OTHER-v1",
            "steps": [{"step_id": 1, "source": "user", "message": "hi"}],
        });
        assert!(trajectory_to_case_draft(&bad_version, 0).is_err());
    }

    #[test]
    fn draft_falls_back_to_indexed_name() {
        let t = json!({
            "schema_version": "ATIF-v1.7",
            "steps": [{"step_id": 1, "source": "user", "message": "hi"}],
        });
        let draft = trajectory_to_case_draft(&t, 4).unwrap();
        assert_eq!(draft.name, "atif-case-5");
    }
}

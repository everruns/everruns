// ATIF (Agent Trajectory Interchange Format) adoption. See knowledge/evaluation/atif-adoption.md.
//
// This module is the single place that folds a session's event log into an
// ATIF-v1.7 trajectory JSON document (Harbor RFC 0001), and the inverse parser
// used by the eval-case importer. It is pure over `&[Event]` so both export
// paths (session export, eval dataset export) and the tests share one folding
// implementation.
//
// Folding rules (see the mapping table in knowledge/evaluation/atif-adoption.md):
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
use everruns_core::message::{ContentPart, RuntimeMessage};
use serde_json::{Map, Value, json};
use std::collections::HashMap;

use crate::domains::evals::dataset::{REDACTED, sanitize_value};
use crate::domains::evals::record::{EvalCaseResult, EvalInputMessage, EvalRun};

/// Pinned ATIF schema version produced by exports. Imports tolerate any
/// `ATIF-*` version (unknown fields are ignored by construction).
pub const ATIF_SCHEMA_VERSION: &str = "ATIF-v1.7";

/// Hard cap on a single serialized ATIF session-export document, in bytes.
///
/// The session export (`?format=atif`) is a synchronous single-document
/// response, so an unbounded fold of a very large event log could pin server
/// memory and saturate the response path (TM-DOS-026). Documents over this
/// cap are rejected with HTTP 413 at the export route; the recoverable path is
/// segmented export (`?format=atif&segmented=true`, see `build_segment`), which
/// bounds every response to this cap and links segments via ATIF
/// `continued_trajectory_ref`.
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
    /// Image content parts that could NOT be materialized into an ATIF image
    /// ContentPart (an inline image carrying neither a URL nor base64 bytes) and
    /// were therefore flattened to `"[image]"` markers across the whole document
    /// (step messages and observation content). Materialized images (URL, file
    /// reference, or inlined base64) are exported as real content parts and do
    /// NOT count here, so this is typically 0. Also recorded inside the document
    /// as root `extra.images_omitted` when non-zero.
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
    let folded = fold_events(events, options.redact_content);
    let FoldOutput {
        steps,
        turns,
        totals,
        model_name,
        images_omitted,
    } = folded;

    if !turns.is_empty() {
        root_extra.insert("turns".to_string(), Value::Array(turns));
    }
    if images_omitted > 0 {
        root_extra.insert("images_omitted".to_string(), json!(images_omitted));
    }

    let final_metrics = totals.to_final_metrics(steps.len());
    let mut value = assemble_document(
        session_id,
        None,
        model_name,
        final_metrics,
        steps,
        root_extra,
    );
    // Always-on secret scrubbing over every exported string. Content redaction
    // was already applied structurally during the fold, so scrub-only here.
    sanitize_value(&mut value, false);
    BuiltTrajectory {
        document: value,
        images_omitted,
    }
}

/// Raw fold outputs shared by the whole-document and segmented export paths.
/// `steps` already carry their absolute 1-based `step_id`.
struct FoldOutput {
    steps: Vec<Value>,
    turns: Vec<Value>,
    totals: Totals,
    model_name: Option<String>,
    images_omitted: usize,
}

/// Fold an event slice into ATIF pieces once. The single event→step mapping
/// (`Fold`) is shared by every export surface; callers assemble the outer
/// document (whole-doc or one segment) from these pieces.
fn fold_events(events: &[Event], redact: bool) -> FoldOutput {
    let mut fold = Fold::new(redact);
    for event in events {
        fold.push(event);
    }
    let (steps, turns, totals, model_name, images_omitted) = fold.finish();
    FoldOutput {
        steps,
        turns,
        totals,
        model_name,
        images_omitted,
    }
}

/// Assemble one ATIF root document from already-folded steps. `trajectory_id`
/// is set only for segments (the whole-doc export has none). Not scrubbed here
/// — callers run `sanitize_value` after assembly.
fn assemble_document(
    session_id: Option<&str>,
    trajectory_id: Option<&str>,
    model_name: Option<String>,
    final_metrics: Value,
    steps: Vec<Value>,
    root_extra: Map<String, Value>,
) -> Value {
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
    if let Some(tid) = trajectory_id {
        root.insert("trajectory_id".to_string(), json!(tid));
    }
    root.insert("agent".to_string(), Value::Object(agent));
    root.insert("final_metrics".to_string(), final_metrics);
    root.insert("steps".to_string(), Value::Array(steps));
    if !root_extra.is_empty() {
        root.insert("extra".to_string(), Value::Object(root_extra));
    }
    Value::Object(root)
}

/// Build one dataset NDJSON record for an eval case: a complete ATIF
/// trajectory with reward and case identity in root `extra`.
///
/// Folds the raw event log. Used by the whole-session export path; the eval
/// **dataset** export uses [`build_case_record_from_messages`] instead so its
/// rows reflect the compaction model view.
pub fn build_case_record(
    run: &EvalRun,
    result: &EvalCaseResult,
    events: &[Event],
    options: AtifOptions,
) -> Value {
    let (extra, session_id) = case_extra(run, result);
    build_trajectory(session_id.as_deref(), events, extra, options).document
}

/// Build one dataset NDJSON record from the case's **model-view** messages
/// (post-compaction masking) rather than the raw event log, so training rows
/// match what the model actually saw. Reward and case identity go in root `extra`, same as
/// [`build_case_record`].
///
/// Model-view messages carry no per-step token usage or turn boundaries, so
/// steps have no `metrics` and there is no `extra.turns`; case-level token
/// totals from the result are surfaced in `final_metrics` instead.
pub fn build_case_record_from_messages(
    run: &EvalRun,
    result: &EvalCaseResult,
    messages: &[RuntimeMessage],
    options: AtifOptions,
) -> Value {
    let (extra, session_id) = case_extra(run, result);
    let steps = fold_messages(messages, options.redact_content);
    let final_metrics = case_final_metrics(result, steps.len());
    let mut value = assemble_document(
        session_id.as_deref(),
        None,
        None,
        final_metrics,
        steps,
        extra,
    );
    // Same always-on secret scrubbing as every other export path. Content
    // redaction was already applied structurally during the fold.
    sanitize_value(&mut value, false);
    value
}

/// Reward + case-identity `extra` shared by both dataset record builders, plus
/// the case session id (as the ATIF root `session_id`).
fn case_extra(run: &EvalRun, result: &EvalCaseResult) -> (Map<String, Value>, Option<String>) {
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
    (extra, result.session_id.map(|s| s.to_string()))
}

/// Fold post-compaction model-view messages into ATIF steps. Mirrors the event
/// fold's step shape (user/agent steps, tool-call parts → `tool_calls[]`,
/// thinking → `reasoning_content`, tool results → `observation.results[]`) but
/// over reconstructed `RuntimeMessage`s, so it reflects exactly what the model saw.
fn fold_messages(messages: &[RuntimeMessage], redact: bool) -> Vec<Value> {
    use everruns_core::message::RuntimeMessageRole;

    let mut steps: Vec<Value> = Vec::new();
    // Index of the agent step that trailing tool results attach observations to.
    let mut open_agent: Option<usize> = None;
    // Model-view tool result messages carry only a call id. Preserve the
    // originating tool name from prior assistant messages so structural
    // subagent links are only emitted for real spawn_agent results.
    let mut tool_names_by_call_id: HashMap<String, String> = HashMap::new();

    for message in messages {
        let ts = json!(message.created_at.to_rfc3339());
        match message.role {
            RuntimeMessageRole::System | RuntimeMessageRole::User => {
                let source = if message.role == RuntimeMessageRole::System {
                    "system"
                } else {
                    "user"
                };
                let mut omitted = Vec::new();
                let content = build_message_content(message, redact, &mut omitted);
                let mut step = Map::new();
                step.insert("timestamp".to_string(), ts);
                step.insert("source".to_string(), json!(source));
                step.insert("message".to_string(), content);
                let mut extra = Map::new();
                append_omitted_images(&mut extra, omitted);
                if !extra.is_empty() {
                    step.insert("extra".to_string(), Value::Object(extra));
                }
                steps.push(Value::Object(step));
                open_agent = None;
            }
            RuntimeMessageRole::Agent => {
                let mut omitted = Vec::new();
                let content = build_message_content(message, redact, &mut omitted);
                let mut step = Map::new();
                step.insert("timestamp".to_string(), ts);
                step.insert("source".to_string(), json!("agent"));
                step.insert("message".to_string(), content);
                // ATIF carries readable reasoning only; opaque signatures and
                // encrypted payloads are replay state and never exported.
                if let Some(thinking) = message.reasoning_display_text() {
                    let reasoning = if redact {
                        REDACTED.to_string()
                    } else {
                        thinking
                    };
                    step.insert("reasoning_content".to_string(), json!(reasoning));
                }
                let tool_calls: Vec<Value> = message
                    .tool_calls()
                    .iter()
                    .map(|tc| {
                        tool_names_by_call_id.insert(tc.id.clone(), tc.name.clone());
                        json!({
                            "tool_call_id": tc.id,
                            "function_name": tc.name,
                            "arguments": if redact {
                                Value::String(REDACTED.to_string())
                            } else {
                                tc.arguments.clone()
                            },
                        })
                    })
                    .collect();
                if !tool_calls.is_empty() {
                    step.insert("tool_calls".to_string(), Value::Array(tool_calls));
                }
                let mut extra = Map::new();
                append_omitted_images(&mut extra, omitted);
                if !extra.is_empty() {
                    step.insert("extra".to_string(), Value::Object(extra));
                }
                steps.push(Value::Object(step));
                open_agent = Some(steps.len() - 1);
            }
            RuntimeMessageRole::ToolResult => {
                let mut omitted = Vec::new();
                let tool_name = message.tool_result_content().and_then(|tr| {
                    tool_names_by_call_id
                        .get(&tr.tool_call_id)
                        .map(String::as_str)
                });
                let Some(observation) =
                    message_tool_result_observation(message, redact, &mut omitted, tool_name)
                else {
                    continue;
                };
                // Attach to the open agent step, synthesizing a bare one if a
                // tool result somehow leads (defensive; normally impossible).
                let idx = match open_agent {
                    Some(i) => i,
                    None => {
                        let mut step = Map::new();
                        step.insert("timestamp".to_string(), ts);
                        step.insert("source".to_string(), json!("agent"));
                        step.insert("message".to_string(), json!(""));
                        steps.push(Value::Object(step));
                        let i = steps.len() - 1;
                        open_agent = Some(i);
                        i
                    }
                };
                if let Value::Object(step) = &mut steps[idx] {
                    if !omitted.is_empty() {
                        match step.get_mut("extra") {
                            Some(Value::Object(m)) => append_omitted_images(m, omitted),
                            _ => {
                                let mut m = Map::new();
                                append_omitted_images(&mut m, omitted);
                                step.insert("extra".to_string(), Value::Object(m));
                            }
                        }
                    }
                    match step.get_mut("observation") {
                        Some(Value::Object(obs)) => {
                            if let Some(Value::Array(results)) = obs.get_mut("results") {
                                results.push(observation);
                            }
                        }
                        _ => {
                            step.insert(
                                "observation".to_string(),
                                json!({ "results": [observation] }),
                            );
                        }
                    }
                }
            }
        }
    }

    for (i, step) in steps.iter_mut().enumerate() {
        if let Value::Object(map) = step {
            map.insert("step_id".to_string(), json!(i + 1));
        }
    }
    steps
}

/// Build one ATIF observation from a model-view tool-result message: the
/// result/error text (plus any attached images as image ContentParts) and, when
/// the result is a subagent spawn, the `subagent_trajectory_ref` linkage.
fn message_tool_result_observation(
    message: &RuntimeMessage,
    redact: bool,
    omitted: &mut Vec<Value>,
    tool_name: Option<&str>,
) -> Option<Value> {
    let tr = message.tool_result_content()?;
    let base = if let Some(err) = &tr.error {
        format!("[error] {err}")
    } else if let Some(res) = &tr.result {
        match res {
            Value::String(s) => s.clone(),
            other => serde_json::to_string(other).unwrap_or_default(),
        }
    } else {
        String::new()
    };
    let base = if redact { REDACTED.to_string() } else { base };

    // Images carried alongside the result (tool_result_with_images) export as
    // multimodal image ContentParts, matching the event-fold behavior.
    let mut atif_parts: Vec<Value> = vec![json!({ "type": "text", "text": base.clone() })];
    let mut has_image = false;
    for part in &message.content {
        if matches!(part, ContentPart::Image(_) | ContentPart::ImageFile(_)) {
            match image_source(part, redact) {
                Some(source) => {
                    has_image = true;
                    atif_parts.push(json!({ "type": "image", "source": source }));
                }
                None => {
                    atif_parts.push(json!({ "type": "text", "text": "[image]" }));
                    omitted.extend(omitted_image_ref(part, redact));
                }
            }
        }
    }
    let content = if has_image {
        Value::Array(atif_parts)
    } else {
        Value::String(base)
    };

    let mut obs = Map::new();
    obs.insert("source_call_id".to_string(), json!(tr.tool_call_id));
    obs.insert("content".to_string(), content);
    let mut extra = Map::new();
    extra.insert(
        "status".to_string(),
        json!(if tr.error.is_some() {
            "error"
        } else {
            "success"
        }),
    );
    obs.insert("extra".to_string(), Value::Object(extra));
    // Subagent linkage parity with the event fold: only spawn_agent results
    // treat `subagent_id` as structural metadata. Other tools may return a
    // user-controlled field with that name, which must remain content and
    // therefore be redacted with the observation body.
    if tool_name == Some("spawn_agent")
        && let Some(res) = &tr.result
        && let Some(id) = res.get("subagent_id").and_then(Value::as_str)
        && !id.is_empty()
    {
        obs.insert(
            "subagent_trajectory_ref".to_string(),
            subagent_trajectory_ref(id),
        );
    }
    Some(Value::Object(obs))
}

/// Case-level `final_metrics` for the model-view fold. Messages carry no
/// per-step usage, so token totals come from the case result (the same source
/// the `trajectory` format's `metadata` uses).
fn case_final_metrics(result: &EvalCaseResult, total_steps: usize) -> Value {
    let mut m = Map::new();
    if let Some(t) = result.input_tokens {
        m.insert("total_prompt_tokens".to_string(), json!(t));
    }
    if let Some(t) = result.output_tokens {
        m.insert("total_completion_tokens".to_string(), json!(t));
    }
    m.insert("total_steps".to_string(), json!(total_steps));
    Value::Object(m)
}

// ============================================================================
// Segmented export (large sessions over the size cap)
// ============================================================================
//
// The whole-document export (`?format=atif`) is a single synchronous JSON body
// capped at `ATIF_EXPORT_MAX_BYTES` (413 over the cap). Segmented export
// (`?format=atif&segmented=true`) is the recoverable path: it folds the same
// event log into steps once and hands them out in byte-bounded segments, each a
// standalone ATIF-v1.7 document. When steps remain, a segment carries the RFC's
// `continued_trajectory_ref` (a string reference — Harbor RFC 0001 defines it as
// a scalar pointer to the continuation file) pointing at the next segment's URL,
// which embeds an opaque `cursor`. Concatenating every segment's `steps[]` in
// order reproduces the whole-document `steps[]` (absolute `step_id`s preserved).

/// Opaque continuation cursor for segmented ATIF export.
///
/// Encodes the session id it was minted for plus the absolute step offset
/// (0-based index into the full folded step list) where the next segment
/// begins. It is base64url(JSON) — not encrypted, but tamper-evident by
/// construction: the session binding is re-checked against the path session on
/// decode (a cursor forged for another session is rejected), and the offset is
/// bounds-checked against the resolved session's own step count. It therefore
/// cannot widen scope (the session is always resolved org-scoped from the path)
/// or drive unbounded work (the offset only indexes into that session's steps).
#[derive(Debug, Clone, PartialEq, Eq)]
struct SegmentCursor {
    session_id: String,
    /// Absolute 0-based step offset where the next segment starts.
    offset: usize,
}

/// Cursor schema tag, bumped if the wire shape changes.
const CURSOR_VERSION: u8 = 1;
/// Hard cap on an accepted cursor's base64 length, so a bogus giant `cursor=`
/// query value is rejected before any allocation/parsing work (TM-DOS/TM-API).
const MAX_CURSOR_BYTES: usize = 4096;

impl SegmentCursor {
    fn encode(&self) -> String {
        use base64::Engine;
        let payload = json!({
            "v": CURSOR_VERSION,
            "sid": self.session_id,
            "off": self.offset,
        });
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&payload).unwrap_or_default())
    }

    /// Decode and validate a cursor for `expected_session`. Returns the absolute
    /// step offset. Any malformed input, version mismatch, or session mismatch
    /// is a caller error (surfaced as HTTP 400), never a panic.
    fn decode(raw: &str, expected_session: &str) -> Result<usize, CursorError> {
        use base64::Engine;
        if raw.len() > MAX_CURSOR_BYTES {
            return Err(CursorError("cursor is too long"));
        }
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(raw.as_bytes())
            .map_err(|_| CursorError("cursor is not valid base64url"))?;
        let value: Value =
            serde_json::from_slice(&bytes).map_err(|_| CursorError("cursor is not valid JSON"))?;
        let obj = value
            .as_object()
            .ok_or(CursorError("cursor is malformed"))?;
        if obj.get("v").and_then(Value::as_u64) != Some(CURSOR_VERSION as u64) {
            return Err(CursorError("unsupported cursor version"));
        }
        let sid = obj
            .get("sid")
            .and_then(Value::as_str)
            .ok_or(CursorError("cursor is missing a session"))?;
        // THREAT[TM-TENANT]: the cursor's session must match the path session.
        // Scope is always the path session (resolved org-scoped by the caller);
        // this check just rejects a cursor minted for a different session so a
        // stale/confused cursor cannot silently read another trajectory.
        if sid != expected_session {
            return Err(CursorError("cursor does not belong to this session"));
        }
        let offset = obj
            .get("off")
            .and_then(Value::as_u64)
            .ok_or(CursorError("cursor is missing an offset"))?;
        usize::try_from(offset).map_err(|_| CursorError("cursor offset is out of range"))
    }
}

/// Validate and decode the cheap, session-bound parts of a segmented ATIF cursor.
///
/// This must run before loading or folding session events in request handlers so
/// malformed, oversized, or foreign-session cursors fail without expensive work.
pub fn decode_segment_cursor(cursor: &str, expected_session: &str) -> Result<usize, CursorError> {
    SegmentCursor::decode(cursor, expected_session)
}

/// A rejected cursor. Rendered as an HTTP 400 by the export route; never a panic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CursorError(pub &'static str);

impl std::fmt::Display for CursorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for CursorError {}

/// One byte-bounded segment of a segmented ATIF export.
#[derive(Debug)]
pub struct SegmentedTrajectory {
    /// A standalone, valid ATIF-v1.7 document holding this segment's steps.
    pub document: Value,
    /// Images flattened to markers within THIS segment's steps (per-segment
    /// `X-Atif-Images-Omitted` semantics; mirrored in root `extra.images_omitted`).
    pub images_omitted: usize,
    /// Opaque cursor selecting the next segment, or `None` on the final/only
    /// segment. When present it is also embedded in the document's
    /// `continued_trajectory_ref` URL.
    pub next_cursor: Option<String>,
    /// 0-based index of this segment in the chain.
    pub segment_index: usize,
}

/// Build one segment of a segmented ATIF export.
///
/// `cursor` is `None` for the first segment, or the opaque cursor returned by
/// the previous segment. `max_bytes` bounds the serialized segment: steps are
/// added greedily until the next step would push the segment over the cap, and
/// every segment holds at least one step. A single step whose own serialization
/// exceeds `max_bytes` is returned alone as its segment (and may exceed the cap
/// — the only way to make progress without dropping data). `link_base` is the
/// absolute request path used to build `continued_trajectory_ref`
/// (e.g. `/v1/sessions/{id}/export`); the query is appended by this function.
///
/// Errors only on a bad `cursor` (→ HTTP 400).
pub fn build_segment(
    session_id: &str,
    events: &[Event],
    options: AtifOptions,
    cursor: Option<&str>,
    max_bytes: usize,
    link_base: &str,
) -> Result<SegmentedTrajectory, CursorError> {
    // Validate attacker-controlled cursor syntax/session binding before the
    // expensive event fold. Only the offset bounds check needs folded step count.
    let start = match cursor {
        None => 0,
        Some(raw) => decode_segment_cursor(raw, session_id)?,
    };

    let folded = fold_events(events, options.redact_content);
    let all_steps = folded.steps;
    let total = all_steps.len();

    // An offset at/after the end is only valid when it is exactly the end (an
    // empty session, or a client re-fetching past the tail).
    if start > total {
        return Err(CursorError("cursor offset is past the end of the session"));
    }

    // Segment index is derivable from the offset only when segments are uniform,
    // which they are not (byte-bounded). Recompute it by replaying the greedy
    // packing from 0 up to `start` so the index is exact and cheap.
    let segment_index = segment_index_for_offset(&all_steps, start, max_bytes);

    // Pack steps [start..end] greedily under the byte budget. `start == total`
    // (empty session, or a cursor at the tail) yields an empty final segment
    // with no continuation, mirroring the whole-doc export of an empty session.
    let end = if start < total {
        pack_end(&all_steps, start, max_bytes)
    } else {
        start
    };
    let has_more = end < total;

    let segment_steps: Vec<Value> = all_steps[start..end].to_vec();
    let images_omitted = count_omitted_images(&segment_steps);

    let mut root_extra = Map::new();
    root_extra.insert("segment_index".to_string(), json!(segment_index));
    if images_omitted > 0 {
        root_extra.insert("images_omitted".to_string(), json!(images_omitted));
    }
    // The session-level turn roll-up is global; carry it once, on the final
    // segment, so it is not duplicated across the chain.
    if !has_more && !folded.turns.is_empty() {
        root_extra.insert("turns".to_string(), Value::Array(folded.turns));
    }

    let next_cursor = has_more.then(|| {
        SegmentCursor {
            session_id: session_id.to_string(),
            offset: end,
        }
        .encode()
    });

    let final_metrics = segment_final_metrics(&segment_steps);
    let trajectory_id = format!("{session_id}#segment-{segment_index}");
    let mut document = assemble_document(
        Some(session_id),
        Some(&trajectory_id),
        folded.model_name,
        final_metrics,
        segment_steps,
        root_extra,
    );
    // Same always-on secret scrubbing as the whole-doc path, per segment. Run
    // it BEFORE inserting the continuation link so the scrubber cannot corrupt
    // the opaque base64url cursor (which is machine-generated, not user content
    // — it carries nothing to scrub).
    sanitize_value(&mut document, false);
    if let Some(cursor) = &next_cursor {
        let reference = json!(continuation_ref(link_base, cursor));
        if let Value::Object(map) = &mut document {
            // Per Harbor RFC 0001, `continued_trajectory_ref` is a root string
            // field; also mirror it into `extra` as segment bookkeeping.
            map.insert("continued_trajectory_ref".to_string(), reference.clone());
            if let Some(Value::Object(extra)) = map.get_mut("extra") {
                extra.insert("continued_trajectory_ref".to_string(), reference);
            }
        }
    }

    Ok(SegmentedTrajectory {
        document,
        images_omitted,
        next_cursor,
        segment_index,
    })
}

/// The `continued_trajectory_ref` value: the export path with the segmented
/// query and the opaque cursor. A relative reference so it works regardless of
/// host/scheme; consumers resolve it against the request origin.
fn continuation_ref(link_base: &str, cursor: &str) -> String {
    format!("{link_base}?format=atif&segmented=true&cursor={cursor}")
}

/// Greedily choose the exclusive end index for a segment starting at `start`.
/// Always returns at least `start + 1` (one step per segment). A step that
/// alone exceeds `max_bytes` is still returned alone.
fn pack_end(steps: &[Value], start: usize, max_bytes: usize) -> usize {
    debug_assert!(start < steps.len(), "callers guarantee at least one step");
    // Document scaffold overhead (schema_version, agent, final_metrics keys,
    // trajectory_id, extra, continued_trajectory_ref). A generous fixed reserve
    // keeps packing O(n) without serializing the whole doc per candidate; the
    // reserve is far below the 50 MiB production cap and the final segment is
    // never re-checked because at least one step is always emitted.
    const SCAFFOLD_RESERVE: usize = 4096;
    let budget = max_bytes.saturating_sub(SCAFFOLD_RESERVE);

    let mut end = start + 1;
    let mut used = step_bytes(&steps[start]);
    while end < steps.len() {
        let next = step_bytes(&steps[end]) + 1; // +1 for the array comma
        if used + next > budget {
            break;
        }
        used += next;
        end += 1;
    }
    end
}

/// Serialized byte length of one step (used for byte-budget packing).
fn step_bytes(step: &Value) -> usize {
    serde_json::to_vec(step).map(|v| v.len()).unwrap_or(0)
}

/// Replay greedy packing from offset 0 to derive the 0-based segment index that
/// begins at `start`. Cheap: one pass over the prefix's step sizes.
fn segment_index_for_offset(steps: &[Value], start: usize, max_bytes: usize) -> usize {
    let mut index = 0;
    let mut cursor = 0;
    while cursor < start {
        cursor = pack_end(steps, cursor, max_bytes);
        index += 1;
    }
    index
}

/// Count image markers (`extra.omitted_images[]` entries) across a segment's
/// steps, for per-segment `images_omitted` bookkeeping.
fn count_omitted_images(steps: &[Value]) -> usize {
    steps
        .iter()
        .filter_map(|s| s.get("extra"))
        .filter_map(|e| e.get("omitted_images"))
        .filter_map(Value::as_array)
        .map(Vec::len)
        .sum()
}

/// Per-segment `final_metrics`, summed over the segment's own agent steps. A
/// segment is a standalone trajectory, so its `final_metrics` describe only its
/// steps (not the whole session).
fn segment_final_metrics(steps: &[Value]) -> Value {
    let mut totals = Totals::default();
    for step in steps {
        let Some(metrics) = step.get("metrics").and_then(Value::as_object) else {
            continue;
        };
        totals.any = true;
        totals.prompt += metrics
            .get("prompt_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        totals.completion += metrics
            .get("completion_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        totals.cached += metrics
            .get("cached_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        if let Some(cost) = metrics.get("cost_usd").and_then(Value::as_f64) {
            *totals.cost_usd.get_or_insert(0.0) += cost;
        }
    }
    totals.to_final_metrics(steps.len())
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
    /// ATIF `message` value: a JSON string for text-only content, or a
    /// ContentPart array (RFC v1.6 multimodal) when materialized image parts
    /// are present.
    message: Value,
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
            message: Value::String(String::new()),
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
    /// Image content parts that could not be materialized and were flattened to
    /// `"[image]"` markers so far (materialized images do not count).
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
                let message = build_message_content(&data.message, self.redact, &mut omitted);
                self.images_omitted += omitted.len();
                let mut step = Map::new();
                step.insert("timestamp".to_string(), json!(timestamp(event)));
                step.insert("source".to_string(), json!("user"));
                step.insert("message".to_string(), message);
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
                acc.message = build_message_content(&data.message, self.redact, &mut omitted);
                self.images_omitted += omitted.len();
                append_omitted_images(&mut acc.extra, omitted);
                acc.reasoning = data
                    .message
                    .reasoning_display_text()
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
                let content: Value = if let Some(err) = &data.error {
                    Value::String(self.content_or_redacted(format!("[error] {err}")))
                } else {
                    match &data.result {
                        Some(parts) => build_observation_content(parts, self.redact, &mut omitted),
                        None => Value::String(String::new()),
                    }
                };
                self.images_omitted += omitted.len();
                let mut extra = Map::new();
                extra.insert("tool_name".to_string(), json!(data.tool_name));
                extra.insert("status".to_string(), json!(data.status));
                if let Some(d) = data.duration_ms {
                    extra.insert("duration_ms".to_string(), json!(d));
                }
                let acc = self.open.get_or_insert_with(|| AgentStepAcc::new(event.ts));
                append_omitted_images(&mut acc.extra, omitted);
                let mut observation = Map::new();
                observation.insert("source_call_id".to_string(), json!(data.tool_call_id));
                observation.insert("content".to_string(), content);
                observation.insert("extra".to_string(), Value::Object(extra));
                // Subagent linkage (Harbor RFC 0001): when this tool call spawned
                // a subagent, attach a `subagent_trajectory_ref` pointing at the
                // child session's own ATIF export. Ref-only — the child's events
                // are not embedded (this fold sees only one session's events; see
                // knowledge/evaluation/atif-adoption.md, "Subagent trajectories").
                if let Some(child) = subagent_child_session(data) {
                    observation.insert(
                        "subagent_trajectory_ref".to_string(),
                        subagent_trajectory_ref(&child),
                    );
                }
                acc.observations.push(Value::Object(observation));
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
        step.insert("message".to_string(), acc.message);
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

/// Build the ATIF `message` value for a step message. Tool-call/tool-result
/// parts are represented elsewhere on the step and are skipped here.
fn build_message_content(
    message: &RuntimeMessage,
    redact: bool,
    omitted: &mut Vec<Value>,
) -> Value {
    build_content_value(&message.content, redact, false, omitted)
}

/// Build the ATIF observation `content` value for a tool result. Non-media
/// parts (tool calls/results embedded in a result) are serialized to text as
/// before.
fn build_observation_content(
    parts: &[ContentPart],
    redact: bool,
    omitted: &mut Vec<Value>,
) -> Value {
    build_content_value(parts, redact, true, omitted)
}

/// Fold content parts into an ATIF `message`/`content` value.
///
/// Text-only content yields a JSON **string** (backward compatible, and what
/// text-only consumers/scorers expect). When at least one image part can be
/// materialized into an ATIF image ContentPart, the value is a ContentPart
/// **array** (Harbor RFC 0001 / ATIF v1.6 multimodal): ordered text and image
/// parts. The RFC allows `message`/`content` to be a string OR an array, so
/// both shapes are spec-legal; consumers that only read text can still project
/// the `text` parts out of an array.
///
/// `serialize_other` controls non-media parts (`ToolCall`/`ToolResult`): step
/// messages skip them (represented elsewhere on the step), observations
/// serialize them to a JSON text part as before.
///
/// An image that cannot be materialized (an inline image with neither a URL nor
/// base64 bytes) is flattened to an `"[image]"` text marker and its locator is
/// appended to `omitted`; such images keep the legacy `extra.omitted_images`
/// bookkeeping.
fn build_content_value(
    parts: &[ContentPart],
    redact: bool,
    serialize_other: bool,
    omitted: &mut Vec<Value>,
) -> Value {
    let redacted = |s: &str| {
        if redact {
            REDACTED.to_string()
        } else {
            s.to_string()
        }
    };
    let mut atif_parts: Vec<Value> = Vec::new();
    let mut has_image = false;
    for part in parts {
        match part {
            ContentPart::Text(t) => {
                atif_parts.push(json!({ "type": "text", "text": redacted(&t.text) }));
            }
            ContentPart::File(_) => match file_source(part, redact) {
                Some(source) => {
                    atif_parts.push(json!({ "type": "file", "source": source }));
                }
                None => {
                    atif_parts.push(json!({ "type": "text", "text": "[file]" }));
                }
            },
            ContentPart::Image(_) | ContentPart::ImageFile(_) => match image_source(part, redact) {
                Some(source) => {
                    has_image = true;
                    atif_parts.push(json!({ "type": "image", "source": source }));
                }
                None => {
                    // Genuinely unmaterializable: keep the marker + locator so
                    // the omission is still visible and counted.
                    atif_parts.push(json!({ "type": "text", "text": "[image]" }));
                    omitted.extend(omitted_image_ref(part, redact));
                }
            },
            ContentPart::ToolCall(_) | ContentPart::ToolResult(_) => {
                if serialize_other {
                    atif_parts.push(json!({
                        "type": "text",
                        "text": redacted(&serde_json::to_string(part).unwrap_or_default()),
                    }));
                }
            }
            // Reasoning is exported as ATIF `reasoning_content` on the step, not
            // as message content. Serializing the part here would also put the
            // opaque signature and encrypted payload into the export.
            ContentPart::Reasoning(_) => {}
            // `ContentPart` is `#[non_exhaustive]`; this arm is required outside
            // `everruns-core` and is unreachable in-workspace, where every crate
            // shares one core version.
            _ => {}
        }
    }
    if has_image {
        Value::Array(atif_parts)
    } else {
        // No materialized image → collapse to the text projection (a string),
        // preserving the prior behavior for text-only and omitted-image content.
        let text = atif_parts
            .iter()
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n");
        Value::String(text)
    }
}

/// Build an ATIF image ContentPart `source` for an image part, or `None` when
/// the image cannot be materialized (an inline image with neither URL nor
/// base64). Prefers a lean reference over inlined bytes:
/// - `Image { url }` → `source.path = url`.
/// - `ImageFile { image_id }` → `source.path` = the org-scoped file-serving
///   route (`/v1/images/{image_id}`); a consumer with the same auth fetches the
///   bytes. Keeps documents small and within the export size cap.
/// - `Image { base64 }` (no URL) → an inline `data:` URI in `source.path`. This
///   is the only self-contained representation, but it bloats the document and
///   counts toward the segment byte cap, so it is a last resort.
///
/// `media_type` (structural) is preserved when known. Under `redact`, the
/// content-bearing `path` is blanked while the structural `media_type` is kept.
fn image_source(part: &ContentPart, redact: bool) -> Option<Value> {
    let mut source = Map::new();
    match part {
        ContentPart::Image(image) => {
            // Materializable only if there is some locator to point at.
            if image.url.is_none() && image.base64.is_none() {
                return None;
            }
            if let Some(media_type) = &image.media_type {
                source.insert("media_type".to_string(), json!(media_type));
            }
            let path = if redact {
                REDACTED.to_string()
            } else if let Some(url) = &image.url {
                url.clone()
            } else {
                let b64 = image.base64.as_deref().unwrap_or_default();
                let media_type = image.media_type.as_deref().unwrap_or("image/png");
                format!("data:{media_type};base64,{b64}")
            };
            source.insert("path".to_string(), json!(path));
        }
        ContentPart::ImageFile(file) => {
            let path = if redact {
                REDACTED.to_string()
            } else {
                format!("/v1/images/{}", file.image_id)
            };
            source.insert("path".to_string(), json!(path));
        }
        _ => return None,
    }
    Some(Value::Object(source))
}

/// Resolve a file (e.g. PDF) attachment to its ATIF `source`: a lean
/// reference to the file bytes via the files API.
fn file_source(part: &ContentPart, redact: bool) -> Option<Value> {
    let redacted = |s: &str| {
        if redact {
            REDACTED.to_string()
        } else {
            s.to_string()
        }
    };
    match part {
        ContentPart::File(f) => {
            let mut source = Map::new();
            source.insert(
                "path".to_string(),
                json!(format!("/v1/files/{}", f.file_id)),
            );
            if let Some(filename) = &f.filename {
                source.insert("filename".to_string(), json!(redacted(filename)));
            }
            Some(Value::Object(source))
        }
        _ => None,
    }
}

/// The child session id of a subagent spawn, if this tool result is one.
///
/// The `spawn_agent` tool (subagent target) returns a JSON object carrying
/// `subagent_id` = the child session id; the tool result is emitted as
/// a single text ContentPart holding that JSON. Returns `None` for any other
/// tool or a result without a `subagent_id`.
fn subagent_child_session(data: &everruns_core::events::ToolCompletedData) -> Option<String> {
    if data.tool_name != "spawn_agent" {
        return None;
    }
    for part in data.result.as_ref()? {
        if let ContentPart::Text(t) = part
            && let Ok(value) = serde_json::from_str::<Value>(&t.text)
            && let Some(id) = value.get("subagent_id").and_then(Value::as_str)
            && !id.is_empty()
        {
            return Some(id.to_string());
        }
    }
    None
}

/// Build the `subagent_trajectory_ref` array for an observation that spawned the
/// child session `child_session_id`. Ref-only: `trajectory_path` points at the
/// child's own ATIF export (a resolvable location per Harbor RFC 0001, which
/// requires at least one of `trajectory_id`/`trajectory_path`); `session_id` is
/// informational. The child trajectory is not embedded (see the ToolCompleted
/// handler).
fn subagent_trajectory_ref(child_session_id: &str) -> Value {
    json!([{
        "trajectory_path": format!("/v1/sessions/{child_session_id}/export?format=atif"),
        "session_id": child_session_id,
    }])
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

/// Import body cap (NDJSON or JSON). Sized like other untrusted-import caps.
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
    pub conversation: Vec<EvalInputMessage>,
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
                conversation.push(EvalInputMessage {
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
mod tests;

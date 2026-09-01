// Dataset export: turn a completed eval run into a reward-labeled trajectory
// dataset (NDJSON). See `knowledge/evaluation/dataset-export.md`.
//
// This module is pure (no IO): the command in `commands.rs` loads the run and
// each case's model-view messages, then calls into here to filter, redact, and
// serialize one NDJSON record per surviving case.

use std::sync::LazyLock;

use everruns_core::message::{ContentPart, Message, MessageRole};
use everruns_platform::eval::{CaseResultStatus, EvalCaseResult, EvalRun};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use utoipa::ToSchema;

/// Output schema for the exported dataset.
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DatasetFormat {
    /// Generic, canonical trajectory record (full model-view messages + reward +
    /// metadata).
    #[default]
    Trajectory,
    /// Chat-message shape loadable by common SFT pipelines, with reward carried
    /// in a sidecar field for verifiable-reward filtering before training.
    Sft,
    /// One complete ATIF (Agent Trajectory Interchange Format) trajectory per
    /// line, folded from the case session's event log, with reward and case
    /// identity in root `extra`. See `knowledge/evaluation/atif-adoption.md`.
    Atif,
}

/// Selection filters applied per case. `org_id` is never part of this — it is
/// injected by the command from the authenticated caller, so a filter can never
/// widen the org scope.
#[derive(Debug, Clone, Default, Deserialize, Serialize, ToSchema)]
pub struct DatasetFilters {
    /// Keep only cases whose pass/fail equals this.
    #[serde(default)]
    pub pass: Option<bool>,
    /// Keep only cases whose mean scorer value is >= this (0.0–1.0).
    #[serde(default)]
    pub min_score: Option<f64>,
}

/// Redaction controls. Secret scrubbing is always applied regardless of these.
#[derive(Debug, Clone, Default, Deserialize, Serialize, ToSchema)]
pub struct RedactionOptions {
    /// When true, replace message text/tool content with a placeholder, keeping
    /// only structure (roles, tool names, ids). Secret scrubbing still runs.
    #[serde(default)]
    pub redact_content: bool,
}

/// Request body for `POST /v1/evals/{eval_id}/runs/{run_id}/dataset`.
#[derive(Debug, Clone, Default, Deserialize, Serialize, ToSchema)]
pub struct ExportEvalRunDatasetRequest {
    /// Output schema (defaults to `trajectory`).
    #[serde(default)]
    pub format: DatasetFormat,
    /// Per-case selection filters.
    #[serde(default)]
    pub filters: DatasetFilters,
    /// Redaction controls.
    #[serde(default)]
    pub redaction: RedactionOptions,
}

/// Keys whose string values carry raw model content (as opposed to structural
/// fields like `role`, `id`, `name`, `type`). Used for full-content redaction.
/// Includes image payload keys (`url`, `base64`) so `redact_content` also blanks
/// inline image data, not just text/tool content.
const CONTENT_KEYS: [&str; 7] = [
    "text",
    "arguments",
    "result",
    "error",
    "thinking",
    "url",
    "base64",
];

/// Structured JSON keys whose values are credentials even when the value itself
/// does not match a standalone token pattern.
/// Substrings (matched against the normalized, separator-stripped, lowercased
/// key) that mark a JSON field as carrying a credential. `contains` matching is
/// used so compound names like `openai_api_key`, `client_secret`, or `x-api-key`
/// are covered, not just the bare words.
const SECRET_KEY_SUBSTRINGS: [&str; 9] = [
    "apikey",
    "accesskey",
    "secretkey",
    "privatekey",
    "secret",
    "password",
    "passwd",
    "credential",
    "authorization",
];

/// High-signal credential patterns scrubbed from every exported string, always.
/// Deliberately conservative to avoid mangling legitimate content.
static SECRET_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        // OpenAI / Anthropic style keys: sk-..., sk-ant-...
        r"\bsk-[A-Za-z0-9_-]{16,}\b",
        // AWS access key id
        r"\bAKIA[0-9A-Z]{16}\b",
        // GitHub tokens
        r"\bgh[pousr]_[A-Za-z0-9]{20,}\b",
        // Bearer tokens
        r"(?i)\bbearer\s+[A-Za-z0-9._~+/=-]{16,}",
        // key/secret/password/token assignments: api_key=..., "secret": "..."
        r#"(?i)\b(api[_-]?key|secret|password|passwd|token|access[_-]?key)\b\s*["']?\s*[:=]\s*["']?[A-Za-z0-9._~+/=-]{6,}"#,
        // Credential values passed to a scripted command as a flag, which is how
        // `set_agent_credential_value --value '<secret>'` reaches a transcript.
        r#"(?i)--(value|channel[_-]?key|api[_-]?key|token|secret|password)[= ]+["']?[A-Za-z0-9._~+/=-]{6,}["']?"#,
        // Visti-style scoped channel keys: vsk_<channel>_<name>_<secret>
        r"\bvsk_[A-Za-z0-9-]+_[A-Za-z0-9-]+_[A-Za-z0-9]{16,}\b",
    ]
    .iter()
    .map(|p| Regex::new(p).expect("valid secret regex"))
    .collect()
});

pub(crate) const REDACTED: &str = "[REDACTED]";

/// Scrub credential-looking substrings from a single string.
pub fn scrub_secrets(input: &str) -> String {
    let mut out = input.to_string();
    for re in SECRET_PATTERNS.iter() {
        out = re.replace_all(&out, REDACTED).into_owned();
    }
    out
}

fn is_secret_key(key: &str) -> bool {
    let normalized = key.to_ascii_lowercase().replace(['-', '_'], "");
    if SECRET_KEY_SUBSTRINGS
        .iter()
        .any(|needle| normalized.contains(needle))
    {
        return true;
    }
    // Redact `token` and any `*token` suffix (accesstoken, refreshtoken,
    // idtoken, sessiontoken, apitoken, ...) while leaving token *count* fields
    // like `input_tokens` / `output_tokens` (plural, `*tokens`) intact so we do
    // not corrupt exported usage metadata.
    normalized.ends_with("token")
}

/// Recursively scrub secrets in every string leaf of `value`. When
/// `redact_content` is set, content-bearing fields are first replaced wholesale
/// with a placeholder (structure is preserved). Shared with the ATIF exporter
/// (`crate::atif`) so every export path applies the same scrubbing policy.
pub(crate) fn sanitize_value(value: &mut Value, redact_content: bool) {
    match value {
        Value::String(s) => *s = scrub_secrets(s),
        Value::Array(items) => {
            for item in items {
                sanitize_value(item, redact_content);
            }
        }
        Value::Object(map) => {
            for (key, val) in map.iter_mut() {
                if is_secret_key(key) || (redact_content && CONTENT_KEYS.contains(&key.as_str())) {
                    *val = Value::String(REDACTED.to_string());
                } else {
                    sanitize_value(val, redact_content);
                }
            }
        }
        _ => {}
    }
}

/// Mean scorer value for a case, or 0.0 when no numeric scores are present.
pub fn mean_score(result: &EvalCaseResult) -> f64 {
    let values: Vec<f64> = result
        .scores
        .as_ref()
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|s| s.get("value").and_then(Value::as_f64))
                .collect()
        })
        .unwrap_or_default();
    if values.is_empty() {
        0.0
    } else {
        values.iter().sum::<f64>() / values.len() as f64
    }
}

/// Whether a case result survives the selection filters.
pub fn case_passes_filters(result: &EvalCaseResult, filters: &DatasetFilters) -> bool {
    if let Some(want) = filters.pass
        && (result.status == CaseResultStatus::Passed) != want
    {
        return false;
    }
    if let Some(min) = filters.min_score
        && mean_score(result) < min
    {
        return false;
    }
    true
}

/// Stable per-record key for idempotent re-export: run + case-result public ids.
pub fn source_key(run: &EvalRun, result: &EvalCaseResult) -> String {
    format!("{}/{}", run.public_id, result.public_id)
}

/// The reward block shared by both formats.
///
/// Scores are persisted as an ordered `Vec<Score>` without immutable scorer
/// identity. Current eval-case scorer definitions are mutable, so historical
/// results must keep stable positional labels instead of joining against current
/// scorer kinds at export time.
pub(crate) fn reward(result: &EvalCaseResult) -> Value {
    let scores = result.scores.as_ref().and_then(Value::as_array);
    let scorers: Vec<Value> = scores
        .map(|arr| {
            arr.iter()
                .enumerate()
                .map(|(i, s)| {
                    let name = json!(format!("scorer_{i}"));
                    json!({
                        "name": name,
                        "value": s.get("value"),
                        "pass": s.get("pass"),
                        "reason": s.get("reason"),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    json!({
        "pass": result.status == CaseResultStatus::Passed,
        "score": mean_score(result),
        "scorers": scorers,
    })
}

/// Efficiency / provenance metadata.
fn metadata(run: &EvalRun, result: &EvalCaseResult) -> Value {
    json!({
        "model": result
            .target
            .as_ref()
            .or(result.target_snapshot.as_ref())
            .and_then(model_of_target)
            .or_else(|| run.model_override.clone()),
        "turns": result.turns,
        "input_tokens": result.input_tokens,
        "output_tokens": result.output_tokens,
        "latency_ms": result.latency_ms,
    })
}

fn model_of_target(target: &everruns_platform::eval::EvalTarget) -> Option<String> {
    // `EvalTarget::Session` carries the model as `model_id`; read that first and
    // fall back to a generic `model` field for other arms.
    let value = serde_json::to_value(target).ok()?;
    value
        .get("model_id")
        .or_else(|| value.get("model"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// SFT chat role string for a message role.
fn sft_role(role: &MessageRole) -> &'static str {
    match role {
        MessageRole::System => "system",
        MessageRole::User => "user",
        MessageRole::Agent => "assistant",
        MessageRole::ToolResult => "tool",
    }
}

/// Best-effort flat text for a message in the SFT shape.
fn message_text(msg: &Message) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(thinking) = msg.reasoning_display_text() {
        parts.push(format!("[thinking] {thinking}"));
    }
    for part in &msg.content {
        match part {
            ContentPart::Text(t) => parts.push(t.text.clone()),
            ContentPart::ToolCall(c) => parts.push(format!(
                "[tool_call {} {}]",
                c.name,
                serde_json::to_string(&c.arguments).unwrap_or_default()
            )),
            ContentPart::ToolResult(r) => {
                if let Some(result) = &r.result {
                    parts.push(serde_json::to_string(result).unwrap_or_default());
                }
                if let Some(err) = &r.error {
                    parts.push(format!("[error] {err}"));
                }
            }
            ContentPart::Image(_) | ContentPart::ImageFile(_) => parts.push("[image]".to_string()),
            // Rendered above via `reasoning_display_text`, ahead of the content
            // parts, so it is not repeated here.
            ContentPart::Reasoning(_) => {}
        }
    }
    parts.join("\n")
}

/// Build one NDJSON record for a case in the requested format.
pub fn build_record(
    format: DatasetFormat,
    run: &EvalRun,
    result: &EvalCaseResult,
    messages: &[Message],
    redaction: &RedactionOptions,
) -> Value {
    let mut record = match format {
        // ATIF is event-based and built by `crate::atif::build_case_record`;
        // the export job never routes it here. Fall back to the canonical
        // trajectory shape defensively rather than panicking.
        DatasetFormat::Trajectory | DatasetFormat::Atif => json!({
            "source_key": source_key(run, result),
            "eval_run_id": run.public_id.to_string(),
            "case_id": result.eval_case_id.to_string(),
            "case_name": result.case_name,
            "session_id": result.session_id.map(|s| s.to_string()),
            "reward": reward(result),
            "messages": serde_json::to_value(messages).unwrap_or_else(|_| json!([])),
            "metadata": metadata(run, result),
        }),
        DatasetFormat::Sft => {
            let chat: Vec<Value> = messages
                .iter()
                .map(|m| {
                    // The SFT `content` key is not in CONTENT_KEYS (trajectory uses
                    // `content` as the message-array container), so blank it here
                    // when redacting; the whole-record pass below still secret-scrubs.
                    let content = if redaction.redact_content {
                        REDACTED.to_string()
                    } else {
                        message_text(m)
                    };
                    json!({ "role": sft_role(&m.role), "content": content })
                })
                .collect();
            json!({
                "source_key": source_key(run, result),
                "messages": chat,
                "reward": reward(result),
            })
        }
    };

    // Always-on secret scrubbing across *every* exported string (case_name,
    // scorer reasons, metadata, messages, …); additionally blank content-bearing
    // fields (incl. image payloads) when redaction is requested.
    sanitize_value(&mut record, redaction.redact_content);
    record
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_core::message::{Message, MessageRole, TextContentPart};
    use everruns_platform::eval::EvalCaseResult;
    use everruns_provider::typed_id::{EvalCaseId, EvalResultId, EvalRunId};

    fn result_with(status: CaseResultStatus, scores: Value) -> EvalCaseResult {
        EvalCaseResult {
            public_id: EvalResultId::new(),
            internal_id: uuid::Uuid::nil(),
            eval_case_id: EvalCaseId::new(),
            case_name: Some("case".into()),
            session_id: None,
            target: None,
            target_snapshot: None,
            status,
            scores: Some(scores),
            metadata: None,
            turns: Some(3),
            latency_ms: Some(1200),
            input_tokens: Some(100),
            output_tokens: Some(50),
            error_message: None,
            artifacts: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    fn text_msg(role: MessageRole, text: &str) -> Message {
        Message {
            id: uuid::Uuid::now_v7().into(),
            role,
            content: vec![ContentPart::Text(TextContentPart::new(text))],
            phase: None,
            phase_source: None,
            controls: None,
            metadata: None,
            external_actor: None,
            created_at: chrono::Utc::now(),
        }
    }

    fn run() -> EvalRun {
        EvalRun {
            public_id: EvalRunId::new(),
            internal_id: uuid::Uuid::nil(),
            org_id: 1,
            target: None,
            model_override: Some("gpt-test".into()),
            filter_tags: None,
            status: everruns_platform::eval::EvalRunStatus::Completed,
            source: everruns_platform::eval::EvalRunSource::Internal,
            attribution: None,
            triggered_by: "test".into(),
            started_at: None,
            completed_at: None,
            summary: None,
            results: vec![],
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn mean_score_averages_values() {
        let r = result_with(
            CaseResultStatus::Passed,
            json!([{"pass": true, "value": 1.0, "reason": "ok"}, {"pass": false, "value": 0.0, "reason": "no"}]),
        );
        assert_eq!(mean_score(&r), 0.5);
    }

    #[test]
    fn filters_pass_and_min_score() {
        let passed = result_with(CaseResultStatus::Passed, json!([{"value": 0.8}]));
        let failed = result_with(CaseResultStatus::Failed, json!([{"value": 0.2}]));

        assert!(case_passes_filters(
            &passed,
            &DatasetFilters {
                pass: Some(true),
                min_score: None
            }
        ));
        assert!(!case_passes_filters(
            &failed,
            &DatasetFilters {
                pass: Some(true),
                min_score: None
            }
        ));
        assert!(case_passes_filters(
            &passed,
            &DatasetFilters {
                pass: None,
                min_score: Some(0.5)
            }
        ));
        assert!(!case_passes_filters(
            &failed,
            &DatasetFilters {
                pass: None,
                min_score: Some(0.5)
            }
        ));
    }

    #[test]
    fn scrub_secrets_redacts_credential_flags_and_scoped_channel_keys() {
        // `set_agent_credential_value` is the one command that carries a secret,
        // so its flag form must never survive into an export.
        let scrubbed = scrub_secrets(
            "set_agent_credential_value --binding_id 01933b5a --value 'vsk_channel-08d9ba08_silver-owl_fgw52m8g1abfkmyk5qf02gcejw'",
        );
        assert!(scrubbed.contains(REDACTED));
        assert!(!scrubbed.contains("fgw52m8g1abfkmyk5qf02gcejw"), "{scrubbed}");
        assert!(scrubbed.contains("set_agent_credential_value"));

        let bare = scrub_secrets("channel is vsk_channel-08d9ba08_silver-owl_fgw52m8g1abfkmyk5qf02gcejw today");
        assert!(!bare.contains("fgw52m8g1abfkmyk5qf02gcejw"), "{bare}");

        // Conservative: ordinary flags are left alone.
        let untouched = scrub_secrets("create_agent --name 'hourly-visti-jokes'");
        assert!(!untouched.contains(REDACTED), "{untouched}");
    }

    #[test]
    fn scrub_secrets_redacts_keys() {
        let scrubbed =
            scrub_secrets("here is sk-abcdef0123456789ABCDEF and AKIAABCDEFGHIJKLMNOP done");
        assert!(scrubbed.contains(REDACTED));
        assert!(!scrubbed.contains("sk-abcdef0123456789"));
        assert!(!scrubbed.contains("AKIAABCDEFGHIJKLMNOP"));
    }

    #[test]
    fn sanitize_value_redacts_structured_secret_fields() {
        let mut value = json!({
            "messages": [{
                "content": [{
                    "type": "tool_call",
                    "arguments": {
                        "api_key": "YExampleApi0",
                        "access-key": "shorttok",
                        "nested": { "password": "YExamplePassword0" }
                    }
                }],
                "metadata": {
                    "secret": "metasecret",
                    "openai_api_key": "raw-openai-value",
                    "client_secret": "raw-client-secret",
                    "refresh_token": "raw-refresh-token",
                    "authorization": "Basic YExample0",
                    "input_tokens": 1234,
                    "safe": "keep me"
                }
            }],
            "token": "bearer-token-value"
        });

        sanitize_value(&mut value, false);

        let serialized = serde_json::to_string(&value).unwrap();
        assert!(!serialized.contains("YExampleApi0"));
        assert!(!serialized.contains("shorttok"));
        assert!(!serialized.contains("YExamplePassword0"));
        assert!(!serialized.contains("metasecret"));
        assert!(!serialized.contains("bearer-token-value"));
        // Compound credential key names must also be redacted.
        assert!(!serialized.contains("raw-openai-value"));
        assert!(!serialized.contains("raw-client-secret"));
        assert!(!serialized.contains("raw-refresh-token"));
        assert!(!serialized.contains("Basic YExample0"));
        assert!(serialized.contains("keep me"));
        // Token *count* metadata must survive (not a credential).
        assert_eq!(
            value["messages"][0]["metadata"]["input_tokens"],
            json!(1234)
        );
        assert_eq!(value["messages"][0]["metadata"]["secret"], json!(REDACTED));
        assert_eq!(
            value["messages"][0]["metadata"]["openai_api_key"],
            json!(REDACTED)
        );
    }

    #[test]
    fn trajectory_record_has_reward_and_messages() {
        let r = result_with(
            CaseResultStatus::Passed,
            json!([{"pass": true, "value": 1.0, "reason": "ok"}]),
        );
        let msgs = vec![
            text_msg(MessageRole::User, "hello"),
            text_msg(MessageRole::Agent, "hi there"),
        ];
        let rec = build_record(
            DatasetFormat::Trajectory,
            &run(),
            &r,
            &msgs,
            &RedactionOptions::default(),
        );
        assert_eq!(rec["reward"]["pass"], json!(true));
        assert_eq!(rec["reward"]["score"], json!(1.0));
        assert_eq!(rec["messages"].as_array().unwrap().len(), 2);
        assert_eq!(rec["metadata"]["turns"], json!(3));
        assert!(rec["source_key"].as_str().unwrap().contains('/'));
    }

    #[test]
    fn sft_record_is_chat_shaped() {
        let r = result_with(CaseResultStatus::Passed, json!([{"value": 1.0}]));
        let msgs = vec![
            text_msg(MessageRole::User, "q"),
            text_msg(MessageRole::Agent, "a"),
        ];
        let rec = build_record(
            DatasetFormat::Sft,
            &run(),
            &r,
            &msgs,
            &RedactionOptions::default(),
        );
        let chat = rec["messages"].as_array().unwrap();
        assert_eq!(chat[0]["role"], json!("user"));
        assert_eq!(chat[1]["role"], json!("assistant"));
        assert_eq!(chat[1]["content"], json!("a"));
        assert!(rec["reward"].is_object());
    }

    #[test]
    fn redact_content_blanks_text_but_keeps_structure() {
        let r = result_with(CaseResultStatus::Passed, json!([{"value": 1.0}]));
        let msgs = vec![text_msg(MessageRole::User, "secret business content")];
        let rec = build_record(
            DatasetFormat::Trajectory,
            &run(),
            &r,
            &msgs,
            &RedactionOptions {
                redact_content: true,
            },
        );
        let serialized = serde_json::to_string(&rec).unwrap();
        assert!(!serialized.contains("secret business content"));
        assert!(serialized.contains(REDACTED));
        // Structure preserved: role still present.
        assert_eq!(rec["messages"][0]["role"], json!("user"));
    }

    #[test]
    fn sft_redacts_content_when_requested() {
        let r = result_with(CaseResultStatus::Passed, json!([{"value": 1.0}]));
        let msgs = vec![text_msg(MessageRole::User, "private text")];
        let rec = build_record(
            DatasetFormat::Sft,
            &run(),
            &r,
            &msgs,
            &RedactionOptions {
                redact_content: true,
            },
        );
        assert_eq!(rec["messages"][0]["content"], json!(REDACTED));
    }

    #[test]
    fn secret_scrubbing_covers_case_name_and_scorer_reason() {
        let mut r = result_with(
            CaseResultStatus::Passed,
            json!([{"value": 1.0, "pass": true, "reason": "leaked sk-abcdef0123456789ABCDEF"}]),
        );
        r.case_name = Some("case AKIAABCDEFGHIJKLMNOP".into());
        let rec = build_record(
            DatasetFormat::Trajectory,
            &run(),
            &r,
            &[],
            &RedactionOptions::default(),
        );
        let serialized = serde_json::to_string(&rec).unwrap();
        // Secrets outside `messages` (case_name, scorer reason) are scrubbed too.
        assert!(!serialized.contains("sk-abcdef0123456789ABCDEF"));
        assert!(!serialized.contains("AKIAABCDEFGHIJKLMNOP"));
        assert!(serialized.contains(REDACTED));
    }

    #[test]
    fn reward_uses_stable_positional_scorer_labels() {
        let r = result_with(
            CaseResultStatus::Passed,
            json!([
                {"pass": true, "value": 1.0, "reason": "has it"},
                {"pass": false, "value": 0.0, "reason": "too many turns"},
            ]),
        );
        let rec = build_record(
            DatasetFormat::Trajectory,
            &run(),
            &r,
            &[],
            &RedactionOptions::default(),
        );
        let scorers = rec["reward"]["scorers"].as_array().unwrap();
        assert_eq!(scorers[0]["name"], json!("scorer_0"));
        assert_eq!(scorers[1]["name"], json!("scorer_1"));
    }

    #[test]
    fn redact_content_blanks_image_payload() {
        use everruns_core::message::ImageContentPart;
        let r = result_with(CaseResultStatus::Passed, json!([{"value": 1.0}]));
        let mut msg = text_msg(MessageRole::User, "x");
        msg.content = vec![ContentPart::Image(ImageContentPart::from_url(
            "https://example.com/SECRETIMAGEDATA.png",
        ))];
        let rec = build_record(
            DatasetFormat::Trajectory,
            &run(),
            &r,
            &[msg],
            &RedactionOptions {
                redact_content: true,
            },
        );
        let serialized = serde_json::to_string(&rec).unwrap();
        assert!(!serialized.contains("SECRETIMAGEDATA"));
    }
}

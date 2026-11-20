// ATIF import: trajectories (v1, and draft v2.0 per Harbor RFC 0004) to eval
// case drafts. Split out of `mod.rs` so the export fold stays one file.

use serde_json::Value;

use crate::domains::evals::record::EvalInputMessage;

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
        // A v2 bundle also carries `trajectories`; keep it whole so
        // `normalize_import_item` can pick its root.
        Ok(Value::Object(obj))
            if obj.get("schema_kind").and_then(Value::as_str) == Some("bundle") =>
        {
            vec![Value::Object(obj)]
        }
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
    // ATIF-v2.0 (Harbor RFC 0004, draft): a bundle packs the root trajectory
    // with its subagent/fork/rewind children. Only the root is the conversation
    // a case should replay, so each bundle collapses to its root. Event-stream
    // lines (ATIF-E) are raw runtime events, not trajectories: reject them with
    // a pointer to consolidate first instead of a confusing per-line error.
    let trajectories = trajectories
        .into_iter()
        .map(normalize_import_item)
        .collect::<Result<Vec<_>, _>>()?;
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

/// Collapse an ATIF-v2.0 bundle to its root trajectory and reject ATIF-E
/// events; anything else (v1 documents, v2 trajectories) passes through.
fn normalize_import_item(item: Value) -> Result<Value, String> {
    match item.get("schema_kind").and_then(Value::as_str) {
        Some("event") => Err(
            "ATIF-E event streams (.atif-e.jsonl) are not importable; consolidate them \
             into an ATIF trajectory (.atif.json) or bundle first"
                .to_string(),
        ),
        Some("bundle") => {
            let children = item
                .get("trajectories")
                .and_then(Value::as_array)
                .ok_or_else(|| "ATIF bundle is missing its `trajectories` array".to_string())?;
            // The RFC lists the root first; prefer the one without a parent in
            // case a producer orders them differently.
            let root = children
                .iter()
                .find(|t| t.get("parent_trajectory_id").is_none_or(Value::is_null))
                .or_else(|| children.first())
                .ok_or_else(|| "ATIF bundle has no trajectories".to_string())?;
            Ok(root.clone())
        }
        _ => Ok(item),
    }
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

    // v1 keeps case identity in root `extra`; v2 moves unpromoted v1 keys
    // under `extensions["com.harborframework.v1"].extra`.
    let extra = obj.get("extra").and_then(Value::as_object);
    let v1_ext_extra = obj
        .get("extensions")
        .and_then(|e| e.get("com.harborframework.v1"))
        .and_then(|e| e.get("extra"))
        .and_then(Value::as_object);
    let extra_str = |key: &str| {
        extra
            .and_then(|e| e.get(key))
            .or_else(|| v1_ext_extra.and_then(|e| e.get(key)))
            .and_then(Value::as_str)
    };
    let name = extra_str("case_name")
        .or_else(|| extra_str("source_key"))
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
    // v2 standardizes the verdict under `com.harborframework.evaluation`.
    let reward = extra
        .and_then(|e| e.get("reward"))
        .or_else(|| v1_ext_extra.and_then(|e| e.get("reward")))
        .or_else(|| {
            obj.get("extensions")
                .and_then(|e| e.get("com.harborframework.evaluation"))
        });
    if let Some(reward) = reward {
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

/// Extract the text of an ATIF step: v1 `message` (string or ContentPart
/// array) or v2 `content` (ContentPart array). Only text parts count;
/// reasoning, action markers and media are not conversation text.
fn step_message_text(step: &Value) -> String {
    match step.get("message").or_else(|| step.get("content")) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|p| {
                // v2 makes `type` optional in event streams; a part with a
                // `text` key and no other type is a text part.
                let ty = p.get("type").and_then(Value::as_str);
                if ty == Some("text") || (ty.is_none() && p.get("text").is_some()) {
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

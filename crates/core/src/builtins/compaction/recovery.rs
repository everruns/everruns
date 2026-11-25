//! Masking that hides a tool result without losing it: which answered results
//! are too large to keep live, and the stub that says how to read one back.

use super::*;

/// Tool results at least `max_live_tool_result_bytes` long that an assistant
/// message has already followed.
///
/// Keeping the newest results verbatim regardless of size held two 60 KiB web
/// pages in every request of a long scheduled session and forced a paid
/// summary every few calls (Ecosystem Watch, 2026-11-24). Once answered, such
/// a result is masked like a stale one; the stub keeps an excerpt and the way
/// to read the original back through `query_history`. Answered stays
/// answered, so the masked view only moves forward and stays cache-stable.
pub(super) fn answered_oversized_results(
    messages: &[RuntimeMessage],
    tool_indices: &[usize],
    config: &CostControlConfig,
) -> Vec<usize> {
    let Some(last_assistant) = messages
        .iter()
        .rposition(|message| message.role == RuntimeMessageRole::Agent)
    else {
        return Vec::new();
    };
    tool_indices
        .iter()
        .copied()
        .filter(|index| {
            *index < last_assistant
                && message_tool_result_len(&messages[*index]) >= config.max_live_tool_result_bytes
        })
        .collect()
}

/// Characters of the original kept in a masked stub, so the model still sees
/// what the result was about.
const MASKED_EXCERPT_CHARS: usize = 400;

/// A masked tool result keeps a short excerpt and says how to read it back:
/// the session journal holds the original, and `query_history` returns it by
/// tool call id, page by page. Masking hides a result; it does not lose it.
pub(super) fn with_recovery_hint(
    summary: String,
    original: &str,
    tool_name: &str,
    tool_call_id: Option<&str>,
) -> String {
    let Some(tool_call_id) = tool_call_id else {
        return summary;
    };
    let excerpt: String = original.chars().take(MASKED_EXCERPT_CHARS).collect();
    let more = if original.chars().count() > MASKED_EXCERPT_CHARS {
        "…"
    } else {
        ""
    };
    format!(
        "{summary}\nHidden to save context. Starts: {excerpt}{more}\nFull output: query_history with tool_call_id \"{tool_call_id}\" (tool_name \"{tool_name}\")."
    )
}

/// Recovery hint for a masked runtime tool result, from its JSON value.
fn masked_result_hint(
    summary: String,
    result: Option<&serde_json::Value>,
    tool_name: &str,
    tool_call_id: &str,
) -> String {
    let original = result
        .map(|value| match value {
            serde_json::Value::String(text) => text.clone(),
            other => other.to_string(),
        })
        .unwrap_or_default();
    with_recovery_hint(summary, &original, tool_name, Some(tool_call_id))
}

pub(super) fn mask_tool_result_message(
    message: &RuntimeMessage,
    tool_name: &str,
) -> RuntimeMessage {
    let Some(result) = message.tool_result_content() else {
        return message.clone();
    };
    let summary = summarize_tool_result(tool_name, result.result.as_ref(), result.error.as_ref());
    let was_error = result.error.is_some();
    let summary = if was_error {
        summary
    } else {
        masked_result_hint(
            summary,
            result.result.as_ref(),
            tool_name,
            &result.tool_call_id,
        )
    };
    let mut masked = message.clone();
    for part in &mut masked.content {
        if let ContentPart::ToolResult(tool_result) = part {
            if was_error {
                tool_result.result = None;
                tool_result.error = Some(summary);
            } else {
                tool_result.result = Some(serde_json::json!({
                    "masked": true,
                    "summary": summary,
                }));
                tool_result.error = None;
            }
            break;
        }
    }
    masked
}

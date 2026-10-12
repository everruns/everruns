//! Reading a `request_approval` pause out of a `tool.completed` event.
//!
//! The `soft_approval` capability pauses a turn with a real tool call: the
//! model ends its turn on `request_approval` and the person's next message
//! answers it. A channel only has to show the question; see
//! [`ChannelApprovalPrompt`](super::ChannelApprovalPrompt).

use serde_json::Value;

use super::ApprovalPrompt;

/// The tool whose completion raises an approval prompt.
pub const REQUEST_APPROVAL_TOOL: &str = "request_approval";

/// The approval a `tool.completed` event's data asks for.
///
/// `None` for every other tool, for a failed call, and for a
/// `request_approval` result that does not say it is waiting: the
/// context-free `execute` path returns the same shape without pausing, and a
/// prompt for it would show buttons nothing answers.
pub fn approval_prompt(data: &Value) -> Option<ApprovalPrompt> {
    if data.get("tool_name")?.as_str()? != REQUEST_APPROVAL_TOOL
        || !data.get("success")?.as_bool().unwrap_or(false)
    {
        return None;
    }
    let payload = result_json(data)?;
    if !payload
        .get("awaiting_approval")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return None;
    }
    let action = payload.get("action")?.as_str()?.trim();
    if action.is_empty() {
        return None;
    }
    let text = |key: &str| {
        payload
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    Some(ApprovalPrompt {
        action: action.to_string(),
        question: text("question"),
        turn_id: text("asked_in_turn"),
    })
}

/// The first text part of a tool result, parsed as JSON.
fn result_json(data: &Value) -> Option<Value> {
    let text = data.get("result")?.as_array()?.iter().find_map(|part| {
        (part.get("type")?.as_str()? == "text")
            .then(|| part.get("text")?.as_str())
            .flatten()
    })?;
    serde_json::from_str(text).ok()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn completed(payload: Value, success: bool) -> Value {
        json!({
            "tool_name": REQUEST_APPROVAL_TOOL,
            "success": success,
            "result": [{"type": "text", "text": payload.to_string()}],
        })
    }

    #[test]
    fn a_waiting_request_becomes_a_prompt() {
        let data = completed(
            json!({"awaiting_approval": true, "action": " Delete the branch ", "question": "OK?", "asked_in_turn": "turn_1"}),
            true,
        );
        let prompt = approval_prompt(&data).unwrap();
        assert_eq!(prompt.action, "Delete the branch");
        assert_eq!(prompt.question.as_deref(), Some("OK?"));
        assert_eq!(prompt.turn_id.as_deref(), Some("turn_1"));
        assert_eq!(prompt.text(), "Approval needed: Delete the branch\nOK?");
    }

    #[test]
    fn anything_that_is_not_waiting_is_no_prompt() {
        let not_waiting = completed(json!({"awaiting_approval": false, "action": "x"}), true);
        let failed = completed(json!({"awaiting_approval": true, "action": "x"}), false);
        let blank = completed(json!({"awaiting_approval": true, "action": "  "}), true);
        let other = json!({"tool_name": "search", "success": true, "result": []});
        for data in [not_waiting, failed, blank, other] {
            assert!(approval_prompt(&data).is_none(), "{data}");
        }
    }
}

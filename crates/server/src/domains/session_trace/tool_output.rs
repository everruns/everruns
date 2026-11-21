//! Decode tool completions for the Trace API.

use serde_json::Value;

pub(super) fn trace_command_outcome(
    completed: &Value,
    result: &Value,
) -> Option<everruns_core::events::CommandOutcome> {
    completed
        .get("command_outcome")
        .and_then(|value| serde_json::from_value(value.clone()).ok())
        .or_else(|| {
            // Earlier events carried the exit code only in the output.
            let tool_name = completed.get("tool_name")?.as_str()?;
            everruns_core::events::CommandOutcome::from_shell_payload(tool_name, result)
        })
}

pub(super) fn tool_result_value(result: Value) -> Value {
    let Value::Array(parts) = &result else {
        return result;
    };
    let texts: Option<Vec<&str>> = parts
        .iter()
        .map(|p| match p.get("type").and_then(Value::as_str) {
            Some("text") => p.get("text").and_then(Value::as_str),
            _ => None,
        })
        .collect();
    match texts {
        Some(texts) if !texts.is_empty() => {
            let text = texts.join("\n");
            serde_json::from_str::<Value>(&text)
                .ok()
                .filter(|v| v.is_object() || v.is_array())
                .unwrap_or(Value::String(text))
        }
        _ => result,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn trace_reports_shell_exit_separately_from_tool_success() {
        let completion = json!({
            "tool_name": "bash",
            "success": true,
            "status": "success",
            "result": [{"type": "text", "text": "{\"stdout\":\"failed tests\",\"exit_code\":7}"}]
        });
        let output = tool_result_value(completion["result"].clone());
        let outcome = trace_command_outcome(&completion, &output).unwrap();
        assert_eq!(outcome.exit_code, 7);
        assert!(!outcome.success);
        assert_eq!(completion["status"], "success");
        assert!(trace_command_outcome(&json!({"tool_name": "web_fetch"}), &output).is_none());

        // A recorded exit remains available even when the output is omitted.
        let explicit = json!({"tool_name": "bash", "command_outcome": outcome});
        assert_eq!(
            trace_command_outcome(&explicit, &Value::Null),
            Some(outcome)
        );
    }
}

//! `report_done`: the agent's own account of finished work.
//!
//! Every turn already gets a work report built by the runtime from its events
//! (`everruns_contracts::runtime::work_report`). This tool lets an agent
//! enrich it: a summary leading with the result, how it checked the work, and
//! links to what it produced. The report reducer reads the call's arguments;
//! the tool itself only validates them, so it has no side effect and nothing
//! else to fail on. It shares its shape with a coordinated worker's
//! `complete_assignment`, which the reducer reads the same way.

use crate::builtins::tool_narration::{ToolNarrationContext, ToolNarrationPhase, generic_phrase};
use crate::builtins::tool_types::{ToolCall, ToolHints};
use crate::builtins::tools::{Tool, ToolExecutionResult};
use async_trait::async_trait;
use serde_json::{Value, json};

/// Bounds the model is told about; longer text is rejected, not cut, so the
/// agent can say it again shorter.
const MAX_SUMMARY_CHARS: usize = 600;
const MAX_VALIDATION_CHARS: usize = 400;
const MAX_ARTIFACTS: usize = 10;

pub struct ReportDoneTool;

#[async_trait]
impl Tool for ReportDoneTool {
    fn narrate(
        &self,
        _tool_call: &ToolCall,
        phase: ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: ToolNarrationContext<'_>,
    ) -> Option<String> {
        let (started, completed, failed) = if locale.is_some_and(|l| l.starts_with("uk")) {
            (
                "Звітую про виконане",
                "Звітував про виконане",
                "Не вдалося звітувати про виконане",
            )
        } else {
            (
                "Reporting what got done",
                "Reported what got done",
                "Could not report what got done",
            )
        };
        Some(generic_phrase(started, completed, failed, None, phase))
    }

    fn name(&self) -> &str {
        "report_done"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Report Done")
    }

    fn description(&self) -> &str {
        "Report finished work once, at the end of substantial work, before your final reply. Lead the summary with the result, say how you checked it, and link what you produced. Use outcome failed when requested work is blocked, incomplete or unverified, and say why. Do not report done merely because the turn is ending; state any pending human step explicitly."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "summary": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": MAX_SUMMARY_CHARS,
                    "description": "One or two sentences, leading with the result (e.g. 'Fixed proration rounding: partial months now round per line')."
                },
                "validation": {
                    "type": "string",
                    "maxLength": MAX_VALIDATION_CHARS,
                    "description": "How you checked it (e.g. 'Ran the 12 billing tests, all pass')."
                },
                "outcome": { "type": "string", "enum": ["done", "failed"], "default": "done" },
                "artifacts": {
                    "type": "array",
                    "maxItems": MAX_ARTIFACTS,
                    "items": {
                        "type": "object",
                        "properties": {
                            "name": { "type": "string" },
                            "type": { "type": "string", "description": "file, url, pr, session, ..." },
                            "url": { "type": "string" },
                            "path": { "type": "string" }
                        },
                        "required": ["name", "type"],
                        "additionalProperties": false
                    }
                }
            },
            "required": ["summary"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default().with_idempotent(true)
    }

    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        match validate(&arguments) {
            Ok(()) => ToolExecutionResult::success(json!({ "status": "reported" })),
            Err(error) => ToolExecutionResult::tool_error(error),
        }
    }
}

fn validate(arguments: &Value) -> Result<(), String> {
    let summary = arguments
        .get("summary")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if summary.is_empty() {
        return Err("Missing 'summary': say what you did, leading with the result.".to_string());
    }
    if summary.chars().count() > MAX_SUMMARY_CHARS {
        return Err(format!(
            "'summary' is over {MAX_SUMMARY_CHARS} characters; keep it to one or two sentences."
        ));
    }
    match arguments.get("validation") {
        None | Some(Value::Null) => {}
        Some(Value::String(text)) if text.chars().count() <= MAX_VALIDATION_CHARS => {}
        Some(Value::String(_)) => {
            return Err(format!(
                "'validation' is over {MAX_VALIDATION_CHARS} characters."
            ));
        }
        Some(_) => return Err("'validation' must be a string.".to_string()),
    }
    match arguments.get("outcome").and_then(Value::as_str) {
        None | Some("done" | "failed") => {}
        Some(other) => return Err(format!("Unknown outcome '{other}': use done or failed.")),
    }
    match arguments.get("artifacts") {
        None | Some(Value::Null) => {}
        Some(Value::Array(items)) => {
            if items.len() > MAX_ARTIFACTS {
                return Err(format!("At most {MAX_ARTIFACTS} artifacts."));
            }
            for (index, item) in items.iter().enumerate() {
                let named = item
                    .get("name")
                    .and_then(Value::as_str)
                    .is_some_and(|n| !n.is_empty());
                let typed = item.get("type").and_then(Value::as_str).is_some();
                if !named || !typed {
                    return Err(format!(
                        "Artifact {} needs a 'name' and a 'type'.",
                        index + 1
                    ));
                }
            }
        }
        Some(_) => return Err("'artifacts' must be an array.".to_string()),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_complete_report_is_accepted() {
        let result = ReportDoneTool
            .execute(json!({
                "summary": "Fixed proration rounding.",
                "validation": "Ran the billing tests.",
                "outcome": "done",
                "artifacts": [{ "name": "PR #1", "type": "pr", "url": "https://example.com/pr/1" }]
            }))
            .await;
        assert!(result.is_success(), "{result:?}");
    }

    #[tokio::test]
    async fn a_report_the_reducer_could_not_use_is_refused_with_a_reason() {
        for (arguments, needle) in [
            (json!({}), "Missing 'summary'"),
            (json!({ "summary": "  " }), "Missing 'summary'"),
            (json!({ "summary": "x".repeat(601) }), "over 600"),
            (
                json!({ "summary": "ok", "outcome": "maybe" }),
                "Unknown outcome",
            ),
            (
                json!({ "summary": "ok", "artifacts": [{ "name": "x" }] }),
                "needs a 'name' and a 'type'",
            ),
            (
                json!({ "summary": "ok", "validation": 3 }),
                "must be a string",
            ),
        ] {
            match ReportDoneTool.execute(arguments).await {
                ToolExecutionResult::ToolError(error) => assert!(error.contains(needle), "{error}"),
                other => panic!("expected a tool error for {needle}, got {other:?}"),
            }
        }
    }
}

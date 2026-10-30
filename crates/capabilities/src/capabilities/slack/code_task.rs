use super::*;

pub struct SlackCodeTaskTool;

#[async_trait]
impl Tool for SlackCodeTaskTool {
    fn hints(&self) -> everruns_contracts::tool_types::ToolHints {
        // A reversible projection into the already-authorized task surface.
        // Consequential repository actions retain their own enforced gate.
        everruns_contracts::tool_types::ToolHints::default()
            .with_destructive(false)
            .with_open_world(false)
            .with_idempotent(true)
    }
    fn name(&self) -> &str {
        "slack_code_task"
    }
    fn display_name(&self) -> Option<&str> {
        Some("Slack: Publish Code Task")
    }
    fn description(&self) -> &str {
        "Publish a complete review snapshot to this session's Slack Code channel. Start expected_revision at 0, then use the returned revision for the next snapshot. Supply repo/branch/PR/check links in context. Generate a real unified diff with git and save it to diff_path; never reconstruct it from conversation. To finish, set complete=true with the final summary after validation. The channel archives only after the turn ends. This tool cannot select another channel."
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type":"object","additionalProperties":false,
            "properties":{
                "expected_revision":{"type":"integer","minimum":0},
                "context":{"type":"array","maxItems":4,"items":{"type":"object","additionalProperties":false,"properties":{"key":{"type":"string"},"label":{"type":"string"},"url":{"type":"string"}},"required":["key","label"]}},
                "diff_path":{"type":"string","description":"Path to a UTF-8 unified diff in the session filesystem."},
                "base_branch":{"type":"string"},"head_branch":{"type":"string"},
                "summary":{"type":"string","maxLength":2800},"complete":{"type":"boolean"}
            },"required":["expected_revision","context","summary"]
        })
    }
    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("Slack Code publication requires session context")
    }
    fn requires_context(&self) -> bool {
        true
    }
    async fn execute_with_context(
        &self,
        mut arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        if let Some(path) = arguments
            .get("diff_path")
            .and_then(Value::as_str)
            .map(str::to_owned)
        {
            let Some(files) = &context.file_store else {
                return ToolExecutionResult::tool_error("Session filesystem unavailable");
            };
            match files.stat_file(context.session_id, &path).await {
                Ok(Some(stat)) if stat.size_bytes <= 1_000_000 && !stat.is_directory => {}
                Ok(Some(_)) => {
                    return ToolExecutionResult::tool_error(
                        "Diff is too large or is a directory. Publish a review URL as a context item instead.",
                    );
                }
                Ok(None) => return ToolExecutionResult::tool_error("Diff file not found"),
                Err(error) => return ToolExecutionResult::internal_error_msg(error.to_string()),
            }
            let file = match files.read_file(context.session_id, &path).await {
                Ok(Some(file)) => file,
                Ok(None) => return ToolExecutionResult::tool_error("Diff file not found"),
                Err(error) => return ToolExecutionResult::internal_error_msg(error.to_string()),
            };
            if file.encoding != "text" || file.content.as_ref().is_none_or(|s| s.len() > 1_000_000)
            {
                return ToolExecutionResult::tool_error(
                    "Diff exceeds Slack's 1,000,000-byte limit or is not UTF-8. Publish its review URL as a context item instead.",
                );
            }
            let Some(object) = arguments.as_object_mut() else {
                return ToolExecutionResult::tool_error("Expected publication object");
            };
            object.remove("diff_path");
            arguments["diff"] = json!(file.content.unwrap_or_default());
        }
        if arguments.to_string().len() > 2_100_000 {
            return ToolExecutionResult::tool_error("Code task publication is too large");
        }
        run(
            context,
            SlackAction::CodeTask {
                request: arguments.to_string(),
            },
        )
        .await
    }
}

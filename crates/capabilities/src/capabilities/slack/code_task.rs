use super::*;

pub struct SlackCodeTaskTool;

#[async_trait]
impl Tool for SlackCodeTaskTool {
    fn narrate(
        &self,
        call: &everruns_contracts::tool_types::ToolCall,
        phase: everruns_core::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: everruns_core::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        // Review content may be private; narration describes only the action.
        Some(everruns_core::tool_narration::narrate_labeled_action(
            &call.arguments,
            phase,
            locale,
            (
                "Publishing Slack Code task",
                "Published Slack Code task",
                "Could not publish Slack Code task",
            ),
            (
                "Публікую завдання Slack Code",
                "Опублікував завдання Slack Code",
                "Не вдалося опублікувати завдання Slack Code",
            ),
            &[],
        ))
    }

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
            let diff = match read_diff(context, &path).await {
                Ok(diff) => diff,
                Err(error) => return error,
            };
            let Some(object) = arguments.as_object_mut() else {
                return ToolExecutionResult::tool_error("Expected publication object");
            };
            object.remove("diff_path");
            arguments["diff"] = json!(diff);
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

/// Select the authoritative filesystem, never a stale VFS copy of sandbox data.
async fn read_diff(context: &ToolContext, path: &str) -> Result<String, ToolExecutionResult> {
    const MAX_BYTES: usize = 1_000_000;
    if let Some(workspace) = context
        .extensions
        .get::<crate::capabilities::ManagedSandboxWorkspace>()
    {
        return workspace.read_review_file(context, path, MAX_BYTES).await;
    }
    #[cfg(feature = "container-sandbox")]
    if let Some(workspace) = context
        .extensions
        .get::<crate::container_sandbox::ContainerWorkspaceFiles>()
    {
        return workspace.read_review_file(context, path, MAX_BYTES).await;
    }
    let Some(files) = &context.file_store else {
        return Err(ToolExecutionResult::tool_error(
            "Session filesystem unavailable",
        ));
    };
    let key = context.workspace_fs_key();
    match files.stat_file(key, path).await {
        Ok(Some(stat))
            if (0..=MAX_BYTES as i64).contains(&stat.size_bytes) && !stat.is_directory => {}
        Ok(Some(_)) => {
            return Err(ToolExecutionResult::tool_error(
                "Diff is too large or is a directory. Publish a review URL instead.",
            ));
        }
        Ok(None) => return Err(ToolExecutionResult::tool_error("Diff file not found")),
        Err(error) => return Err(ToolExecutionResult::internal_error_msg(error.to_string())),
    }
    let file = files
        .read_file(key, path)
        .await
        .map_err(|error| ToolExecutionResult::internal_error_msg(error.to_string()))?
        .ok_or_else(|| ToolExecutionResult::tool_error("Diff file not found"))?;
    if file.encoding != "text" || file.content.as_ref().is_none_or(|s| s.len() > MAX_BYTES) {
        return Err(ToolExecutionResult::tool_error(
            "Diff exceeds Slack's byte limit or is not UTF-8. Publish a review URL instead.",
        ));
    }
    Ok(file.content.unwrap_or_default())
}

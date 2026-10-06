use super::*;

const MAX_SEARCH_VISITED: usize = 10_000;

#[cfg(test)]
pub(super) fn schema_contains_workspace(value: &Value) -> bool {
    fn walk(value: &Value) -> bool {
        match value {
            Value::String(text) => text.contains(WORKSPACE_PREFIX),
            Value::Array(items) => items.iter().any(walk),
            Value::Object(fields) => fields.values().any(walk),
            _ => false,
        }
    }
    walk(value)
}

#[cfg(test)]
pub(super) fn filesystem_tool_schemas_with_presentation(
    presentation: &FilePathPresentation,
) -> Vec<(String, Value)> {
    SESSION_FILE_SYSTEM_TOOL_NAMES
        .iter()
        .filter_map(|name| {
            presentation
                .parameters_schema_for_tool(name)
                .map(|schema| ((*name).to_string(), schema))
        })
        .collect()
}

pub(super) fn glob_parameters_schema(presentation: &FilePathPresentation) -> Value {
    json!({
        "type": "object",
        "properties": {
            "pattern": {
                "type": "string",
                "description": format!("Glob relative to `{}`", presentation.root)
            },
            "limit": {"type": "integer", "minimum": 1, "maximum": 1000, "default": 200}
        },
        "required": ["pattern"],
        "additionalProperties": false
    })
}

pub(super) fn grep_parameters_schema(_presentation: &FilePathPresentation) -> Value {
    json!({
        "type": "object",
        "properties": {
            "pattern": {"type": "string", "description": "Rust regular expression"},
            "glob": {"type": "string", "description": "Optional file glob"},
            "offset": {"type": "integer", "minimum": 0, "default": 0},
            "limit": {"type": "integer", "minimum": 1, "maximum": 1000, "default": 200}
        },
        "required": ["pattern"],
        "additionalProperties": false
    })
}

/// Stable file discovery tool shared by every Environment target.
pub struct GlobTool;

#[async_trait]
impl Tool for GlobTool {
    fn narrate(
        &self,
        tool_call: &crate::filesystem::tool_types::ToolCall,
        phase: crate::filesystem::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        ctx: crate::filesystem::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        Some(crate::filesystem::tool_narration::narrate_list_directory(
            &tool_call.arguments,
            phase,
            locale,
            ctx,
        ))
    }

    fn name(&self) -> &str {
        "glob"
    }

    fn description(&self) -> &str {
        "List files under /workspace matching a glob."
    }

    fn parameters_schema(&self) -> Value {
        glob_parameters_schema(&FilePathPresentation::vfs())
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("glob requires session context")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let Some(pattern) = arguments.get("pattern").and_then(Value::as_str) else {
            return ToolExecutionResult::tool_error("Missing required parameter: pattern");
        };
        let pattern = pattern
            .strip_prefix("/workspace/")
            .unwrap_or(pattern.trim_start_matches('/'));
        let matcher = match globset::Glob::new(pattern) {
            Ok(glob) => glob.compile_matcher(),
            Err(error) => return ToolExecutionResult::tool_error(format!("Invalid glob: {error}")),
        };
        let limit = arguments
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(200)
            .clamp(1, 1_000) as usize;
        let Some(store) = context.file_store.as_ref() else {
            return ToolExecutionResult::tool_error("File system not available in this context");
        };
        let mut pending = vec!["/workspace".to_string()];
        let mut paths = Vec::new();
        let mut visited = 0;
        while let Some(directory) = pending.pop() {
            let entries = match store.list_directory(context.session_id, &directory).await {
                Ok(entries) => entries,
                Err(error) => return ToolExecutionResult::internal_error(error),
            };
            for entry in entries {
                visited += 1;
                if visited > MAX_SEARCH_VISITED {
                    paths.sort();
                    return ToolExecutionResult::success(
                        json!({"paths": paths, "truncated": true}),
                    );
                }
                if entry.is_directory {
                    pending.push(entry.path);
                    continue;
                }
                let relative = entry
                    .path
                    .strip_prefix("/workspace/")
                    .unwrap_or_else(|| entry.path.trim_start_matches('/'));
                if matcher.is_match(relative) {
                    paths.push(fs_display_path(store.as_ref(), &entry.path));
                    if paths.len() == limit {
                        paths.sort();
                        return ToolExecutionResult::success(
                            json!({"paths": paths, "truncated": true}),
                        );
                    }
                }
            }
        }
        paths.sort();
        ToolExecutionResult::success(json!({"paths": paths, "truncated": false}))
    }

    fn requires_context(&self) -> bool {
        true
    }

    fn required_context_services(&self) -> &'static [ToolContextService] {
        &[ToolContextService::SessionFileSystem]
    }
}

/// Stable grep name that adapts the legacy `grep_files` implementation.
pub struct GrepTool;

#[async_trait]
impl Tool for GrepTool {
    fn narrate(
        &self,
        tool_call: &crate::filesystem::tool_types::ToolCall,
        phase: crate::filesystem::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: crate::filesystem::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        Some(crate::filesystem::tool_narration::narrate_grep_files(
            &tool_call.arguments,
            phase,
            locale,
        ))
    }

    fn name(&self) -> &str {
        "grep"
    }

    fn description(&self) -> &str {
        "Search UTF-8 files under /workspace with a Rust regular expression."
    }

    fn parameters_schema(&self) -> Value {
        grep_parameters_schema(&FilePathPresentation::vfs())
    }

    fn hints(&self) -> ToolHints {
        GrepFilesTool.hints()
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("grep requires session context")
    }

    async fn execute_with_context(
        &self,
        mut arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        if let Some(glob) = arguments.get("glob").cloned()
            && let Some(object) = arguments.as_object_mut()
        {
            object.remove("glob");
            object.insert("path_pattern".to_string(), glob);
        }
        GrepFilesTool.execute_with_context(arguments, context).await
    }

    fn requires_context(&self) -> bool {
        true
    }

    fn required_context_services(&self) -> &'static [ToolContextService] {
        &[ToolContextService::SessionFileSystem]
    }
}

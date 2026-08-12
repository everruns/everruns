use async_trait::async_trait;
use everruns_core::session_task::{
    NewTaskMessage, SessionTask, SessionTaskUpdate, TaskMessageDirection, TaskMessagePart,
    task_result_path,
};
use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_core::traits::{SessionFileSystem, SessionStore, ToolContext};
use everruns_core::typed_id::{SessionId, WorkspaceId};
use serde_json::{Value, json};
use std::sync::Arc;

pub(crate) const RESULT_SCHEMA_SPEC_KEY: &str = "result_schema";
pub(crate) const MESSAGE_SCHEMA_SPEC_KEY: &str = "message_schema";

pub(crate) fn declared_result_schema(task: &SessionTask) -> Option<&Value> {
    task.spec
        .get(RESULT_SCHEMA_SPEC_KEY)
        .filter(|schema| schema.is_object())
}

pub(crate) fn declared_message_schema(task: &SessionTask) -> Option<&Value> {
    task.spec
        .get(MESSAGE_SCHEMA_SPEC_KEY)
        .filter(|schema| schema.is_object())
}

fn normalize_optional_schema(
    arguments: &Value,
    key: &str,
) -> Result<Option<Value>, ToolExecutionResult> {
    let Some(schema) = arguments.get(key).filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    if !schema.is_object() {
        return Err(ToolExecutionResult::tool_error(format!(
            "{key} must be a JSON Schema object when provided."
        )));
    }
    Ok(Some(schema.clone()))
}

pub(crate) fn normalize_result_schema(
    arguments: &Value,
) -> Result<Option<Value>, ToolExecutionResult> {
    normalize_optional_schema(arguments, RESULT_SCHEMA_SPEC_KEY)
}

pub(crate) fn normalize_message_schema(
    arguments: &Value,
) -> Result<Option<Value>, ToolExecutionResult> {
    normalize_optional_schema(arguments, MESSAGE_SCHEMA_SPEC_KEY)
}

pub(crate) fn schema_validation_errors(schema: &Value, value: &Value) -> Vec<String> {
    let validator = match jsonschema::draft202012::options()
        .should_validate_formats(true)
        .build(schema)
    {
        Ok(validator) => validator,
        Err(error) => return vec![format!("result schema is invalid: {error}")],
    };

    validator
        .iter_errors(value)
        .map(|error| {
            let path = error.instance_path().to_string();
            let path = if path.is_empty() {
                "$".to_string()
            } else {
                path
            };
            format!("{path} {error}")
        })
        .collect()
}

const MAX_TASK_SUMMARY_CHARS: usize = 2_048;

pub(crate) fn truncate_summary(text: &str) -> String {
    let mut chars = text.chars();
    let truncated: String = chars.by_ref().take(MAX_TASK_SUMMARY_CHARS).collect();
    if chars.next().is_some() {
        format!("{truncated}\n[truncated]")
    } else {
        truncated
    }
}

pub(crate) async fn task_for_child_session(
    child_session_id: SessionId,
    session_store: &dyn SessionStore,
    task_registry: &dyn everruns_core::session_task::SessionTaskRegistry,
) -> everruns_core::error::Result<Option<(SessionTask, WorkspaceId)>> {
    let Some(child) = session_store.get_session(child_session_id).await? else {
        return Ok(None);
    };
    let Some(parent_session_id) = child.parent_session_id.or(child.forked_from_session_id) else {
        return Ok(None);
    };
    let Some(parent) = session_store.get_session(parent_session_id).await? else {
        return Ok(None);
    };
    let task = task_registry
        .list(parent_session_id, None)
        .await?
        .into_iter()
        .find(|task| task.links.child_session_id == Some(child_session_id));
    Ok(task.map(|task| (task, parent.workspace_id)))
}

pub struct ReportResultTool {
    parent_session_id: SessionId,
    parent_workspace_id: WorkspaceId,
    child_session_id: SessionId,
    task_id: String,
    result_schema: Value,
    file_store: Option<Arc<dyn SessionFileSystem>>,
}

impl ReportResultTool {
    pub fn new(
        parent_session_id: SessionId,
        parent_workspace_id: WorkspaceId,
        child_session_id: SessionId,
        task_id: String,
        result_schema: Value,
    ) -> Self {
        Self {
            parent_session_id,
            parent_workspace_id,
            child_session_id,
            task_id,
            result_schema,
            file_store: None,
        }
    }

    pub fn with_file_store(mut self, file_store: Arc<dyn SessionFileSystem>) -> Self {
        self.file_store = Some(file_store);
        self
    }
}

#[async_trait]
impl Tool for ReportResultTool {
    fn narrate(
        &self,
        tool_call: &everruns_core::tool_types::ToolCall,
        phase: everruns_core::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: everruns_core::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        everruns_core::tool_narration::narrate_delegation_result(&tool_call.name, phase, locale)
    }

    fn name(&self) -> &str {
        "report_result"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Report Result")
    }

    fn description(&self) -> &str {
        "Submit the final structured result for this delegated task. The call arguments must match the declared result schema."
    }

    fn parameters_schema(&self) -> Value {
        self.result_schema.clone()
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "report_result requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let errors = schema_validation_errors(&self.result_schema, &arguments);
        if !errors.is_empty() {
            return ToolExecutionResult::tool_error(format!(
                "report_result arguments do not match result_schema: {}",
                errors.join("; ")
            ));
        }
        let Some(registry) = context.session_task_registry.as_ref() else {
            return ToolExecutionResult::tool_error(
                "report_result requires session_task_registry context",
            );
        };
        let Some(file_store) = self.file_store.as_ref().or(context.file_store.as_ref()) else {
            return ToolExecutionResult::tool_error("report_result requires file_store context");
        };

        if context.session_id != self.child_session_id {
            return ToolExecutionResult::tool_error(
                "report_result can only be called from the linked child session",
            );
        }
        let task = match registry.get(self.parent_session_id, &self.task_id).await {
            Ok(Some(task)) => task,
            Ok(None) => return ToolExecutionResult::tool_error("report_result task was not found"),
            Err(error) => return ToolExecutionResult::internal_error(error),
        };
        if task.links.child_session_id != Some(self.child_session_id) {
            return ToolExecutionResult::tool_error(
                "report_result child session is no longer linked to this task",
            );
        }
        if task.state.is_terminal() {
            return ToolExecutionResult::tool_error(
                "report_result is closed because the subagent task is terminal",
            );
        }
        if task.result_path.is_some() {
            return ToolExecutionResult::tool_error(
                "report_result was already recorded for this subagent task",
            );
        }

        let path = task_result_path(&self.task_id);
        let content = match serde_json::to_string_pretty(&arguments) {
            Ok(content) => content,
            Err(error) => return ToolExecutionResult::internal_error(error),
        };
        let parent_workspace_key = SessionId::from_uuid(self.parent_workspace_id.uuid());
        if let Err(error) = file_store
            .write_file(parent_workspace_key, &path, &content, "utf-8")
            .await
        {
            return ToolExecutionResult::internal_error(error);
        }
        if let Err(error) = registry
            .update(
                self.parent_session_id,
                &self.task_id,
                SessionTaskUpdate {
                    // Fence result writes to the active attempt and repeat the
                    // observed non-terminal state so a task that finalized after
                    // our read rejects this stale content mutation too.
                    expected_attempt: Some(task.attempt),
                    state: Some(task.state),
                    result_path: Some(path.clone()),
                    summary: Some(truncate_summary(&content)),
                    ..Default::default()
                },
            )
            .await
        {
            return ToolExecutionResult::internal_error(error);
        }
        ToolExecutionResult::success(json!({
            "status": "recorded",
            "task_id": self.task_id,
            "result_path": path,
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

pub struct ReportTaskProgressTool {
    parent_session_id: SessionId,
    task_id: String,
    task_attempt: i32,
    message_schema: Value,
}

impl ReportTaskProgressTool {
    pub fn new(
        parent_session_id: SessionId,
        task_id: String,
        task_attempt: i32,
        message_schema: Value,
    ) -> Self {
        Self {
            parent_session_id,
            task_id,
            task_attempt,
            message_schema,
        }
    }
}

#[async_trait]
impl Tool for ReportTaskProgressTool {
    fn narrate(
        &self,
        tool_call: &everruns_core::tool_types::ToolCall,
        phase: everruns_core::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: everruns_core::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        everruns_core::tool_narration::narrate_delegation_result(&tool_call.name, phase, locale)
    }

    fn name(&self) -> &str {
        "report_task_progress"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Report Task Progress")
    }

    fn description(&self) -> &str {
        "Post a structured progress message for this delegated task. The call arguments must match the declared message schema."
    }

    fn parameters_schema(&self) -> Value {
        self.message_schema.clone()
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "report_task_progress requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let errors = schema_validation_errors(&self.message_schema, &arguments);
        if !errors.is_empty() {
            return ToolExecutionResult::tool_error(format!(
                "report_task_progress arguments do not match message_schema: {}",
                errors.join("; ")
            ));
        }
        let Some(registry) = context.session_task_registry.as_ref() else {
            return ToolExecutionResult::tool_error(
                "report_task_progress requires session_task_registry context",
            );
        };
        let stored = match registry
            .record_message(
                self.parent_session_id,
                &self.task_id,
                NewTaskMessage {
                    direction: TaskMessageDirection::Outbound,
                    content: vec![TaskMessagePart::Data {
                        data: arguments.clone(),
                    }],
                    in_reply_to: None,
                    expected_attempt: Some(self.task_attempt),
                },
            )
            .await
        {
            Ok(stored) => stored,
            Err(error) => return ToolExecutionResult::internal_error(error),
        };
        ToolExecutionResult::success(json!({
            "status": "posted",
            "task_id": self.task_id,
            "message_id": stored.id,
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

pub async fn report_result_tool_for_child_session(
    child_session_id: SessionId,
    session_store: &dyn SessionStore,
    task_registry: &dyn everruns_core::session_task::SessionTaskRegistry,
) -> everruns_core::error::Result<Option<ReportResultTool>> {
    let Some((task, parent_workspace_id)) =
        task_for_child_session(child_session_id, session_store, task_registry).await?
    else {
        return Ok(None);
    };
    let Some(schema) = declared_result_schema(&task).cloned() else {
        return Ok(None);
    };
    Ok(Some(ReportResultTool::new(
        task.session_id,
        parent_workspace_id,
        child_session_id,
        task.id,
        schema,
    )))
}

pub async fn report_task_progress_tool_for_child_session(
    child_session_id: SessionId,
    session_store: &dyn SessionStore,
    task_registry: &dyn everruns_core::session_task::SessionTaskRegistry,
) -> everruns_core::error::Result<Option<ReportTaskProgressTool>> {
    let Some((task, _)) =
        task_for_child_session(child_session_id, session_store, task_registry).await?
    else {
        return Ok(None);
    };
    if task.state.is_terminal() {
        return Ok(None);
    }
    let Some(schema) = declared_message_schema(&task).cloned() else {
        return Ok(None);
    };
    Ok(Some(ReportTaskProgressTool::new(
        task.session_id,
        task.id,
        task.attempt,
        schema,
    )))
}

pub(crate) async fn result_value_for_task(
    context: &ToolContext,
    task_id: Option<&str>,
) -> Option<Value> {
    let task_id = task_id?;
    let registry = context.session_task_registry.as_ref()?;
    let task = registry
        .get(context.session_id, task_id)
        .await
        .ok()
        .flatten()?;
    declared_result_schema(&task)?;
    let result_path = task.result_path.as_deref()?;
    let file_store = context.file_store.as_ref()?;
    let file = file_store
        .read_file(context.workspace_fs_key(), result_path)
        .await
        .ok()
        .flatten()?;
    serde_json::from_str(file.content.as_deref()?).ok()
}

pub(crate) async fn required_result_is_missing(
    context: &ToolContext,
    task_id: Option<&str>,
) -> bool {
    let Some(task_id) = task_id else {
        return false;
    };
    let Some(registry) = context.session_task_registry.as_ref() else {
        return false;
    };
    registry
        .get(context.session_id, task_id)
        .await
        .ok()
        .flatten()
        .is_some_and(|task| declared_result_schema(&task).is_some() && task.result_path.is_none())
}

#[cfg(feature = "a2a")]
pub(crate) async fn write_task_result_value(
    context: &ToolContext,
    task_id: &str,
    value: &Value,
) -> everruns_core::error::Result<Option<String>> {
    let Some(file_store) = context.file_store.as_ref() else {
        return Ok(None);
    };
    let path = task_result_path(task_id);
    let content = serde_json::to_string_pretty(value).map_err(|error| {
        everruns_core::error::AgentLoopError::store(format!(
            "failed to serialize task result: {error}"
        ))
    })?;
    file_store
        .write_file(context.workspace_fs_key(), &path, &content, "utf-8")
        .await?;
    Ok(Some(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_validator_reports_required_type_and_extra_property_errors() {
        let schema = json!({
            "type": "object",
            "properties": {
                "answer": {"type": "string"},
                "count": {"type": "integer"}
            },
            "required": ["answer", "count"],
            "additionalProperties": false
        });
        let errors =
            schema_validation_errors(&schema, &json!({"count": "not-an-integer", "extra": true}));
        assert!(
            errors
                .iter()
                .any(|error| error.contains("answer") && error.contains("required"))
        );
        assert!(
            errors
                .iter()
                .any(|error| error.contains("count") && error.contains("integer"))
        );
        assert!(errors.iter().any(|error| error.contains("extra")
            && (error.contains("additional") || error.contains("not allowed"))));
    }

    #[test]
    fn shared_validator_enforces_full_json_schema_constraints() {
        let schema = json!({
            "type": "object",
            "properties": {
                "echo": {"type": "string", "pattern": "^[0-9]+$", "minLength": 2},
                "score": {"type": "integer", "minimum": 0},
                "tags": {
                    "type": "array",
                    "minItems": 2,
                    "items": {"type": "string"}
                },
                "choice": {
                    "oneOf": [
                        {"const": "alpha"},
                        {"const": "beta"}
                    ]
                }
            },
            "required": ["echo", "score", "tags", "choice"],
            "additionalProperties": false
        });

        let errors = schema_validation_errors(
            &schema,
            &json!({
                "echo": "x",
                "score": -5,
                "tags": ["solo"],
                "choice": "gamma"
            }),
        );

        assert!(errors.iter().any(|error| error.contains("echo")
            && (error.contains("pattern") || error.contains("^[0-9]+$"))));
        assert!(
            errors
                .iter()
                .any(|error| error.contains("score") && error.contains("0"))
        );
        assert!(
            errors
                .iter()
                .any(|error| error.contains("tags") && error.contains("2"))
        );
        assert!(
            errors
                .iter()
                .any(|error| error.contains("choice") && error.contains("oneOf"))
        );
    }

    #[test]
    fn shared_schema_normalization_rejects_non_objects() {
        let ToolExecutionResult::ToolError(error) =
            normalize_result_schema(&json!({"result_schema": "object"})).unwrap_err()
        else {
            panic!("expected tool error");
        };
        assert!(error.contains("result_schema must be a JSON Schema object"));
    }
}

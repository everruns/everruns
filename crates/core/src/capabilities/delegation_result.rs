use crate::session_task::{
    NewTaskMessage, SessionTask, SessionTaskUpdate, TaskMessageDirection, TaskMessagePart,
    task_result_path,
};
use crate::tools::{Tool, ToolExecutionResult};
use crate::traits::{SessionFileSystem, SessionStore, ToolContext};
use crate::typed_id::{SessionId, WorkspaceId};
use async_trait::async_trait;
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

fn json_schema_type_matches(expected: &str, value: &Value) -> bool {
    match expected {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "boolean" => value.is_boolean(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "number" => value.is_number(),
        "null" => value.is_null(),
        _ => true,
    }
}

fn validate_against_schema(schema: &Value, value: &Value, path: &str, errors: &mut Vec<String>) {
    if let Some(enum_values) = schema.get("enum").and_then(Value::as_array)
        && !enum_values.iter().any(|candidate| candidate == value)
    {
        errors.push(format!("{path} is not one of the allowed enum values"));
    }
    if let Some(const_value) = schema.get("const")
        && const_value != value
    {
        errors.push(format!("{path} does not match the required const value"));
    }
    if let Some(type_value) = schema.get("type") {
        let matches = match type_value {
            Value::String(expected) => json_schema_type_matches(expected, value),
            Value::Array(types) => types
                .iter()
                .filter_map(Value::as_str)
                .any(|expected| json_schema_type_matches(expected, value)),
            _ => true,
        };
        if !matches {
            errors.push(format!("{path} has the wrong JSON type"));
            return;
        }
    }
    if let (Some(object), Some(properties)) = (
        value.as_object(),
        schema.get("properties").and_then(Value::as_object),
    ) {
        if let Some(required) = schema.get("required").and_then(Value::as_array) {
            for key in required.iter().filter_map(Value::as_str) {
                if !object.contains_key(key) {
                    errors.push(format!("{path}.{key} is required"));
                }
            }
        }
        if schema.get("additionalProperties") == Some(&Value::Bool(false)) {
            for key in object.keys() {
                if !properties.contains_key(key) {
                    errors.push(format!("{path}.{key} is not allowed"));
                }
            }
        }
        for (key, property_schema) in properties {
            if let Some(property_value) = object.get(key) {
                validate_against_schema(
                    property_schema,
                    property_value,
                    &format!("{path}.{key}"),
                    errors,
                );
            }
        }
    }
    if let (Some(array), Some(item_schema)) = (value.as_array(), schema.get("items")) {
        for (index, item) in array.iter().enumerate() {
            validate_against_schema(item_schema, item, &format!("{path}[{index}]"), errors);
        }
    }
}

pub(crate) fn schema_validation_errors(schema: &Value, value: &Value) -> Vec<String> {
    let mut errors = Vec::new();
    validate_against_schema(schema, value, "$", &mut errors);
    errors
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
    task_registry: &dyn crate::session_task::SessionTaskRegistry,
) -> crate::error::Result<Option<(SessionTask, WorkspaceId)>> {
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
    task_id: String,
    result_schema: Value,
    file_store: Option<Arc<dyn SessionFileSystem>>,
}

impl ReportResultTool {
    pub fn new(
        parent_session_id: SessionId,
        parent_workspace_id: WorkspaceId,
        task_id: String,
        result_schema: Value,
    ) -> Self {
        Self {
            parent_session_id,
            parent_workspace_id,
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
    message_schema: Value,
}

impl ReportTaskProgressTool {
    pub fn new(parent_session_id: SessionId, task_id: String, message_schema: Value) -> Self {
        Self {
            parent_session_id,
            task_id,
            message_schema,
        }
    }
}

#[async_trait]
impl Tool for ReportTaskProgressTool {
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
                    expected_attempt: None,
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
    task_registry: &dyn crate::session_task::SessionTaskRegistry,
) -> crate::error::Result<Option<ReportResultTool>> {
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
        task.id,
        schema,
    )))
}

pub async fn report_task_progress_tool_for_child_session(
    child_session_id: SessionId,
    session_store: &dyn SessionStore,
    task_registry: &dyn crate::session_task::SessionTaskRegistry,
) -> crate::error::Result<Option<ReportTaskProgressTool>> {
    let Some((task, _)) =
        task_for_child_session(child_session_id, session_store, task_registry).await?
    else {
        return Ok(None);
    };
    let Some(schema) = declared_message_schema(&task).cloned() else {
        return Ok(None);
    };
    Ok(Some(ReportTaskProgressTool::new(
        task.session_id,
        task.id,
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

pub(crate) async fn write_task_result_value(
    context: &ToolContext,
    task_id: &str,
    value: &Value,
) -> crate::error::Result<Option<String>> {
    let Some(file_store) = context.file_store.as_ref() else {
        return Ok(None);
    };
    let path = task_result_path(task_id);
    let content = serde_json::to_string_pretty(value).map_err(|error| {
        crate::error::AgentLoopError::store(format!("failed to serialize task result: {error}"))
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
        assert!(errors.iter().any(|error| error == "$.answer is required"));
        assert!(
            errors
                .iter()
                .any(|error| error == "$.count has the wrong JSON type")
        );
        assert!(errors.iter().any(|error| error == "$.extra is not allowed"));
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

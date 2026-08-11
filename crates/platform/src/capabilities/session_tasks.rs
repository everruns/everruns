// Session Tasks Capability
//
// Generic agent-facing tools over the session task registry
// (knowledge/runtime-resources/session-tasks.md): list_tasks / get_task / message_task /
// cancel_task / wait_task. Spawning stays with each creation surface
// (spawn_agent, spawn_background) — every spawn creates a task and returns its
// task_id; these tools provide the uniform query/messaging/cancel/wait plane.
//
// Decision: tools declare `SessionTaskRegistry` as a hard context-service
// requirement, so production runtime assembly rejects the capability before
// model exposure when the host lacks that backend. Direct/test execution still
// returns a tool error instead of panicking.

use super::{Capability, CapabilityLocalization, CapabilityStatus};
use async_trait::async_trait;
use everruns_core::session_task::{
    NewTaskMessage, SessionTask, SessionTaskFilter, SessionTaskRegistry, SessionTaskState,
    TaskMessage, find_task_executor,
};
use everruns_core::tool_types::ToolHints;
use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_core::traits::{ToolContext, ToolContextService};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::{Instant, sleep};

pub const SESSION_TASKS_CAPABILITY_ID: &str = "session_tasks";

const DEFAULT_WAIT_TIMEOUT_SECS: u64 = 300;
const WAIT_POLL_INTERVAL: Duration = Duration::from_secs(1);
/// Reconcile polled task kinds (e.g. remote A2A tasks) every Nth wait poll.
const WAIT_RECONCILE_EVERY: u64 = 5;
/// Recent-thread size returned by get_task.
const GET_TASK_MESSAGE_LIMIT: u32 = 20;

/// Session tasks capability — uniform tracking of background work.
pub struct SessionTasksCapability;

impl Capability for SessionTasksCapability {
    fn id(&self) -> &str {
        SESSION_TASKS_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Session Tasks"
    }

    fn description(&self) -> &str {
        "Track, message, cancel, and wait on the session's background tasks (subagents, external agents, background tools)."
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "Завдання сесії",
            "Відстежуйте фонові завдання сесії (субагенти, зовнішні агенти, фонові інструменти), надсилайте їм повідомлення, скасовуйте їх та очікуйте на їхнє завершення.",
        )]
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("list-checks")
    }

    fn category(&self) -> Option<&str> {
        Some("Orchestration")
    }

    fn features(&self) -> Vec<&'static str> {
        vec!["session_tasks"]
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(SESSION_TASKS_SYSTEM_PROMPT)
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![
            Box::new(ListTasksTool),
            Box::new(GetTaskTool),
            Box::new(MessageTaskTool),
            Box::new(CancelTaskTool),
            Box::new(WaitTaskTool),
        ]
    }
}

const SESSION_TASKS_SYSTEM_PROMPT: &str = "Every spawned background work item (subagent, external agent, background tool) is a task with a task_id. Use list_tasks/get_task to check status instead of re-spawning. Answer a task in awaiting_input with message_task (set in_reply_to to the pending input request id). Use wait_task only when you have nothing else to do until the task finishes.";

// =============================================================================
// Helpers
// =============================================================================

fn require_task_registry(
    context: &ToolContext,
) -> Result<&Arc<dyn SessionTaskRegistry>, ToolExecutionResult> {
    context.session_task_registry.as_ref().ok_or_else(|| {
        ToolExecutionResult::tool_error(
            "Session task tools require session_task_registry context (not available in this environment)",
        )
    })
}

use super::util::require_str_trimmed as require_str;

async fn load_task(
    context: &ToolContext,
    task_id: &str,
) -> Result<SessionTask, ToolExecutionResult> {
    let registry = require_task_registry(context)?;
    registry
        .get(context.session_id, task_id)
        .await
        .map_err(ToolExecutionResult::internal_error)?
        .ok_or_else(|| ToolExecutionResult::tool_error(format!("No task found with id: {task_id}")))
}

/// Compact list entry: enough to decide whether to drill in with get_task.
fn compact_task_json(task: &SessionTask) -> Value {
    json!({
        "id": task.id,
        "kind": task.kind,
        "display_name": task.display_name,
        "state": task.state,
        "state_detail": task.state_detail,
        "progress": task.progress,
        "summary": task.summary,
        "created_at": task.created_at.to_rfc3339(),
        "finished_at": task.finished_at.map(|t| t.to_rfc3339()),
    })
}

fn message_json(message: &TaskMessage) -> Value {
    serde_json::to_value(message).unwrap_or_else(|_| json!({}))
}

fn full_task_json(task: &SessionTask) -> Value {
    serde_json::to_value(task).unwrap_or_else(|_| json!({}))
}

// =============================================================================
// Tool: list_tasks
// =============================================================================

pub struct ListTasksTool;

#[async_trait]
impl Tool for ListTasksTool {
    fn narrate(
        &self,
        tool_call: &everruns_core::tool_types::ToolCall,
        phase: everruns_core::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: everruns_core::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        everruns_core::tool_narration::narrate_session_task(
            self.name(),
            &tool_call.arguments,
            phase,
            locale,
        )
    }

    fn name(&self) -> &str {
        "list_tasks"
    }

    fn display_name(&self) -> Option<&str> {
        Some("List Tasks")
    }

    fn description(&self) -> &str {
        "List this session's background tasks (subagents, external agents, background tools) with state, progress, and summary."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "state": {
                    "type": "string",
                    "enum": ["queued", "running", "awaiting_input", "succeeded", "failed", "canceled"],
                    "description": "Filter by lifecycle state."
                },
                "kind": {
                    "type": "string",
                    "description": "Filter by task kind (e.g. 'subagent', 'external_agent', 'background_tool')."
                }
            },
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("list_tasks requires session context.")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        list_tasks_impl(arguments, context)
            .await
            .unwrap_or_else(|e| e)
    }

    fn requires_context(&self) -> bool {
        true
    }

    fn required_context_services(&self) -> &'static [ToolContextService] {
        &[ToolContextService::SessionTaskRegistry]
    }
}

async fn list_tasks_impl(
    arguments: Value,
    context: &ToolContext,
) -> Result<ToolExecutionResult, ToolExecutionResult> {
    let registry = require_task_registry(context)?;
    let state = match arguments.get("state").and_then(Value::as_str) {
        Some(raw) => match SessionTaskState::parse(raw) {
            Some(state) => Some(state),
            None => {
                return Ok(ToolExecutionResult::tool_error(format!(
                    "Unknown state filter \"{raw}\". Valid states: queued, running, \
                     awaiting_input, succeeded, failed, canceled."
                )));
            }
        },
        None => None,
    };
    let filter = SessionTaskFilter {
        kind: arguments
            .get("kind")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string),
        state,
    };
    let tasks = registry
        .list(context.session_id, Some(&filter))
        .await
        .map_err(ToolExecutionResult::internal_error)?;
    let entries = tasks.iter().map(compact_task_json).collect::<Vec<_>>();
    Ok(ToolExecutionResult::success(json!({
        "tasks": entries,
        "count": entries.len(),
    })))
}

// =============================================================================
// Tool: get_task
// =============================================================================

pub struct GetTaskTool;

#[async_trait]
impl Tool for GetTaskTool {
    fn narrate(
        &self,
        tool_call: &everruns_core::tool_types::ToolCall,
        phase: everruns_core::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: everruns_core::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        everruns_core::tool_narration::narrate_session_task(
            self.name(),
            &tool_call.arguments,
            phase,
            locale,
        )
    }

    fn name(&self) -> &str {
        "get_task"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Get Task")
    }

    fn description(&self) -> &str {
        "Get a task's full snapshot (state, progress, input request, result path, error) plus its recent message thread."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "Task ID (task_*)."
                }
            },
            "required": ["task_id"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("get_task requires session context.")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        get_task_impl(arguments, context)
            .await
            .unwrap_or_else(|e| e)
    }

    fn requires_context(&self) -> bool {
        true
    }

    fn required_context_services(&self) -> &'static [ToolContextService] {
        &[ToolContextService::SessionTaskRegistry]
    }
}

async fn get_task_impl(
    arguments: Value,
    context: &ToolContext,
) -> Result<ToolExecutionResult, ToolExecutionResult> {
    let task_id = require_str(&arguments, "task_id")?;
    let task = load_task(context, task_id).await?;
    let registry = require_task_registry(context)?;
    let messages = registry
        .list_messages(
            context.session_id,
            task_id,
            Some(GET_TASK_MESSAGE_LIMIT),
            None,
        )
        .await
        .unwrap_or_default();
    Ok(ToolExecutionResult::success(json!({
        "task": full_task_json(&task),
        "messages": messages.iter().map(message_json).collect::<Vec<_>>(),
    })))
}

// =============================================================================
// Tool: message_task
// =============================================================================

pub struct MessageTaskTool;

#[async_trait]
impl Tool for MessageTaskTool {
    fn narrate(
        &self,
        tool_call: &everruns_core::tool_types::ToolCall,
        phase: everruns_core::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: everruns_core::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        everruns_core::tool_narration::narrate_session_task(
            self.name(),
            &tool_call.arguments,
            phase,
            locale,
        )
    }

    fn name(&self) -> &str {
        "message_task"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Message Task")
    }

    fn description(&self) -> &str {
        "Send an inbound message to a task. To answer a pending input request, set in_reply_to to the input request id."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "Task ID (task_*)."
                },
                "message": {
                    "type": "string",
                    "description": "Message to deliver to the task."
                },
                "in_reply_to": {
                    "type": "string",
                    "description": "ID of the pending input request this message answers."
                }
            },
            "required": ["task_id", "message"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default().with_long_running(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("message_task requires session context.")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        message_task_impl(arguments, context)
            .await
            .unwrap_or_else(|e| e)
    }

    fn requires_context(&self) -> bool {
        true
    }

    fn required_context_services(&self) -> &'static [ToolContextService] {
        &[ToolContextService::SessionTaskRegistry]
    }
}

async fn message_task_impl(
    arguments: Value,
    context: &ToolContext,
) -> Result<ToolExecutionResult, ToolExecutionResult> {
    let task_id = require_str(&arguments, "task_id")?.to_string();
    let message = require_str(&arguments, "message")?.to_string();
    let in_reply_to = arguments
        .get("in_reply_to")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string);

    let task = load_task(context, &task_id).await?;
    let registry = require_task_registry(context)?;

    let mut new_message = NewTaskMessage::inbound_text(message);
    new_message.in_reply_to = in_reply_to;
    let recorded = registry
        .record_message(context.session_id, &task_id, new_message)
        .await
        .map_err(ToolExecutionResult::internal_error)?;

    // Best-effort delivery: the message is durably recorded either way;
    // report delivery outcome instead of failing the whole call.
    let delivery = match find_task_executor(&task.kind) {
        Some(executor) => {
            // Re-read: recording an input-request answer returns the task
            // to running before the executor sees it.
            let current = registry
                .get(context.session_id, &task_id)
                .await
                .ok()
                .flatten()
                .unwrap_or(task);
            match executor.deliver(&current, &recorded, context).await {
                Ok(()) => "delivered".to_string(),
                Err(e) => format!("failed: {e}"),
            }
        }
        None => format!(
            "failed: no executor registered for task kind '{}'",
            task.kind
        ),
    };

    Ok(ToolExecutionResult::success(json!({
        "task_id": task_id,
        "message_id": recorded.id,
        "recorded": true,
        "delivery": delivery,
    })))
}

// =============================================================================
// Tool: cancel_task
// =============================================================================

pub struct CancelTaskTool;

#[async_trait]
impl Tool for CancelTaskTool {
    fn narrate(
        &self,
        tool_call: &everruns_core::tool_types::ToolCall,
        phase: everruns_core::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: everruns_core::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        everruns_core::tool_narration::narrate_session_task(
            self.name(),
            &tool_call.arguments,
            phase,
            locale,
        )
    }

    fn name(&self) -> &str {
        "cancel_task"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Cancel Task")
    }

    fn description(&self) -> &str {
        "Request cooperative cancellation of a task. The task winds down and may still end succeeded or failed. For a detached `session` task this also cancels the peer session (not just the tracking chip)."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "Task ID (task_*)."
                }
            },
            "required": ["task_id"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default().with_idempotent(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("cancel_task requires session context.")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        cancel_task_impl(arguments, context)
            .await
            .unwrap_or_else(|e| e)
    }

    fn requires_context(&self) -> bool {
        true
    }

    fn required_context_services(&self) -> &'static [ToolContextService] {
        &[ToolContextService::SessionTaskRegistry]
    }
}

async fn cancel_task_impl(
    arguments: Value,
    context: &ToolContext,
) -> Result<ToolExecutionResult, ToolExecutionResult> {
    let task_id = require_str(&arguments, "task_id")?.to_string();
    let registry = require_task_registry(context)?;
    let task = match registry.request_cancel(context.session_id, &task_id).await {
        Ok(Some(task)) => task,
        Ok(None) => {
            return Ok(ToolExecutionResult::tool_error(format!(
                "No task found with id: {task_id}"
            )));
        }
        Err(e) => return Err(ToolExecutionResult::internal_error(e)),
    };

    // Best-effort executor wind-down; the intent is recorded regardless.
    let executor_result = if task.state.is_terminal() {
        "task already terminal".to_string()
    } else {
        match find_task_executor(&task.kind) {
            Some(executor) => match executor.cancel(&task, context).await {
                Ok(()) => "cancellation requested".to_string(),
                Err(e) => format!("failed: {e}"),
            },
            None => format!(
                "no executor registered for task kind '{}'; cancel intent recorded",
                task.kind
            ),
        }
    };

    Ok(ToolExecutionResult::success(json!({
        "task_id": task_id,
        "state": task.state,
        "cancel_requested": true,
        "executor": executor_result,
    })))
}

// =============================================================================
// Tool: wait_task
// =============================================================================

pub struct WaitTaskTool;

#[async_trait]
impl Tool for WaitTaskTool {
    fn narrate(
        &self,
        tool_call: &everruns_core::tool_types::ToolCall,
        phase: everruns_core::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: everruns_core::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        everruns_core::tool_narration::narrate_session_task(
            self.name(),
            &tool_call.arguments,
            phase,
            locale,
        )
    }

    fn name(&self) -> &str {
        "wait_task"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Wait Task")
    }

    fn description(&self) -> &str {
        "Wait until a task reaches a terminal state or asks for input. Returns the latest task snapshot."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "Task ID (task_*)."
                },
                "timeout_seconds": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 86400,
                    "default": 300,
                    "description": "Maximum seconds to wait before returning the current snapshot."
                }
            },
            "required": ["task_id"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default().with_long_running(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("wait_task requires session context.")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        wait_task_impl(arguments, context)
            .await
            .unwrap_or_else(|e| e)
    }

    fn requires_context(&self) -> bool {
        true
    }

    fn required_context_services(&self) -> &'static [ToolContextService] {
        &[ToolContextService::SessionTaskRegistry]
    }
}

async fn wait_task_impl(
    arguments: Value,
    context: &ToolContext,
) -> Result<ToolExecutionResult, ToolExecutionResult> {
    let task_id = require_str(&arguments, "task_id")?.to_string();
    let timeout_secs = arguments
        .get("timeout_seconds")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_WAIT_TIMEOUT_SECS);
    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    let mut polls: u64 = 0;

    loop {
        let task = load_task(context, &task_id).await?;
        if task.state.is_terminal() || task.state == SessionTaskState::AwaitingInput {
            return Ok(ToolExecutionResult::success(json!({
                "task": full_task_json(&task),
                "timed_out": false,
            })));
        }
        if Instant::now() >= deadline {
            return Ok(ToolExecutionResult::success(json!({
                "task": full_task_json(&task),
                "timed_out": true,
                "message": format!("Task {task_id} still {} after {timeout_secs}s", task.state),
            })));
        }
        polls += 1;
        // Refresh polled kinds (e.g. remote A2A tasks) periodically so the
        // registry snapshot converges even when nothing pushes updates.
        if polls.is_multiple_of(WAIT_RECONCILE_EVERY)
            && let Some(executor) = find_task_executor(&task.kind)
        {
            let _ = executor.reconcile(&task, context).await;
        }
        sleep(WAIT_POLL_INTERVAL).await;
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use chrono::Utc;
    use everruns_core::session_task::{
        CreateSessionTask, SessionTaskUpdate, TaskError, TaskExecutor, TaskExecutorPlugin,
        TaskInputRequest, TaskLinks, TaskMessageDirection, TaskMessagePart, TaskWakePolicy,
        apply_task_update, generate_task_message_id, new_session_task,
    };
    use everruns_core::typed_id::SessionId;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// In-memory `SessionTaskRegistry` test double. Updates route through
    /// `apply_task_update` so lifecycle invariants match real backends.
    #[derive(Default)]
    pub(crate) struct InMemorySessionTaskRegistry {
        tasks: Mutex<HashMap<String, SessionTask>>,
        messages: Mutex<HashMap<String, Vec<TaskMessage>>>,
    }

    #[async_trait]
    impl SessionTaskRegistry for InMemorySessionTaskRegistry {
        async fn create(
            &self,
            input: CreateSessionTask,
        ) -> everruns_core::error::Result<SessionTask> {
            let mut tasks = self.tasks.lock().unwrap();
            if let Some(id) = &input.id
                && let Some(existing) = tasks.get(id)
            {
                return Ok(existing.clone());
            }
            let task = new_session_task(input, Utc::now());
            tasks.insert(task.id.clone(), task.clone());
            Ok(task)
        }

        async fn update(
            &self,
            _session_id: SessionId,
            task_id: &str,
            update: SessionTaskUpdate,
        ) -> everruns_core::error::Result<Option<SessionTask>> {
            let mut tasks = self.tasks.lock().unwrap();
            let Some(task) = tasks.get_mut(task_id) else {
                return Ok(None);
            };
            apply_task_update(task, update, Utc::now());
            Ok(Some(task.clone()))
        }

        async fn get(
            &self,
            _session_id: SessionId,
            task_id: &str,
        ) -> everruns_core::error::Result<Option<SessionTask>> {
            Ok(self.tasks.lock().unwrap().get(task_id).cloned())
        }

        async fn list(
            &self,
            session_id: SessionId,
            filter: Option<&SessionTaskFilter>,
        ) -> everruns_core::error::Result<Vec<SessionTask>> {
            let tasks = self.tasks.lock().unwrap();
            Ok(tasks
                .values()
                .filter(|task| {
                    task.session_id == session_id
                        && filter.is_none_or(|f| {
                            f.kind.as_deref().is_none_or(|kind| task.kind == kind)
                                && f.state.is_none_or(|state| task.state == state)
                        })
                })
                .cloned()
                .collect())
        }

        async fn request_cancel(
            &self,
            _session_id: SessionId,
            task_id: &str,
        ) -> everruns_core::error::Result<Option<SessionTask>> {
            let mut tasks = self.tasks.lock().unwrap();
            let Some(task) = tasks.get_mut(task_id) else {
                return Ok(None);
            };
            task.cancel_requested_at.get_or_insert_with(Utc::now);
            task.updated_at = Utc::now();
            Ok(Some(task.clone()))
        }

        async fn record_message(
            &self,
            session_id: SessionId,
            task_id: &str,
            message: NewTaskMessage,
        ) -> everruns_core::error::Result<TaskMessage> {
            let stored = {
                let tasks = self.tasks.lock().unwrap();
                let Some(task) = tasks.get(task_id) else {
                    return Err(everruns_core::error::AgentLoopError::tool(format!(
                        "no task {task_id}"
                    )));
                };
                task.clone()
            };
            // Stale-attempt fence (mirrors DbSessionTaskRegistry).
            if let Some(expected) = message.expected_attempt
                && expected != stored.attempt
            {
                return Err(everruns_core::error::AgentLoopError::store(format!(
                    "Stale attempt {expected} for task {task_id} (current attempt {})",
                    stored.attempt
                )));
            }
            let recorded = TaskMessage {
                id: generate_task_message_id(),
                task_id: task_id.to_string(),
                direction: message.direction,
                content: message.content,
                in_reply_to: message.in_reply_to,
                created_at: Utc::now(),
            };
            // Answering messages clear a matching pending input request and
            // return the task to running.
            if let Some(in_reply_to) = &recorded.in_reply_to
                && stored
                    .input_request
                    .as_ref()
                    .is_some_and(|req| &req.id == in_reply_to)
            {
                self.update(
                    session_id,
                    task_id,
                    SessionTaskUpdate {
                        state: Some(SessionTaskState::Running),
                        ..Default::default()
                    },
                )
                .await?;
            }
            self.messages
                .lock()
                .unwrap()
                .entry(task_id.to_string())
                .or_default()
                .push(recorded.clone());
            Ok(recorded)
        }

        async fn list_messages(
            &self,
            _session_id: SessionId,
            task_id: &str,
            limit: Option<u32>,
            after_id: Option<&str>,
        ) -> everruns_core::error::Result<Vec<TaskMessage>> {
            let messages = self.messages.lock().unwrap();
            let all = messages.get(task_id).cloned().unwrap_or_default();
            let mut iter: Box<dyn Iterator<Item = TaskMessage>> = if let Some(cursor) = after_id {
                Box::new(all.into_iter().skip_while(move |m| m.id != cursor).skip(1))
            } else {
                Box::new(all.into_iter())
            };
            let collected: Vec<_> = iter.by_ref().collect();
            if let Some(limit) = limit {
                if after_id.is_some() {
                    return Ok(collected.into_iter().take(limit as usize).collect());
                }
                let skip = collected.len().saturating_sub(limit as usize);
                return Ok(collected.into_iter().skip(skip).collect());
            }
            Ok(collected)
        }
    }

    /// Test executor kind. `deliver`/`cancel` succeed and log invocations.
    /// The executor is process-global (inventory), so invocations are logged
    /// per task id; task ids are unique per test, which keeps assertions
    /// race-free under parallel test execution.
    const TEST_EXECUTOR_KIND: &str = "session_tasks_test";
    static TEST_DELIVERED: Mutex<Vec<String>> = Mutex::new(Vec::new());
    static TEST_CANCELED: Mutex<Vec<String>> = Mutex::new(Vec::new());

    fn executor_invocations(log: &Mutex<Vec<String>>, task_id: &str) -> usize {
        log.lock()
            .unwrap()
            .iter()
            .filter(|id| *id == task_id)
            .count()
    }

    struct TestTaskExecutor;

    #[async_trait]
    impl TaskExecutor for TestTaskExecutor {
        fn kind(&self) -> &str {
            TEST_EXECUTOR_KIND
        }

        async fn deliver(
            &self,
            task: &SessionTask,
            _message: &TaskMessage,
            _context: &ToolContext,
        ) -> everruns_core::error::Result<()> {
            TEST_DELIVERED.lock().unwrap().push(task.id.clone());
            Ok(())
        }

        async fn cancel(
            &self,
            task: &SessionTask,
            _context: &ToolContext,
        ) -> everruns_core::error::Result<()> {
            TEST_CANCELED.lock().unwrap().push(task.id.clone());
            Ok(())
        }
    }

    inventory::submit! {
        TaskExecutorPlugin {
            executor: || Arc::new(TestTaskExecutor),
        }
    }

    fn test_context(registry: Arc<InMemorySessionTaskRegistry>) -> ToolContext {
        ToolContext::new(SessionId::new()).with_session_task_registry(registry)
    }

    async fn create_task(
        registry: &InMemorySessionTaskRegistry,
        context: &ToolContext,
        kind: &str,
        state: SessionTaskState,
    ) -> SessionTask {
        registry
            .create(CreateSessionTask {
                session_id: context.session_id,
                id: None,
                kind: kind.to_string(),
                display_name: "Test Task".to_string(),
                spec: json!({}),
                state,
                links: TaskLinks::default(),
                wake_policy: TaskWakePolicy::Silent,
            })
            .await
            .unwrap()
    }

    // Metadata/tool-list constants covered by builtin_capabilities_satisfy_registry_invariants.

    #[tokio::test]
    async fn tools_error_without_registry() {
        let context = ToolContext::new(SessionId::new());
        let result = ListTasksTool
            .execute_with_context(json!({}), &context)
            .await;
        assert!(matches!(result, ToolExecutionResult::ToolError(_)));
        let result = WaitTaskTool
            .execute_with_context(json!({"task_id": "task_x"}), &context)
            .await;
        assert!(matches!(result, ToolExecutionResult::ToolError(_)));
    }

    #[tokio::test]
    async fn list_tasks_returns_compact_entries_with_filters() {
        let registry = Arc::new(InMemorySessionTaskRegistry::default());
        let context = test_context(registry.clone());
        let running = create_task(
            &registry,
            &context,
            TEST_EXECUTOR_KIND,
            SessionTaskState::Running,
        )
        .await;
        create_task(&registry, &context, "other_kind", SessionTaskState::Queued).await;

        let result = ListTasksTool
            .execute_with_context(json!({}), &context)
            .await;
        let ToolExecutionResult::Success(value) = result else {
            panic!("expected success: {result:?}");
        };
        assert_eq!(value["count"], 2);

        let result = ListTasksTool
            .execute_with_context(
                json!({"kind": TEST_EXECUTOR_KIND, "state": "running"}),
                &context,
            )
            .await;
        let ToolExecutionResult::Success(value) = result else {
            panic!("expected success: {result:?}");
        };
        assert_eq!(value["count"], 1);
        let entry = &value["tasks"][0];
        assert_eq!(entry["id"], running.id);
        assert_eq!(entry["state"], "running");
        assert!(entry.get("display_name").is_some());
        assert!(entry.get("created_at").is_some());
        // Compact entries omit the full spec.
        assert!(entry.get("spec").is_none());
    }

    #[tokio::test]
    async fn get_task_returns_snapshot_and_recent_messages() {
        let registry = Arc::new(InMemorySessionTaskRegistry::default());
        let context = test_context(registry.clone());
        let task = create_task(
            &registry,
            &context,
            TEST_EXECUTOR_KIND,
            SessionTaskState::Running,
        )
        .await;
        for i in 0..25 {
            registry
                .record_message(
                    context.session_id,
                    &task.id,
                    NewTaskMessage::outbound_text(format!("update {i}")),
                )
                .await
                .unwrap();
        }

        let result = GetTaskTool
            .execute_with_context(json!({"task_id": task.id}), &context)
            .await;
        let ToolExecutionResult::Success(value) = result else {
            panic!("expected success: {result:?}");
        };
        assert_eq!(value["task"]["id"], task.id);
        let messages = value["messages"].as_array().unwrap();
        assert_eq!(messages.len(), GET_TASK_MESSAGE_LIMIT as usize);
        // Most recent messages, oldest first.
        assert_eq!(messages[0]["content"][0]["text"], "update 5");
        assert_eq!(messages[19]["content"][0]["text"], "update 24");
    }

    #[tokio::test]
    async fn get_task_unknown_id_errors() {
        let registry = Arc::new(InMemorySessionTaskRegistry::default());
        let context = test_context(registry);
        let result = GetTaskTool
            .execute_with_context(json!({"task_id": "task_missing"}), &context)
            .await;
        assert!(matches!(result, ToolExecutionResult::ToolError(_)));
    }

    #[tokio::test]
    async fn message_task_records_and_delivers() {
        let registry = Arc::new(InMemorySessionTaskRegistry::default());
        let context = test_context(registry.clone());
        let task = create_task(
            &registry,
            &context,
            TEST_EXECUTOR_KIND,
            SessionTaskState::Running,
        )
        .await;

        let result = MessageTaskTool
            .execute_with_context(
                json!({"task_id": task.id, "message": "keep going"}),
                &context,
            )
            .await;
        let ToolExecutionResult::Success(value) = result else {
            panic!("expected success: {result:?}");
        };
        assert_eq!(value["delivery"], "delivered");
        assert_eq!(executor_invocations(&TEST_DELIVERED, &task.id), 1);

        let messages = registry
            .list_messages(context.session_id, &task.id, None, None)
            .await
            .unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].direction, TaskMessageDirection::Inbound);
        assert_eq!(
            messages[0].content,
            vec![TaskMessagePart::text("keep going")]
        );
    }

    #[tokio::test]
    async fn message_task_without_executor_still_records() {
        let registry = Arc::new(InMemorySessionTaskRegistry::default());
        let context = test_context(registry.clone());
        let task = create_task(
            &registry,
            &context,
            "kind_without_executor",
            SessionTaskState::Running,
        )
        .await;

        let result = MessageTaskTool
            .execute_with_context(json!({"task_id": task.id, "message": "hello"}), &context)
            .await;
        let ToolExecutionResult::Success(value) = result else {
            panic!("expected success: {result:?}");
        };
        assert_eq!(value["recorded"], true);
        let delivery = value["delivery"].as_str().unwrap();
        assert!(delivery.starts_with("failed:"), "delivery: {delivery}");
        let messages = registry
            .list_messages(context.session_id, &task.id, None, None)
            .await
            .unwrap();
        assert_eq!(messages.len(), 1);
    }

    #[tokio::test]
    async fn message_task_in_reply_to_resumes_awaiting_input() {
        let registry = Arc::new(InMemorySessionTaskRegistry::default());
        let context = test_context(registry.clone());
        let task = create_task(
            &registry,
            &context,
            TEST_EXECUTOR_KIND,
            SessionTaskState::Running,
        )
        .await;
        registry
            .update(
                context.session_id,
                &task.id,
                SessionTaskUpdate {
                    input_request: Some(TaskInputRequest {
                        id: "req_1".to_string(),
                        prompt: "Approve?".to_string(),
                        expected: None,
                    }),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let result = MessageTaskTool
            .execute_with_context(
                json!({"task_id": task.id, "message": "yes", "in_reply_to": "req_1"}),
                &context,
            )
            .await;
        assert!(matches!(result, ToolExecutionResult::Success(_)));

        let current = registry
            .get(context.session_id, &task.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(current.state, SessionTaskState::Running);
        assert!(current.input_request.is_none());
    }

    #[tokio::test]
    async fn cancel_task_records_intent_and_calls_executor() {
        let registry = Arc::new(InMemorySessionTaskRegistry::default());
        let context = test_context(registry.clone());
        let task = create_task(
            &registry,
            &context,
            TEST_EXECUTOR_KIND,
            SessionTaskState::Running,
        )
        .await;

        let result = CancelTaskTool
            .execute_with_context(json!({"task_id": task.id}), &context)
            .await;
        let ToolExecutionResult::Success(value) = result else {
            panic!("expected success: {result:?}");
        };
        assert_eq!(value["cancel_requested"], true);
        assert_eq!(value["executor"], "cancellation requested");
        assert_eq!(executor_invocations(&TEST_CANCELED, &task.id), 1);

        let current = registry
            .get(context.session_id, &task.id)
            .await
            .unwrap()
            .unwrap();
        assert!(current.cancel_requested_at.is_some());
    }

    #[tokio::test]
    async fn cancel_task_on_terminal_task_skips_executor() {
        let registry = Arc::new(InMemorySessionTaskRegistry::default());
        let context = test_context(registry.clone());
        let task = create_task(
            &registry,
            &context,
            TEST_EXECUTOR_KIND,
            SessionTaskState::Succeeded,
        )
        .await;

        let result = CancelTaskTool
            .execute_with_context(json!({"task_id": task.id}), &context)
            .await;
        let ToolExecutionResult::Success(value) = result else {
            panic!("expected success: {result:?}");
        };
        assert_eq!(value["executor"], "task already terminal");
        assert_eq!(executor_invocations(&TEST_CANCELED, &task.id), 0);
    }

    #[tokio::test]
    async fn wait_task_returns_immediately_when_terminal() {
        let registry = Arc::new(InMemorySessionTaskRegistry::default());
        let context = test_context(registry.clone());
        let task = create_task(
            &registry,
            &context,
            TEST_EXECUTOR_KIND,
            SessionTaskState::Running,
        )
        .await;
        registry
            .update(
                context.session_id,
                &task.id,
                SessionTaskUpdate {
                    state: Some(SessionTaskState::Failed),
                    error: Some(TaskError {
                        kind: "error".to_string(),
                        message: "boom".to_string(),
                    }),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let result = WaitTaskTool
            .execute_with_context(json!({"task_id": task.id}), &context)
            .await;
        let ToolExecutionResult::Success(value) = result else {
            panic!("expected success: {result:?}");
        };
        assert_eq!(value["timed_out"], false);
        assert_eq!(value["task"]["state"], "failed");
        assert_eq!(value["task"]["error"]["message"], "boom");
    }

    #[tokio::test]
    async fn wait_task_returns_when_awaiting_input() {
        let registry = Arc::new(InMemorySessionTaskRegistry::default());
        let context = test_context(registry.clone());
        let task = create_task(
            &registry,
            &context,
            TEST_EXECUTOR_KIND,
            SessionTaskState::AwaitingInput,
        )
        .await;

        let result = WaitTaskTool
            .execute_with_context(json!({"task_id": task.id}), &context)
            .await;
        let ToolExecutionResult::Success(value) = result else {
            panic!("expected success: {result:?}");
        };
        assert_eq!(value["timed_out"], false);
        assert_eq!(value["task"]["state"], "awaiting_input");
    }

    #[tokio::test(start_paused = true)]
    async fn wait_task_times_out_with_snapshot() {
        let registry = Arc::new(InMemorySessionTaskRegistry::default());
        let context = test_context(registry.clone());
        let task = create_task(
            &registry,
            &context,
            TEST_EXECUTOR_KIND,
            SessionTaskState::Running,
        )
        .await;

        let result = WaitTaskTool
            .execute_with_context(json!({"task_id": task.id, "timeout_seconds": 3}), &context)
            .await;
        let ToolExecutionResult::Success(value) = result else {
            panic!("expected success: {result:?}");
        };
        assert_eq!(value["timed_out"], true);
        assert_eq!(value["task"]["state"], "running");
    }
}

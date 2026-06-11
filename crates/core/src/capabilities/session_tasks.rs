// Session Tasks Capability
//
// Generic agent-facing tools over the session task registry
// (specs/session-tasks.md): list_tasks / get_task / message_task /
// cancel_task / wait_task. Spawning stays per-capability (spawn_subagent,
// spawn_agent, spawn_background) — every spawn creates a task and returns its
// task_id; these tools provide the uniform query/messaging/cancel/wait plane.
//
// Decision: tools degrade gracefully when `context.session_task_registry` is
// absent (e.g. embedders without a registry) — they return a tool error
// instead of panicking, matching other registry-backed capabilities.

use super::{Capability, CapabilityStatus};
use crate::session_task::{
    NewTaskMessage, SessionTask, SessionTaskFilter, SessionTaskRegistry, SessionTaskState,
    TaskMessage, find_task_executor,
};
use crate::tool_types::ToolHints;
use crate::tools::{Tool, ToolExecutionResult};
use crate::traits::ToolContext;
use async_trait::async_trait;
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

fn require_str<'a>(args: &'a Value, field: &str) -> Result<&'a str, ToolExecutionResult> {
    args.get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            ToolExecutionResult::tool_error(format!("Missing required parameter: {field}"))
        })
}

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
        let registry = match require_task_registry(context) {
            Ok(r) => r,
            Err(e) => return e,
        };
        let filter = SessionTaskFilter {
            kind: arguments
                .get("kind")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToString::to_string),
            state: arguments
                .get("state")
                .and_then(Value::as_str)
                .map(SessionTaskState::from),
        };
        let tasks = match registry.list(context.session_id, Some(&filter)).await {
            Ok(tasks) => tasks,
            Err(e) => return ToolExecutionResult::internal_error(e),
        };
        let entries = tasks.iter().map(compact_task_json).collect::<Vec<_>>();
        ToolExecutionResult::success(json!({
            "tasks": entries,
            "count": entries.len(),
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// =============================================================================
// Tool: get_task
// =============================================================================

pub struct GetTaskTool;

#[async_trait]
impl Tool for GetTaskTool {
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
        let task_id = match require_str(&arguments, "task_id") {
            Ok(id) => id,
            Err(e) => return e,
        };
        let task = match load_task(context, task_id).await {
            Ok(task) => task,
            Err(e) => return e,
        };
        let registry = match require_task_registry(context) {
            Ok(r) => r,
            Err(e) => return e,
        };
        let messages = registry
            .list_messages(context.session_id, task_id, Some(GET_TASK_MESSAGE_LIMIT))
            .await
            .unwrap_or_default();
        ToolExecutionResult::success(json!({
            "task": full_task_json(&task),
            "messages": messages.iter().map(message_json).collect::<Vec<_>>(),
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// =============================================================================
// Tool: message_task
// =============================================================================

pub struct MessageTaskTool;

#[async_trait]
impl Tool for MessageTaskTool {
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
        let task_id = match require_str(&arguments, "task_id") {
            Ok(id) => id.to_string(),
            Err(e) => return e,
        };
        let message = match require_str(&arguments, "message") {
            Ok(m) => m.to_string(),
            Err(e) => return e,
        };
        let in_reply_to = arguments
            .get("in_reply_to")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string);

        let task = match load_task(context, &task_id).await {
            Ok(task) => task,
            Err(e) => return e,
        };
        let registry = match require_task_registry(context) {
            Ok(r) => r,
            Err(e) => return e,
        };

        let mut new_message = NewTaskMessage::inbound_text(message);
        new_message.in_reply_to = in_reply_to;
        let recorded = match registry
            .record_message(context.session_id, &task_id, new_message)
            .await
        {
            Ok(recorded) => recorded,
            Err(e) => return ToolExecutionResult::internal_error(e),
        };

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

        ToolExecutionResult::success(json!({
            "task_id": task_id,
            "message_id": recorded.id,
            "recorded": true,
            "delivery": delivery,
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// =============================================================================
// Tool: cancel_task
// =============================================================================

pub struct CancelTaskTool;

#[async_trait]
impl Tool for CancelTaskTool {
    fn name(&self) -> &str {
        "cancel_task"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Cancel Task")
    }

    fn description(&self) -> &str {
        "Request cooperative cancellation of a task. The task winds down and may still end succeeded or failed."
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
        let task_id = match require_str(&arguments, "task_id") {
            Ok(id) => id.to_string(),
            Err(e) => return e,
        };
        let registry = match require_task_registry(context) {
            Ok(r) => r,
            Err(e) => return e,
        };
        let task = match registry.request_cancel(context.session_id, &task_id).await {
            Ok(Some(task)) => task,
            Ok(None) => {
                return ToolExecutionResult::tool_error(format!(
                    "No task found with id: {task_id}"
                ));
            }
            Err(e) => return ToolExecutionResult::internal_error(e),
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

        ToolExecutionResult::success(json!({
            "task_id": task_id,
            "state": task.state,
            "cancel_requested": true,
            "executor": executor_result,
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// =============================================================================
// Tool: wait_task
// =============================================================================

pub struct WaitTaskTool;

#[async_trait]
impl Tool for WaitTaskTool {
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
        let task_id = match require_str(&arguments, "task_id") {
            Ok(id) => id.to_string(),
            Err(e) => return e,
        };
        let timeout_secs = arguments
            .get("timeout_seconds")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_WAIT_TIMEOUT_SECS);
        let deadline = Instant::now() + Duration::from_secs(timeout_secs);
        let mut polls: u64 = 0;

        loop {
            let task = match load_task(context, &task_id).await {
                Ok(task) => task,
                Err(e) => return e,
            };
            if task.state.is_terminal() || task.state == SessionTaskState::AwaitingInput {
                return ToolExecutionResult::success(json!({
                    "task": full_task_json(&task),
                    "timed_out": false,
                }));
            }
            if Instant::now() >= deadline {
                return ToolExecutionResult::success(json!({
                    "task": full_task_json(&task),
                    "timed_out": true,
                    "message": format!("Task {task_id} still {} after {timeout_secs}s", task.state),
                }));
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

    fn requires_context(&self) -> bool {
        true
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::session_task::{
        CreateSessionTask, SessionTaskUpdate, TaskError, TaskExecutor, TaskExecutorPlugin,
        TaskInputRequest, TaskLinks, TaskMessageDirection, TaskMessagePart, TaskWakePolicy,
        apply_task_update, generate_task_message_id, new_session_task,
    };
    use crate::typed_id::SessionId;
    use chrono::Utc;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// In-memory `SessionTaskRegistry` test double. Updates route through
    /// `apply_task_update` so lifecycle invariants match real backends.
    #[derive(Default)]
    pub(crate) struct InMemorySessionTaskRegistry {
        tasks: Mutex<HashMap<String, SessionTask>>,
        messages: Mutex<HashMap<String, Vec<TaskMessage>>>,
    }

    #[async_trait]
    impl SessionTaskRegistry for InMemorySessionTaskRegistry {
        async fn create(&self, input: CreateSessionTask) -> crate::error::Result<SessionTask> {
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
        ) -> crate::error::Result<Option<SessionTask>> {
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
        ) -> crate::error::Result<Option<SessionTask>> {
            Ok(self.tasks.lock().unwrap().get(task_id).cloned())
        }

        async fn list(
            &self,
            _session_id: SessionId,
            filter: Option<&SessionTaskFilter>,
        ) -> crate::error::Result<Vec<SessionTask>> {
            let tasks = self.tasks.lock().unwrap();
            Ok(tasks
                .values()
                .filter(|task| {
                    filter.is_none_or(|f| {
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
        ) -> crate::error::Result<Option<SessionTask>> {
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
        ) -> crate::error::Result<TaskMessage> {
            let stored = {
                let tasks = self.tasks.lock().unwrap();
                let Some(task) = tasks.get(task_id) else {
                    return Err(crate::error::AgentLoopError::tool(format!(
                        "no task {task_id}"
                    )));
                };
                task.clone()
            };
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
        ) -> crate::error::Result<Vec<TaskMessage>> {
            let messages = self.messages.lock().unwrap();
            let all = messages.get(task_id).cloned().unwrap_or_default();
            let Some(limit) = limit else {
                return Ok(all);
            };
            let skip = all.len().saturating_sub(limit as usize);
            Ok(all.into_iter().skip(skip).collect())
        }
    }

    /// Test executor kind. `deliver`/`cancel` succeed and count invocations.
    const TEST_EXECUTOR_KIND: &str = "session_tasks_test";
    static TEST_DELIVERED: AtomicUsize = AtomicUsize::new(0);
    static TEST_CANCELED: AtomicUsize = AtomicUsize::new(0);

    struct TestTaskExecutor;

    #[async_trait]
    impl TaskExecutor for TestTaskExecutor {
        fn kind(&self) -> &str {
            TEST_EXECUTOR_KIND
        }

        async fn deliver(
            &self,
            _task: &SessionTask,
            _message: &TaskMessage,
            _context: &ToolContext,
        ) -> crate::error::Result<()> {
            TEST_DELIVERED.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn cancel(
            &self,
            _task: &SessionTask,
            _context: &ToolContext,
        ) -> crate::error::Result<()> {
            TEST_CANCELED.fetch_add(1, Ordering::SeqCst);
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

    #[test]
    fn capability_basics() {
        let cap = SessionTasksCapability;
        assert_eq!(cap.id(), SESSION_TASKS_CAPABILITY_ID);
        assert_eq!(cap.category(), Some("Orchestration"));
        let tools = cap.tools();
        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert_eq!(
            names,
            vec![
                "list_tasks",
                "get_task",
                "message_task",
                "cancel_task",
                "wait_task"
            ]
        );
    }

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

        let before = TEST_DELIVERED.load(Ordering::SeqCst);
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
        assert_eq!(TEST_DELIVERED.load(Ordering::SeqCst), before + 1);

        let messages = registry
            .list_messages(context.session_id, &task.id, None)
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
            .list_messages(context.session_id, &task.id, None)
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

        let before = TEST_CANCELED.load(Ordering::SeqCst);
        let result = CancelTaskTool
            .execute_with_context(json!({"task_id": task.id}), &context)
            .await;
        let ToolExecutionResult::Success(value) = result else {
            panic!("expected success: {result:?}");
        };
        assert_eq!(value["cancel_requested"], true);
        assert_eq!(value["executor"], "cancellation requested");
        assert_eq!(TEST_CANCELED.load(Ordering::SeqCst), before + 1);

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

        let before = TEST_CANCELED.load(Ordering::SeqCst);
        let result = CancelTaskTool
            .execute_with_context(json!({"task_id": task.id}), &context)
            .await;
        let ToolExecutionResult::Success(value) = result else {
            panic!("expected success: {result:?}");
        };
        assert_eq!(value["executor"], "task already terminal");
        assert_eq!(TEST_CANCELED.load(Ordering::SeqCst), before);
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

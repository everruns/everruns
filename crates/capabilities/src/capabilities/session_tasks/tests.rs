use super::*;
use chrono::Utc;
use everruns_contracts::typed_id::SessionId;
use everruns_core::session_task::{
    CreateSessionTask, SessionTaskUpdate, TaskError, TaskExecutor, TaskExecutorPlugin,
    TaskInputRequest, TaskLinks, TaskMessageDirection, TaskMessagePart, TaskWakePolicy,
    apply_task_update, generate_task_message_id, new_session_task,
};
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
    ) -> everruns_contracts::error::Result<SessionTask> {
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
    ) -> everruns_contracts::error::Result<Option<SessionTask>> {
        let mut tasks = self.tasks.lock().unwrap();
        let Some(task) = tasks.get_mut(task_id) else {
            return Ok(None);
        };
        if update
            .expected_recovery
            .as_ref()
            .is_some_and(|guard| !guard.matches(task))
        {
            return Ok(None);
        }
        apply_task_update(task, update, Utc::now());
        Ok(Some(task.clone()))
    }

    async fn get(
        &self,
        _session_id: SessionId,
        task_id: &str,
    ) -> everruns_contracts::error::Result<Option<SessionTask>> {
        Ok(self.tasks.lock().unwrap().get(task_id).cloned())
    }

    async fn list(
        &self,
        session_id: SessionId,
        filter: Option<&SessionTaskFilter>,
    ) -> everruns_contracts::error::Result<Vec<SessionTask>> {
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
    ) -> everruns_contracts::error::Result<Option<SessionTask>> {
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
    ) -> everruns_contracts::error::Result<TaskMessage> {
        let stored = {
            let tasks = self.tasks.lock().unwrap();
            let Some(task) = tasks.get(task_id) else {
                return Err(everruns_contracts::error::AgentLoopError::tool(format!(
                    "no task {task_id}"
                )));
            };
            task.clone()
        };
        // Stale-attempt fence (mirrors DbSessionTaskRegistry).
        if let Some(expected) = message.expected_attempt
            && expected != stored.attempt
        {
            return Err(everruns_contracts::error::AgentLoopError::store(format!(
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
    ) -> everruns_contracts::error::Result<Vec<TaskMessage>> {
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
    ) -> everruns_contracts::error::Result<()> {
        TEST_DELIVERED.lock().unwrap().push(task.id.clone());
        Ok(())
    }

    async fn cancel(
        &self,
        task: &SessionTask,
        _context: &ToolContext,
    ) -> everruns_contracts::error::Result<()> {
        TEST_CANCELED.lock().unwrap().push(task.id.clone());
        Ok(())
    }
}

inventory::submit! {
    TaskExecutorPlugin {
        executor: || Arc::new(TestTaskExecutor),
    }
}

/// Test executor kind whose `cancel` winds the task down synchronously:
/// it posts a final outbound message and transitions to `canceled`, the
/// way in-process executors (background tools, monitors) do.
const TEST_WINDDOWN_EXECUTOR_KIND: &str = "session_tasks_test_winddown";

struct WindDownTaskExecutor;

#[async_trait]
impl TaskExecutor for WindDownTaskExecutor {
    fn kind(&self) -> &str {
        TEST_WINDDOWN_EXECUTOR_KIND
    }

    async fn deliver(
        &self,
        _task: &SessionTask,
        _message: &TaskMessage,
        _context: &ToolContext,
    ) -> everruns_contracts::error::Result<()> {
        Ok(())
    }

    async fn cancel(
        &self,
        task: &SessionTask,
        context: &ToolContext,
    ) -> everruns_contracts::error::Result<()> {
        let registry = context.session_task_registry.as_ref().unwrap();
        registry
            .record_message(
                context.session_id,
                &task.id,
                NewTaskMessage::outbound_text("stopping: partial results saved"),
            )
            .await?;
        registry
            .update(
                context.session_id,
                &task.id,
                SessionTaskUpdate {
                    state: Some(SessionTaskState::Canceled),
                    summary: Some("canceled after 2 of 5 steps".to_string()),
                    ..Default::default()
                },
            )
            .await?;
        Ok(())
    }
}

inventory::submit! {
    TaskExecutorPlugin {
        executor: || Arc::new(WindDownTaskExecutor),
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
async fn list_tasks_keeps_ids_readable_when_failure_summaries_are_large() {
    let registry = Arc::new(InMemorySessionTaskRegistry::default());
    let context = test_context(registry.clone());
    let summary = "failed validation: ".to_string() + &"diagnostic 測試\n".repeat(1000);
    let mut ids = Vec::new();
    for _ in 0..4 {
        let task = create_task(
            &registry,
            &context,
            TEST_EXECUTOR_KIND,
            SessionTaskState::Failed,
        )
        .await;
        registry
            .update(
                context.session_id,
                &task.id,
                SessionTaskUpdate {
                    summary: Some(summary.clone()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        ids.push(task.id);
    }
    let ToolExecutionResult::Success(list) = ListTasksTool
        .execute_with_context(json!({}), &context)
        .await
    else {
        panic!("list should succeed");
    };
    assert!(
        list.to_string().len() < 8192,
        "compact task metadata must not be crowded out by diagnostics"
    );
    for id in ids {
        let entry = list["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|task| task["id"] == id)
            .unwrap();
        assert_eq!(entry["state"], "failed");
        assert!(entry["summary"].as_str().unwrap().contains("get_task"));
        let ToolExecutionResult::Success(full) = GetTaskTool
            .execute_with_context(json!({"task_id": id}), &context)
            .await
        else {
            panic!("get should succeed");
        };
        assert_eq!(full["task"]["summary"], summary);
    }
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
async fn cancel_task_returns_post_cancel_state_and_message_tail() {
    let registry = Arc::new(InMemorySessionTaskRegistry::default());
    let context = test_context(registry.clone());
    let task = create_task(
        &registry,
        &context,
        TEST_WINDDOWN_EXECUTOR_KIND,
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

    let result = CancelTaskTool
        .execute_with_context(json!({"task_id": task.id}), &context)
        .await;
    let ToolExecutionResult::Success(value) = result else {
        panic!("expected success: {result:?}");
    };
    // Existing fields stay, but reflect the state after wind-down.
    assert_eq!(value["task_id"], task.id);
    assert_eq!(value["cancel_requested"], true);
    assert_eq!(value["executor"], "cancellation requested");
    assert_eq!(value["state"], "canceled");
    // Additive get_task-shaped fields: full snapshot plus live tail.
    assert_eq!(value["task"]["id"], task.id);
    assert_eq!(value["task"]["state"], "canceled");
    assert_eq!(value["task"]["summary"], "canceled after 2 of 5 steps");
    assert!(value["task"]["cancel_requested_at"].is_string());
    let messages = value["messages"].as_array().unwrap();
    assert_eq!(messages.len(), GET_TASK_MESSAGE_LIMIT as usize);
    assert_eq!(
        messages.last().unwrap()["content"][0]["text"],
        "stopping: partial results saved"
    );

    // Same shape get_task returns, so no follow-up read is needed.
    let ToolExecutionResult::Success(fresh) = GetTaskTool
        .execute_with_context(json!({"task_id": task.id}), &context)
        .await
    else {
        panic!("get should succeed");
    };
    assert_eq!(value["task"], fresh["task"]);
    assert_eq!(value["messages"], fresh["messages"]);
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

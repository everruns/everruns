//! Unit tests for `spawn_background`, background run artifacts, schedules,
//! cancellation, and re-attach.

use super::test_support::*;
use super::*;
use async_trait::async_trait;
use everruns_contracts::tool_types::ToolHints;
use everruns_core::background::BackgroundExecutableTool;
use everruns_core::session_task::SessionTaskRegistry;
use everruns_core::{session_files::SessionFileSystem, session_services::SessionScheduleStore};
use std::sync::Arc as StdArc;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Default)]
struct TestFailingBackgroundTool;

#[async_trait]
impl BackgroundExecutableTool for TestFailingBackgroundTool {
    async fn execute_background(
        &self,
        _arguments: Value,
        _context: ToolContext,
        sink: Arc<dyn BackgroundEventSink>,
    ) -> std::result::Result<BackgroundOutcome, ToolExecutionResult> {
        sink.status("Running failing test")
            .await
            .map_err(ToolExecutionResult::internal_error)?;
        sink.output("stderr", "background failed")
            .await
            .map_err(ToolExecutionResult::internal_error)?;
        Err(ToolExecutionResult::tool_error("boom"))
    }
}

#[async_trait]
impl Tool for TestFailingBackgroundTool {
    fn name(&self) -> &str {
        "test_background_fail"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Test Background Fail")
    }

    fn description(&self) -> &str {
        "failing background test tool"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {}
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("foreground unsupported")
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default().with_supports_background(true)
    }

    fn as_background_executable(&self) -> Option<&dyn BackgroundExecutableTool> {
        Some(self)
    }
}

#[derive(Default)]
struct TestLargeOutputBackgroundTool;

#[async_trait]
impl BackgroundExecutableTool for TestLargeOutputBackgroundTool {
    async fn execute_background(
        &self,
        _arguments: Value,
        _context: ToolContext,
        sink: Arc<dyn BackgroundEventSink>,
    ) -> std::result::Result<BackgroundOutcome, ToolExecutionResult> {
        let large_chunk = "x".repeat(MAX_BACKGROUND_OUTPUT_LOG_CHARS + 4096);
        sink.output("stdout", &large_chunk)
            .await
            .map_err(ToolExecutionResult::internal_error)?;
        Ok(BackgroundOutcome {
            summary: "large output complete".to_string(),
            result: json!({"ok": true}),
            raw_output: None,
        })
    }
}

#[async_trait]
impl Tool for TestLargeOutputBackgroundTool {
    fn name(&self) -> &str {
        "test_background_large_output"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Test Background Large Output")
    }

    fn description(&self) -> &str {
        "background test tool with huge output"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {}
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("foreground unsupported")
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default().with_supports_background(true)
    }

    fn as_background_executable(&self) -> Option<&dyn BackgroundExecutableTool> {
        Some(self)
    }
}

struct BlockingBackgroundTool {
    release: StdArc<AtomicBool>,
}

#[async_trait]
impl BackgroundExecutableTool for BlockingBackgroundTool {
    async fn execute_background(
        &self,
        _arguments: Value,
        _context: ToolContext,
        sink: Arc<dyn BackgroundEventSink>,
    ) -> std::result::Result<BackgroundOutcome, ToolExecutionResult> {
        sink.status("Blocking until released")
            .await
            .map_err(ToolExecutionResult::internal_error)?;
        while !self.release.load(Ordering::SeqCst) {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        Ok(BackgroundOutcome {
            summary: "released".to_string(),
            result: json!({"ok": true}),
            raw_output: None,
        })
    }
}

#[async_trait]
impl Tool for BlockingBackgroundTool {
    fn name(&self) -> &str {
        "test_background_blocking"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Test Background Blocking")
    }

    fn description(&self) -> &str {
        "background test tool that waits for test release"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {}
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("foreground unsupported")
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default().with_supports_background(true)
    }

    fn as_background_executable(&self) -> Option<&dyn BackgroundExecutableTool> {
        Some(self)
    }
}

/// A background tool that sleeps indefinitely, allowing the test to exercise
/// cancel via the cancel-watcher without actually waiting forever.
#[derive(Default)]
struct SleepingBackgroundTool;

#[async_trait]
impl BackgroundExecutableTool for SleepingBackgroundTool {
    async fn execute_background(
        &self,
        _arguments: Value,
        _context: ToolContext,
        sink: Arc<dyn BackgroundEventSink>,
    ) -> std::result::Result<BackgroundOutcome, ToolExecutionResult> {
        sink.status("Sleeping forever")
            .await
            .map_err(ToolExecutionResult::internal_error)?;
        // Sleep for a very long time; the cancel-watcher will win the select.
        tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
        Ok(BackgroundOutcome {
            summary: "should not reach here".to_string(),
            result: json!({}),
            raw_output: None,
        })
    }
}

#[async_trait]
impl Tool for SleepingBackgroundTool {
    fn name(&self) -> &str {
        "test_background_sleeping"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Test Background Sleeping")
    }

    fn description(&self) -> &str {
        "background test tool that sleeps indefinitely"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {}
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("foreground unsupported")
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default().with_supports_background(true)
    }

    fn as_background_executable(&self) -> Option<&dyn BackgroundExecutableTool> {
        Some(self)
    }
}

#[tokio::test]
async fn test_spawn_background_executes_and_signals_session() {
    let session_id = SessionId::new();
    let file_store = Arc::new(TestFileStore::default());
    let platform_store = Arc::new(TestSubagentDelegate::default());
    let storage_store = Arc::new(NoopStorageStore);
    let task_registry = Arc::new(InMemoryTaskRegistry::default());
    let tool_registry = ToolRegistry::builder()
        .tool(SpawnBackgroundTool)
        .tool(TestBackgroundTool)
        .build();

    let context = ToolContext::with_stores(session_id, file_store.clone(), storage_store)
        .with_tool_registry(Arc::new(tool_registry))
        .with_nested_tool_policy(allow_all_policy())
        .with_subagent_delegate(platform_store.clone())
        .with_session_task_registry(task_registry.clone());

    let tool = SpawnBackgroundTool;
    let result = tool
        .execute_with_context(
            json!({
                "tool": "test_background",
                "args": { "summary": "Background complete" }
            }),
            &context,
        )
        .await;

    let ToolExecutionResult::Success(value) = result else {
        panic!("spawn_background should succeed");
    };
    let run_id = value["run_id"].as_str().unwrap().to_string();
    let task_id = value["task_id"].as_str().unwrap().to_string();

    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            if let Ok(Some(task)) = task_registry.get(session_id, &task_id).await
                && task.state == everruns_core::session_task::SessionTaskState::Succeeded
            {
                break task;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("background run should complete");
    let _ = run_id; // still available in result json

    let messages = platform_store.sent_messages.lock().unwrap().clone();
    assert_eq!(messages.len(), 1);
    assert!(messages[0].contains("Background run completed"));

    let log_file = file_store
        .read_file(session_id, &format!("/.background/{run_id}/output.log"))
        .await
        .unwrap()
        .expect("log file");
    assert!(
        log_file
            .content
            .as_deref()
            .unwrap_or_default()
            .contains("hello from background")
    );
}

/// EVE-1165: under the read-only default the model-facing store denies every
/// write, so run artifacts land through the host's runtime artifact store, and
/// the model can read (not write) the `result_path` the wake message names.
#[tokio::test]
async fn test_spawn_background_writes_artifacts_under_read_only_policy() {
    use everruns_core::WorkspacePolicy;
    use everruns_core::host::PolicyFileStore;
    use everruns_core::session_files::RuntimeArtifactFileSystem;

    let session_id = SessionId::new();
    let backing = Arc::new(TestFileStore::default());
    let model_store: Arc<dyn SessionFileSystem> = Arc::new(PolicyFileStore::new(
        backing.clone(),
        WorkspacePolicy::read_only(),
    ));
    let artifacts: Arc<dyn SessionFileSystem> = Arc::new(PolicyFileStore::new(
        backing.clone(),
        WorkspacePolicy::runtime_artifacts(),
    ));
    let platform_store = Arc::new(TestSubagentDelegate::default());
    let task_registry = Arc::new(InMemoryTaskRegistry::default());
    let tool_registry = ToolRegistry::builder()
        .tool(SpawnBackgroundTool)
        .tool(TestBackgroundTool)
        .build();

    let mut context =
        ToolContext::with_stores(session_id, model_store.clone(), Arc::new(NoopStorageStore))
            .with_tool_registry(Arc::new(tool_registry))
            .with_nested_tool_policy(allow_all_policy())
            .with_subagent_delegate(platform_store.clone())
            .with_session_task_registry(task_registry.clone());
    context
        .extensions
        .insert(Arc::new(RuntimeArtifactFileSystem(artifacts)));

    let result = SpawnBackgroundTool
        .execute_with_context(
            json!({
                "tool": "test_background",
                "args": { "summary": "Background complete" }
            }),
            &context,
        )
        .await;
    let ToolExecutionResult::Success(value) = result else {
        panic!("spawn_background should succeed, got {result:?}");
    };
    let run_id = value["run_id"].as_str().unwrap().to_string();
    let task_id = value["task_id"].as_str().unwrap().to_string();

    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            if let Ok(Some(task)) = task_registry.get(session_id, &task_id).await
                && task.state == everruns_core::session_task::SessionTaskState::Succeeded
            {
                break task;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("background run should complete");

    let messages = platform_store.sent_messages.lock().unwrap().clone();
    let result_path = format!("/.background/{run_id}/result.json");
    assert!(
        messages
            .iter()
            .any(|message| message.contains(&result_path)),
        "wake message names the result path: {messages:?}"
    );
    for path in [result_path, format!("/.background/{run_id}/output.log")] {
        assert!(
            model_store
                .read_file(session_id, &path)
                .await
                .expect("the model may read run artifacts")
                .is_some(),
            "missing {path}"
        );
        assert!(
            model_store
                .write_file(session_id, &path, "forged", "text")
                .await
                .is_err(),
            "the model must not write {path}"
        );
    }
}

#[tokio::test]
async fn test_spawn_background_persists_failure_artifacts() {
    let session_id = SessionId::new();
    let file_store = Arc::new(TestFileStore::default());
    let storage_store = Arc::new(NoopStorageStore);
    let task_registry = Arc::new(InMemoryTaskRegistry::default());
    let tool_registry = ToolRegistry::builder()
        .tool(SpawnBackgroundTool)
        .tool(TestFailingBackgroundTool)
        .build();

    let context = ToolContext::with_stores(session_id, file_store.clone(), storage_store)
        .with_tool_registry(Arc::new(tool_registry))
        .with_nested_tool_policy(allow_all_policy())
        .with_session_task_registry(task_registry.clone());

    let result = SpawnBackgroundTool
        .execute_with_context(
            json!({
                "tool": "test_background_fail",
                "args": {}
            }),
            &context,
        )
        .await;

    let ToolExecutionResult::Success(value) = result else {
        panic!("spawn_background should succeed");
    };
    let run_id = value["run_id"].as_str().unwrap().to_string();
    let task_id = value["task_id"].as_str().unwrap().to_string();

    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            if let Ok(Some(task)) = task_registry.get(session_id, &task_id).await
                && task.state == everruns_core::session_task::SessionTaskState::Failed
            {
                break task;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("background run should fail");
    let _ = run_id;

    let log_file = file_store
        .read_file(session_id, &format!("/.background/{run_id}/output.log"))
        .await
        .unwrap()
        .expect("log file");
    assert!(
        log_file
            .content
            .as_deref()
            .unwrap_or_default()
            .contains("background failed")
    );

    let result_file = file_store
        .read_file(session_id, &format!("/.background/{run_id}/result.json"))
        .await
        .unwrap()
        .expect("result file");
    let result_json: Value =
        serde_json::from_str(result_file.content.as_deref().unwrap_or_default())
            .expect("valid json");
    assert_eq!(result_json["status"], "failed");
    assert_eq!(result_json["error"], "boom");
}

#[tokio::test]
async fn test_spawn_background_rejects_when_session_active_run_limit_reached() {
    let session_id = SessionId::new();
    let file_store = Arc::new(TestFileStore::default());
    let storage_store = Arc::new(NoopStorageStore);
    let task_registry = Arc::new(InMemoryTaskRegistry::default());
    let release = StdArc::new(AtomicBool::new(false));
    let tool_registry = ToolRegistry::builder()
        .tool(SpawnBackgroundTool)
        .tool(BlockingBackgroundTool {
            release: release.clone(),
        })
        .build();

    let context = ToolContext::with_stores(session_id, file_store, storage_store)
        .with_tool_registry(Arc::new(tool_registry))
        .with_nested_tool_policy(allow_all_policy())
        .with_session_task_registry(task_registry.clone());

    let mut task_ids = Vec::new();
    for _ in 0..MAX_ACTIVE_BACKGROUND_RUNS_PER_SESSION {
        let result = SpawnBackgroundTool
            .execute_with_context(
                json!({
                    "tool": "test_background_blocking",
                    "args": {}
                }),
                &context,
            )
            .await;

        let ToolExecutionResult::Success(value) = result else {
            panic!("background run below the session limit should start");
        };
        task_ids.push(value["task_id"].as_str().unwrap().to_string());
    }

    // Wait for all tasks to be running (semaphore acquired).
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let running = task_registry
                .list(
                    session_id,
                    Some(&everruns_core::session_task::SessionTaskFilter {
                        kind: Some(
                            everruns_core::session_task::TASK_KIND_BACKGROUND_TOOL.to_string(),
                        ),
                        state: Some(everruns_core::session_task::SessionTaskState::Running),
                    }),
                )
                .await
                .unwrap();
            if running.len() == MAX_ACTIVE_BACKGROUND_RUNS_PER_SESSION {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("background runs should become running");

    let result = SpawnBackgroundTool
        .execute_with_context(
            json!({
                "tool": "test_background_blocking",
                "args": {}
            }),
            &context,
        )
        .await;

    let ToolExecutionResult::ToolError(message) = result else {
        release.store(true, Ordering::SeqCst);
        panic!("spawn_background should reject once the session limit is reached");
    };
    assert!(message.contains("active background runs per session"));

    release.store(true, Ordering::SeqCst);
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        for task_id in task_ids {
            loop {
                if let Ok(Some(task)) = task_registry.get(session_id, &task_id).await
                    && task.state == everruns_core::session_task::SessionTaskState::Succeeded
                {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        }
    })
    .await
    .expect("blocking background runs should complete after release");

    // The permit drop is enqueued in the spawned task after it marks the
    // resource Completed, so the cache entry may still exist briefly once
    // we observe Completed status.  Poll until pruned rather than asserting
    // immediately to avoid a race.
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            if !has_session_background_permits(session_id) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("completed background runs should prune their per-session permit cache entry");
}

#[tokio::test]
async fn test_spawn_background_requires_task_registry() {
    let session_id = SessionId::new();
    let file_store = Arc::new(TestFileStore::default());
    let storage_store = Arc::new(NoopStorageStore);
    let tool_registry = ToolRegistry::builder()
        .tool(SpawnBackgroundTool)
        .tool(TestBackgroundTool)
        .build();

    // No task_registry wired — should fail.
    let context = ToolContext::with_stores(session_id, file_store, storage_store)
        .with_tool_registry(Arc::new(tool_registry))
        .with_nested_tool_policy(allow_all_policy());

    let result = SpawnBackgroundTool
        .execute_with_context(
            json!({
                "tool": "test_background",
                "args": {}
            }),
            &context,
        )
        .await;

    let ToolExecutionResult::ToolError(message) = result else {
        panic!("spawn_background should reject missing task registry");
    };
    assert!(message.contains("Session task registry not available"));
}

#[tokio::test]
async fn test_spawn_background_requires_file_store() {
    let session_id = SessionId::new();
    let storage_store = Arc::new(NoopStorageStore);
    let task_registry = Arc::new(InMemoryTaskRegistry::default());
    let tool_registry = ToolRegistry::builder()
        .tool(SpawnBackgroundTool)
        .tool(TestBackgroundTool)
        .build();

    // No file_store wired — should fail.
    let context = ToolContext::with_storage_store(session_id, storage_store)
        .with_tool_registry(Arc::new(tool_registry))
        .with_nested_tool_policy(allow_all_policy())
        .with_session_task_registry(task_registry);

    let result = SpawnBackgroundTool
        .execute_with_context(
            json!({
                "tool": "test_background",
                "args": {}
            }),
            &context,
        )
        .await;

    let ToolExecutionResult::ToolError(message) = result else {
        panic!("spawn_background should reject missing file store");
    };
    assert!(message.contains("Session file store not available"));
}

#[tokio::test]
async fn test_spawn_background_caps_output_log_size() {
    let session_id = SessionId::new();
    let file_store = Arc::new(TestFileStore::default());
    let storage_store = Arc::new(NoopStorageStore);
    let task_registry = Arc::new(InMemoryTaskRegistry::default());
    let tool_registry = ToolRegistry::builder()
        .tool(SpawnBackgroundTool)
        .tool(TestLargeOutputBackgroundTool)
        .build();

    let context = ToolContext::with_stores(session_id, file_store.clone(), storage_store)
        .with_tool_registry(Arc::new(tool_registry))
        .with_nested_tool_policy(allow_all_policy())
        .with_session_task_registry(task_registry.clone());

    let result = SpawnBackgroundTool
        .execute_with_context(
            json!({
                "tool": "test_background_large_output",
                "args": {}
            }),
            &context,
        )
        .await;

    let ToolExecutionResult::Success(value) = result else {
        panic!("spawn_background should succeed");
    };
    let run_id = value["run_id"].as_str().unwrap().to_string();
    let task_id = value["task_id"].as_str().unwrap().to_string();

    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            if let Ok(Some(task)) = task_registry.get(session_id, &task_id).await
                && task.state == everruns_core::session_task::SessionTaskState::Succeeded
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("background run should complete");
    let _ = run_id;

    let log_content = file_store
        .read_file(session_id, &format!("/.background/{run_id}/output.log"))
        .await
        .unwrap()
        .expect("log file")
        .content
        .unwrap_or_default();

    assert!(log_content.contains("[system] background output truncated"));
    assert!(log_content.chars().count() <= MAX_BACKGROUND_OUTPUT_LOG_CHARS + 128);
}

#[tokio::test]
async fn test_spawn_background_can_create_scheduled_monitor() {
    let session_id = SessionId::new();
    let schedule_store = Arc::new(TestScheduleStore::default());
    let storage_store = Arc::new(NoopStorageStore);
    let tool_registry = ToolRegistry::builder()
        .tool(SpawnBackgroundTool)
        .tool(TestBackgroundTool)
        .build();

    let context = ToolContext::with_storage_store(session_id, storage_store)
        .with_tool_registry(Arc::new(tool_registry))
        .with_nested_tool_policy(allow_all_policy())
        .with_schedule_store(schedule_store.clone());

    let result = SpawnBackgroundTool
        .execute_with_context(
            json!({
                "tool": "test_background",
                "title": "Watch PR 1319",
                "args": { "summary": "Background complete" },
                "schedule": {
                    "cron_expression": "*/10 * * * *",
                    "timezone": "America/Chicago"
                }
            }),
            &context,
        )
        .await;

    let ToolExecutionResult::Success(value) = result else {
        panic!("spawn_background should create a schedule: {result:?}");
    };

    assert_eq!(value["status"], "scheduled");
    assert_eq!(value["title"], "Watch PR 1319");
    assert_eq!(value["cron_expression"], "*/10 * * * *");
    assert_eq!(value["timezone"], "America/Chicago");

    let schedules = schedule_store.list_schedules(session_id).await.unwrap();
    assert_eq!(schedules.len(), 1);
    assert_eq!(
        schedules[0].cron_expression.as_deref(),
        Some("*/10 * * * *")
    );
    assert!(schedules[0].description.contains("Monitor: Watch PR 1319"));
    assert!(
        schedules[0]
            .description
            .contains("\"summary\": \"Background complete\"")
    );
}

#[tokio::test]
async fn test_spawn_background_rejects_invalid_scheduled_at() {
    let session_id = SessionId::new();
    let storage_store = Arc::new(NoopStorageStore);
    let tool_registry = ToolRegistry::builder()
        .tool(SpawnBackgroundTool)
        .tool(TestBackgroundTool)
        .build();
    let context = ToolContext::with_storage_store(session_id, storage_store)
        .with_tool_registry(Arc::new(tool_registry));

    let result = SpawnBackgroundTool
        .execute_with_context(
            json!({
                "tool": "test_background",
                "args": {},
                "schedule": {
                    "scheduled_at": "tomorrow at noon"
                }
            }),
            &context,
        )
        .await;

    let ToolExecutionResult::ToolError(message) = result else {
        panic!("spawn_background should reject invalid scheduled_at");
    };
    assert!(message.contains("scheduled_at must be RFC3339"));
}

#[tokio::test]
async fn test_spawn_background_rejects_ambiguous_schedule_shape() {
    let session_id = SessionId::new();
    let storage_store = Arc::new(NoopStorageStore);
    let tool_registry = ToolRegistry::builder()
        .tool(SpawnBackgroundTool)
        .tool(TestBackgroundTool)
        .build();
    let context = ToolContext::with_storage_store(session_id, storage_store)
        .with_tool_registry(Arc::new(tool_registry));

    let result = SpawnBackgroundTool
        .execute_with_context(
            json!({
                "tool": "test_background",
                "args": {},
                "schedule": {
                    "cron_expression": "*/10 * * * *",
                    "scheduled_at": "2026-04-16T15:30:00Z"
                }
            }),
            &context,
        )
        .await;

    let ToolExecutionResult::ToolError(message) = result else {
        panic!("spawn_background should reject ambiguous schedule shape");
    };
    assert!(message.contains("must not include both cron_expression and scheduled_at"));
}

/// End-to-end cancel test: spawn a long-sleeping background tool, then call
/// `request_cancel` on the task registry, and assert the task ends Canceled.
#[tokio::test]
async fn test_cancel_background_run_via_task_registry() {
    let session_id = SessionId::new();
    let file_store = Arc::new(TestFileStore::default());
    let storage_store = Arc::new(NoopStorageStore);
    let task_registry = Arc::new(InMemoryTaskRegistry::default());

    let tool_registry = ToolRegistry::builder()
        .tool(SpawnBackgroundTool)
        .tool(SleepingBackgroundTool)
        .build();

    let context = ToolContext::with_stores(session_id, file_store.clone(), storage_store)
        .with_tool_registry(Arc::new(tool_registry))
        .with_nested_tool_policy(allow_all_policy())
        .with_session_task_registry(task_registry.clone());

    let result = SpawnBackgroundTool
        .execute_with_context(
            json!({
                "tool": "test_background_sleeping",
                "args": {},
                "signal_on_completion": false
            }),
            &context,
        )
        .await;

    let ToolExecutionResult::Success(value) = result else {
        panic!("spawn_background should succeed");
    };
    let run_id = value["run_id"].as_str().unwrap().to_string();
    let task_id = value["task_id"].as_str().unwrap().to_string();

    // Wait until the background task is Running (heartbeat loop started).
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            // Wait for a heartbeat to confirm the watcher is live.
            if let Ok(Some(task)) = task_registry.get(session_id, &task_id).await
                && task.heartbeat_at.is_some()
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("background run should start and send at least one heartbeat");

    // Request cancel.
    task_registry
        .request_cancel(session_id, &task_id)
        .await
        .expect("request_cancel should succeed");

    // Wait for the task to reach Canceled state.
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if let Ok(Some(task)) = task_registry.get(session_id, &task_id).await
                && task.state == everruns_core::session_task::SessionTaskState::Canceled
            {
                break task;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("background task should reach Canceled state");

    // Verify result.json and output.log were written.
    let result_file = file_store
        .read_file(session_id, &format!("/.background/{run_id}/result.json"))
        .await
        .unwrap()
        .expect("result.json should exist");
    let result_json: Value =
        serde_json::from_str(result_file.content.as_deref().unwrap_or_default())
            .expect("valid json");
    assert_eq!(result_json["status"], "canceled");

    let log_file = file_store
        .read_file(session_id, &format!("/.background/{run_id}/output.log"))
        .await
        .unwrap()
        .expect("output.log should exist");
    assert!(
        log_file
            .content
            .as_deref()
            .unwrap_or_default()
            .contains("Canceled by request.")
    );
}

#[tokio::test]
async fn reattach_fails_with_missing_file_store() {
    let session_id = SessionId::new();
    // Context with no file_store — only session_task_registry is wired.
    let task_registry = Arc::new(InMemoryTaskRegistry::default());
    let context = everruns_core::tool_context::ToolContext::new(session_id)
        .with_session_task_registry(task_registry);
    let task = make_reattach_task(serde_json::json!({
        "tool": "get_current_time",
        "arguments": {},
        "reattachable": true,
        "signal_on_completion": true,
    }));
    let err = reattach_background_run(&task, &context)
        .await
        .expect_err("should fail without file store");
    assert!(
        err.to_string().contains("file store"),
        "error should mention file store, got: {err}"
    );
}

#[tokio::test]
async fn reattach_fails_with_missing_task_registry() {
    let session_id = SessionId::new();
    let file_store = Arc::new(TestFileStore::default());
    let storage_store = Arc::new(NoopStorageStore);
    // Context has a file_store but no session_task_registry.
    let context = everruns_core::tool_context::ToolContext::with_stores(
        session_id,
        file_store,
        storage_store,
    );
    let task = make_reattach_task(serde_json::json!({
        "tool": "get_current_time",
        "arguments": {},
        "reattachable": true,
        "signal_on_completion": true,
    }));
    let err = reattach_background_run(&task, &context)
        .await
        .expect_err("should fail without task registry");
    assert!(
        err.to_string().contains("task registry"),
        "error should mention task registry, got: {err}"
    );
}

#[tokio::test]
async fn reattach_fails_with_unknown_tool_name() {
    let session_id = SessionId::new();
    let file_store = Arc::new(TestFileStore::default());
    let storage_store = Arc::new(NoopStorageStore);
    let task_registry = Arc::new(InMemoryTaskRegistry::default());
    let context = everruns_core::tool_context::ToolContext::with_stores(
        session_id,
        file_store,
        storage_store,
    )
    .with_session_task_registry(task_registry);
    // "test_background" is not in ToolRegistry::with_defaults().
    let task = make_reattach_task(serde_json::json!({
        "tool": "test_background",
        "arguments": {},
        "reattachable": true,
        "signal_on_completion": true,
    }));
    let err = reattach_background_run(&task, &context)
        .await
        .expect_err("should fail for unknown tool");
    assert!(
        err.to_string().contains("not found in built-in registry"),
        "error should mention built-in registry, got: {err}"
    );
}

#[tokio::test]
async fn reattach_fails_with_missing_tool_spec_field() {
    let session_id = SessionId::new();
    let file_store = Arc::new(TestFileStore::default());
    let storage_store = Arc::new(NoopStorageStore);
    let task_registry = Arc::new(InMemoryTaskRegistry::default());
    let context = everruns_core::tool_context::ToolContext::with_stores(
        session_id,
        file_store,
        storage_store,
    )
    .with_session_task_registry(task_registry);
    // Spec has no "tool" field.
    let task = make_reattach_task(serde_json::json!({ "reattachable": true }));
    let err = reattach_background_run(&task, &context)
        .await
        .expect_err("should fail with missing tool field");
    assert!(
        err.to_string().contains("missing 'tool' field"),
        "error should mention missing tool field, got: {err}"
    );
}

#[test]
fn test_is_canceled_outcome_detects_sentinel() {
    // The sentinel produced by the cancel-watcher branch.
    let sentinel: std::result::Result<BackgroundOutcome, ToolExecutionResult> = Err(
        ToolExecutionResult::ToolError(BACKGROUND_CANCEL_SENTINEL.to_string()),
    );
    assert!(is_canceled_outcome(&sentinel));
}

#[test]
fn test_is_canceled_outcome_does_not_match_other_errors() {
    let other_err: std::result::Result<BackgroundOutcome, ToolExecutionResult> =
        Err(ToolExecutionResult::ToolError("boom".to_string()));
    assert!(!is_canceled_outcome(&other_err));

    let success: std::result::Result<BackgroundOutcome, ToolExecutionResult> =
        Ok(BackgroundOutcome {
            summary: "ok".to_string(),
            result: json!({"ok": true}),
            raw_output: None,
        });
    assert!(!is_canceled_outcome(&success));
}

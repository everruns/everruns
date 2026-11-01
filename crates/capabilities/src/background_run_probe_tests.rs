//! `spawn_background` probe monitors: a scheduled `web_fetch` with a
//! `wake_when` that the scheduler runs itself.

use super::test_support::*;
use super::*;
use async_trait::async_trait;
use everruns_core::session_services::SessionScheduleStore;

/// Stands in for `web_fetch`: no background support, like the real tool.
struct FakeWebFetch;

#[async_trait]
impl Tool for FakeWebFetch {
    fn name(&self) -> &str {
        "web_fetch"
    }

    fn description(&self) -> &str {
        "Fetch a URL"
    }

    fn parameters_schema(&self) -> Value {
        json!({"type": "object", "properties": {"url": {"type": "string"}, "save_to_file": {"type": "string"}}})
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::success(json!({"status_code": 200}))
    }
}

struct Fixture {
    context: ToolContext,
    schedules: Arc<TestScheduleStore>,
    tasks: Arc<InMemoryTaskRegistry>,
}

fn fixture() -> Fixture {
    let session_id = SessionId::new();
    let schedules = Arc::new(TestScheduleStore::default());
    let tasks = Arc::new(InMemoryTaskRegistry::default());
    let registry = ToolRegistry::builder()
        .tool(SpawnBackgroundTool)
        .tool(FakeWebFetch)
        .build();
    let context = ToolContext::with_storage_store(session_id, Arc::new(NoopStorageStore))
        .with_tool_registry(Arc::new(registry))
        .with_nested_tool_policy(allow_all_policy())
        .with_schedule_store(schedules.clone())
        .with_session_task_registry(tasks.clone());
    Fixture {
        context,
        schedules,
        tasks,
    }
}

fn call(args: Value, schedule: Value) -> Value {
    json!({
        "tool": "web_fetch",
        "title": "Claim approval",
        "args": args,
        "schedule": schedule,
    })
}

fn poll_schedule(wake_when: Option<Value>) -> Value {
    let mut schedule = json!({
        "cron_expression": "*/5 * * * *",
        "max_runs": 96,
        "stop_when": "the claim is approved or expired"
    });
    if let Some(wake_when) = wake_when {
        schedule["wake_when"] = wake_when;
    }
    schedule
}

fn error_text(result: ToolExecutionResult) -> String {
    match result {
        ToolExecutionResult::ToolError(message) => message,
        other => panic!("expected a tool error, got {other:?}"),
    }
}

#[tokio::test]
async fn a_scheduled_web_fetch_with_a_wake_condition_becomes_a_probe_monitor() {
    let f = fixture();
    let wake_when = json!({"path": "/json/error", "not_equals": "authorization_pending"});
    let result = SpawnBackgroundTool
        .execute_with_context(
            call(
                json!({"url": "https://claims.example/token", "method": "POST"}),
                poll_schedule(Some(wake_when.clone())),
            ),
            &f.context,
        )
        .await;
    let ToolExecutionResult::Success(value) = result else {
        panic!("expected a scheduled probe monitor: {result:?}");
    };
    assert_eq!(value["status"], "scheduled");

    let schedules = f
        .schedules
        .list_schedules(f.context.session_id)
        .await
        .unwrap();
    assert!(
        schedules[0]
            .description
            .contains("wakes you once when /json/error is no longer \"authorization_pending\""),
        "{}",
        schedules[0].description
    );
    let tasks = f.tasks.tasks.lock().unwrap();
    let task = tasks.values().next().expect("a monitor task");
    assert_eq!(task.spec["wake_when"], wake_when);
    assert_eq!(task.spec["tool"], "web_fetch");
}

#[tokio::test]
async fn a_scheduled_web_fetch_needs_a_wake_condition() {
    let f = fixture();
    let result = SpawnBackgroundTool
        .execute_with_context(
            call(json!({"url": "https://a.example"}), poll_schedule(None)),
            &f.context,
        )
        .await;
    assert!(error_text(result).contains("needs schedule.wake_when"));
}

#[tokio::test]
async fn web_fetch_still_cannot_run_in_the_background_without_a_schedule() {
    let f = fixture();
    let result = SpawnBackgroundTool
        .execute_with_context(
            json!({"tool": "web_fetch", "args": {"url": "https://a.example"}}),
            &f.context,
        )
        .await;
    assert!(error_text(result).contains("does not support background execution"));
}

#[tokio::test]
async fn bad_wake_conditions_and_file_saves_are_refused() {
    let f = fixture();
    let invalid = SpawnBackgroundTool
        .execute_with_context(
            call(
                json!({"url": "https://a.example"}),
                poll_schedule(Some(json!({"path": "/status_code"}))),
            ),
            &f.context,
        )
        .await;
    assert!(error_text(invalid).contains("exactly one of"));

    let saving = SpawnBackgroundTool
        .execute_with_context(
            call(
                json!({"url": "https://a.example", "save_to_file": "/out.bin"}),
                poll_schedule(Some(json!({"changed": true}))),
            ),
            &f.context,
        )
        .await;
    assert!(error_text(saving).contains("cannot save files"));
    assert!(
        f.schedules
            .list_schedules(f.context.session_id)
            .await
            .unwrap()
            .is_empty()
    );
}

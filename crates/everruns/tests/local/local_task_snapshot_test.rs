//! Exercise polling through the public tool and the production SQLite registry.

use std::sync::Arc;

use everruns::local::{LocalSessionTaskRegistry, SqliteDb};
use everruns_capabilities::capabilities::session_tasks::ListTasksTool;
use everruns_contracts::typed_id::SessionId;
use everruns_core::session_task::{
    CreateSessionTask, SessionTaskRegistry, SessionTaskState, SessionTaskUpdate,
    TASK_KIND_BACKGROUND_TOOL, TaskInputRequest, TaskLinks, TaskWakePolicy,
};
use everruns_core::tool_context::ToolContext;
use everruns_core::tools::{Tool, ToolExecutionResult};
use serde_json::{Value, json};

async fn poll(context: &ToolContext, arguments: Value) -> Value {
    match ListTasksTool.execute_with_context(arguments, context).await {
        ToolExecutionResult::Success(value) => value,
        result => panic!("expected task list: {result:?}"),
    }
}

#[tokio::test]
async fn snapshots_survive_sqlite_reopen_and_preserve_input_requests() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tasks.db");
    let session_id = SessionId::new();
    let (task_id, initial) = {
        let registry =
            Arc::new(LocalSessionTaskRegistry::new(SqliteDb::open(&path).unwrap()).unwrap());
        let task = registry
            .create(CreateSessionTask {
                session_id,
                id: None,
                kind: TASK_KIND_BACKGROUND_TOOL.to_string(),
                display_name: "Polling smoke task".to_string(),
                spec: json!({}),
                state: SessionTaskState::Running,
                links: TaskLinks::default(),
                wake_policy: TaskWakePolicy::Silent,
            })
            .await
            .unwrap();
        let context = ToolContext::new(session_id).with_session_task_registry(registry);
        (task.id, poll(&context, json!({})).await)
    };

    let registry = Arc::new(LocalSessionTaskRegistry::new(SqliteDb::open(&path).unwrap()).unwrap());
    let context = ToolContext::new(session_id).with_session_task_registry(registry.clone());
    registry
        .update(
            session_id,
            &task_id,
            SessionTaskUpdate {
                heartbeat_at: Some(chrono::Utc::now()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let unchanged = poll(&context, json!({"since": initial["snapshot"]})).await;
    assert_eq!(unchanged["unchanged"], true);
    assert_eq!(unchanged["snapshot"], initial["snapshot"]);
    assert!(unchanged.get("tasks").is_none());
    println!("SQLite initial: {initial}\nSQLite reopened + heartbeat: {unchanged}");

    registry
        .update(
            session_id,
            &task_id,
            SessionTaskUpdate {
                input_request: Some(TaskInputRequest {
                    id: "choose_destination".to_string(),
                    prompt: "Which destination?".to_string(),
                    expected: None,
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let changed = poll(&context, json!({"since": initial["snapshot"]})).await;
    assert_ne!(changed["snapshot"], initial["snapshot"]);
    assert_eq!(changed["tasks"][0]["state"], "awaiting_input");
    assert_eq!(
        changed["tasks"][0]["input_request"]["id"],
        "choose_destination"
    );
    assert_eq!(poll(&context, json!({})).await, changed);
    println!("SQLite input request: {changed}");

    let other = ToolContext::new(SessionId::new()).with_session_task_registry(registry);
    let isolated = poll(&other, json!({"since": changed["snapshot"]})).await;
    assert_eq!(isolated["tasks"], json!([]));
    assert_ne!(isolated["snapshot"], changed["snapshot"]);
}

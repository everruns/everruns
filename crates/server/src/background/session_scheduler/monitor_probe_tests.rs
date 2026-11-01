//! Probe monitors with a wake condition, fired through the scheduler.

use super::*;
use crate::background::monitor_probe::monitor_probe_tool_registry;
use async_trait::async_trait;
use everruns_contracts::typed_id::ScheduleId;
use everruns_core::DEFAULT_ORG_ID;
use everruns_core::ToolExecutionResult;
use everruns_core::network_access::NetworkAccessList;
use everruns_core::session_task::{
    CreateSessionTask, SessionTaskRegistry, TaskLinks, TaskMessagePart, TaskWakePolicy,
};
use everruns_core::tools::Tool;
use serde_json::{Value, json};
use std::sync::Mutex;

/// Stands in for `web_fetch`, answering each call with the next scripted body.
struct ScriptedFetch(Mutex<Vec<&'static str>>);

#[async_trait]
impl Tool for ScriptedFetch {
    fn name(&self) -> &str {
        "web_fetch"
    }
    fn description(&self) -> &str {
        "scripted"
    }
    fn parameters_schema(&self) -> Value {
        json!({"type": "object"})
    }
    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        let body = self.0.lock().unwrap().remove(0);
        ToolExecutionResult::success(json!({"status_code": 400, "body": body}))
    }
}

async fn probe_monitor(
    db: &Arc<StorageBackend>,
    session_id: SessionId,
    schedule_id: ScheduleId,
    url: &str,
) -> everruns_core::SessionTask {
    DbSessionTaskRegistry::new(db.clone())
        .create(CreateSessionTask {
            session_id,
            id: None,
            kind: TASK_KIND_MONITOR.to_string(),
            display_name: "Claim approval".to_string(),
            spec: json!({
                "schedule_id": schedule_id.to_string(),
                "tool": "web_fetch",
                "arguments": {"url": url, "method": "POST"},
                "wake_when": {"path": "/json/error", "not_equals": "authorization_pending"},
            }),
            state: SessionTaskState::Running,
            links: TaskLinks::default(),
            wake_policy: TaskWakePolicy::Silent,
        })
        .await
        .unwrap()
}

async fn fire(
    db: &Arc<StorageBackend>,
    host: &ProbeHost,
    session_id: SessionId,
    schedule_id: ScheduleId,
) -> MonitorFire {
    fire_monitor_tasks(
        &DbSessionTaskRegistry::new(db.clone()),
        DEFAULT_ORG_ID,
        session_id,
        schedule_id,
        false,
        Some(host),
    )
    .await
}

async fn thread(
    db: &Arc<StorageBackend>,
    session_id: SessionId,
    task: &everruns_core::SessionTask,
) -> Vec<String> {
    let mut messages = DbSessionTaskRegistry::new(db.clone())
        .list_messages(session_id, &task.id, Some(20), None)
        .await
        .unwrap();
    messages.sort_by_key(|message| message.created_at);
    messages
        .iter()
        .map(|message| {
            message
                .content
                .iter()
                .filter_map(|part| match part {
                    TaskMessagePart::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<String>()
        })
        .collect()
}

#[tokio::test]
async fn a_poll_wakes_the_agent_once_when_the_pending_status_ends() {
    let db = Arc::new(StorageBackend::test_database());
    let session_id = super::tests::harnessed_session(&db).await;
    let schedule_id = ScheduleId::new();
    let task = probe_monitor(&db, session_id, schedule_id, "https://claims.example/token").await;
    let mut tools = everruns_core::tools::ToolRegistry::new();
    tools.register(ScriptedFetch(Mutex::new(vec![
        r#"{"error": "authorization_pending"}"#,
        r#"{"error": "authorization_pending"}"#,
        r#"{"assertion": "eyJ.approved"}"#,
        r#"{"assertion": "eyJ.approved"}"#,
    ])));
    let host = ProbeHost::with_tools(db.clone(), tools, None);

    for _ in 0..2 {
        let fired = fire(&db, &host, session_id, schedule_id).await;
        assert!(fired.probe_ran);
        assert!(fired.wake.is_empty(), "still pending: no turn");
    }
    let fired = fire(&db, &host, session_id, schedule_id).await;
    assert_eq!(fired.wake.len(), 1, "approved: one turn");
    assert!(fired.wake[0].contains("eyJ.approved"), "{}", fired.wake[0]);
    let fired = fire(&db, &host, session_id, schedule_id).await;
    assert!(fired.probe_ran && fired.wake.is_empty(), "no second turn");

    let messages = thread(&db, session_id, &task).await;
    assert_eq!(messages.len(), 4);
    assert!(
        messages[0].contains("waiting until /json/error is no longer"),
        "{}",
        messages[0]
    );
    assert!(messages[2].contains("Waking the agent"), "{}", messages[2]);
}

#[tokio::test]
async fn a_probe_runs_under_the_sessions_network_access_list() {
    let db = Arc::new(StorageBackend::test_database());
    let session_id = super::tests::harnessed_session(&db).await;
    sqlx::query("UPDATE sessions SET network_access = $1 WHERE id = $2")
        .bind(serde_json::to_value(NetworkAccessList::allow_only(["allowed.example"])).unwrap())
        .bind(session_id.uuid())
        .execute(db.database().pool())
        .await
        .unwrap();
    let schedule_id = ScheduleId::new();
    probe_monitor(
        &db,
        session_id,
        schedule_id,
        "https://blocked.example/token",
    )
    .await;
    let host = ProbeHost::with_tools(db.clone(), monitor_probe_tool_registry(), None);

    // The real web_fetch refuses the URL before any request; the refusal is
    // what the condition sees, so the agent hears about it once.
    let fired = fire(&db, &host, session_id, schedule_id).await;
    assert_eq!(fired.wake.len(), 1);
    assert!(
        fired.wake[0].contains("blocked by network access policy"),
        "{}",
        fired.wake[0]
    );
}

//! Durable `ask_user` pause: whether a turn parks for an answer or completes
//! the question unattended, decided from the session hint.
//!
//! Lives beside `runtime_host_test` rather than inside it — that file is on the
//! source-file size debt list and may not grow — and borrows its fixtures.

use std::collections::HashMap;

use everruns_core::execution_loading::SessionStore;
use everruns_core::{EventData, SessionExecutionState};
use everruns_engine::TurnPlan;
use everruns_provider::typed_id::{HarnessId, SessionId};
use serde_json::json;
use uuid::Uuid;

use super::runtime_host_test::{advance_from_state, mock_host, session, turn_state};

#[tokio::test]
async fn durable_ask_user_pause_honors_the_session_hint() {
    let adapter = mock_host();
    let harness_id = HarnessId::from_uuid(Uuid::now_v7());
    let session_id = SessionId::from_uuid(Uuid::now_v7());
    let mut host_session = session(session_id, harness_id);
    host_session.hints = Some(HashMap::from([("ask_user".to_string(), json!(true))]));
    adapter.session_store.insert(host_session).await;

    let input = turn_state(session_id, harness_id);
    let output = json!({
        "waiting_for_tool_results": true,
        "client_tool_calls": [{
            "id": "toolu_ask_1",
            "name": everruns_provider::ASK_USER_TOOL_NAME,
            "arguments": {"questions": []}
        }]
    });

    let plan = advance_from_state(&adapter, "act", &input, &output, 0)
        .await
        .unwrap();

    assert!(matches!(plan, TurnPlan::WaitForToolResults { .. }));
    let stored = adapter
        .session_store
        .get_session(session_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.status, SessionExecutionState::WaitingForToolResults);
}

#[tokio::test]
async fn durable_ask_user_pause_without_hint_emits_unattended_completion() {
    let adapter = mock_host();
    let harness_id = HarnessId::from_uuid(Uuid::now_v7());
    let session_id = SessionId::from_uuid(Uuid::now_v7());
    adapter
        .session_store
        .insert(session(session_id, harness_id))
        .await;

    let input = turn_state(session_id, harness_id);
    let output = json!({
        "waiting_for_tool_results": true,
        "client_tool_calls": [{
            "id": "toolu_ask_1",
            "name": everruns_provider::ASK_USER_TOOL_NAME,
            "arguments": {"questions": []}
        }]
    });

    let plan = advance_from_state(&adapter, "act", &input, &output, 0)
        .await
        .unwrap();

    assert!(matches!(plan, TurnPlan::ScheduleReason(_)));
    let completion = adapter
        .event_emitter
        .events()
        .await
        .into_iter()
        .find_map(|event| match event.data {
            EventData::ToolCompleted(data) if data.tool_call_id == "toolu_ask_1" => Some(data),
            _ => None,
        })
        .expect("unattended ask_user completion emitted");
    assert_eq!(completion.tool_name, everruns_provider::ASK_USER_TOOL_NAME);
}

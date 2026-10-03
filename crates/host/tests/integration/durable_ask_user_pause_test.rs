//! Durable `ask_user` pause: whether a turn parks for an answer or completes
//! the question unattended, decided from session and current-message hints.
//!
//! Lives beside `runtime_host_test` rather than inside it — that file is on the
//! source-file size debt list and may not grow — and borrows its fixtures.

use std::collections::HashMap;

use everruns_contracts::typed_id::{HarnessId, SessionId};
use everruns_core::execution_loading::SessionStore;
use everruns_core::{Controls, EventData, InputMessage, SessionExecutionState};
use everruns_engine::TurnPlan;
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
            "name": everruns_contracts::ASK_USER_TOOL_NAME,
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
async fn durable_ask_user_pause_honors_current_message_overrides() {
    for (session_hint, message_hint, should_wait) in [
        (None, true, true),
        (Some(false), true, true),
        (Some(true), false, false),
    ] {
        let adapter = mock_host();
        let harness_id = HarnessId::from_uuid(Uuid::now_v7());
        let session_id = SessionId::from_uuid(Uuid::now_v7());
        let mut host_session = session(session_id, harness_id);
        host_session.hints =
            session_hint.map(|enabled| HashMap::from([("ask_user".to_string(), json!(enabled))]));
        adapter.session_store.insert(host_session).await;

        let mut input = turn_state(session_id, harness_id);
        let mut message = InputMessage::user("Ask me which guide to use");
        message.controls = Some(Controls {
            hints: Some(HashMap::from([(
                "ask_user".to_string(),
                json!(message_hint),
            )])),
            ..Default::default()
        });
        let stored = adapter
            .message_store
            .add(session_id, message)
            .await
            .unwrap();
        input.input_message_id = stored.id;
        // A later queued input must not change the active turn's capability.
        let mut later = InputMessage::user("Queued follow-up from another client");
        later.controls = Some(Controls {
            hints: Some(HashMap::from([(
                "ask_user".to_string(),
                json!(!message_hint),
            )])),
            ..Default::default()
        });
        adapter.message_store.add(session_id, later).await.unwrap();
        let output = json!({
            "waiting_for_tool_results": true,
            "client_tool_calls": [{
                "id": "toolu_ask_1",
                "name": everruns_contracts::ASK_USER_TOOL_NAME,
                "arguments": {"questions": []}
            }]
        });

        let plan = advance_from_state(&adapter, "act", &input, &output, 0)
            .await
            .unwrap();
        assert_eq!(
            matches!(plan, TurnPlan::WaitForToolResults { .. }),
            should_wait
        );
    }
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
            "name": everruns_contracts::ASK_USER_TOOL_NAME,
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
    assert_eq!(completion.tool_name, everruns_contracts::ASK_USER_TOOL_NAME);
}

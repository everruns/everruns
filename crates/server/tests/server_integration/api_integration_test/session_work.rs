//! API integration tests: session work reports (`GET /v1/sessions/{id}/work`).

use crate::test_harness;
use axum::http::StatusCode;
use everruns_server::domains::agents::record::Agent;
use everruns_server::domains::sessions::record::Session;
use serde_json::{Value, json};
use test_harness::TestServer;

#[tokio::test]
async fn test_session_work_report_reads_the_latest_turn() {
    use everruns_contracts::tool_types::ToolCall;
    use everruns_contracts::typed_id::{MessageId, TurnId};
    use everruns_core::events::{
        EventContext, EventRequest, FileWrittenData, ToolCompletedData, ToolStartedData,
        TurnCompletedData, TurnStartedData,
    };

    let server = TestServer::new().await;
    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({ "name": "work-report-agent", "system_prompt": "Test" }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let session: Session = server
        .post(
            "/v1/sessions",
            json!({ "harness_id": server.seed_base_harness_id, "agent_id": agent.public_id }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let path = format!("/v1/sessions/{}/work", session.id);

    // A session that never ran a turn has no report.
    let empty: Value = server.get(&path).await.assert_status(StatusCode::OK).json();
    assert!(empty["report"].is_null());

    let turn_id = TurnId::new();
    let context = EventContext::turn(turn_id, MessageId::new());
    let emit = |data: everruns_core::events::EventData| {
        let request = EventRequest::new(session.id, context.clone(), data);
        let events = server.event_service.clone();
        async move { events.emit(request).await.expect("emit") }
    };
    emit(
        TurnStartedData {
            turn_id,
            input_message_id: context.input_message_id.unwrap(),
            input_content: None,
            agent_id: None,
            agent_name: Some("Work Report Agent".into()),
            agent_description: None,
        }
        .into(),
    )
    .await;
    emit(
        ToolStartedData {
            tool_call: ToolCall {
                id: "call_1".into(),
                name: "write_file".into(),
                arguments: json!({ "path": "/notes.md" }),
            },
            tool_call_fingerprint: None,
            display_name: None,
            narration: Some("Write notes.md".into()),
            side_effect_class: None,
        }
        .into(),
    )
    .await;

    let working: Value = server.get(&path).await.assert_status(StatusCode::OK).json();
    assert_eq!(working["report"]["state"], "working");
    assert_eq!(working["report"]["headline"], "Write notes.md");
    assert_eq!(working["report"]["agent_name"], "Work Report Agent");

    let mut completed =
        ToolCompletedData::success("call_1".into(), "write_file".into(), vec![], Some(20));
    completed.narration = Some("Wrote notes.md".into());
    emit(completed.into()).await;
    emit(
        FileWrittenData {
            path: "/notes.md".into(),
            operation: "create".into(),
            size_bytes: 12,
            created: true,
        }
        .into(),
    )
    .await;
    emit(
        TurnCompletedData {
            turn_id,
            iterations: 2,
            final_answer_preview: Some("Saved the notes. Anything else?".into()),
            ..Default::default()
        }
        .into(),
    )
    .await;

    let done: Value = server.get(&path).await.assert_status(StatusCode::OK).json();
    let report = &done["report"];
    assert_eq!(report["state"], "done", "{report}");
    assert!(
        report["headline"]
            .as_str()
            .unwrap()
            .ends_with("· 1 file changed"),
        "{report}"
    );
    assert_eq!(report["steps"][0]["label"], "Wrote notes.md");
    assert_eq!(report["result"]["summary"], "Saved the notes.");
    assert_eq!(report["result"]["artifacts"][0]["path"], "/notes.md");

    // The same turn by id reads the same; an unknown turn is not found.
    let by_id: Value = server
        .get(&format!("{path}?turn_id={turn_id}"))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(by_id["report"]["turn_id"], turn_id.to_string());
    server
        .get(&format!("{path}?turn_id={}", TurnId::new()))
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

use super::*;
use everruns_contracts::ExecutionPhase;
use everruns_core::events::{
    EventData, OutputMessageCompletedData, SessionIdledData, ToolCompletedData,
};
use everruns_core::{ContentPart, RuntimeMessage};
use everruns_server::storage::{CreateEventRow, EventRow};

async fn emit(server: &TestServer, session_id: &str, data: EventData) -> EventRow {
    server
        .db
        .create_event(CreateEventRow {
            session_id: session_id.parse().unwrap(),
            event_type: data.event_type().into(),
            ts: chrono::Utc::now(),
            context: json!({}),
            data: serde_json::to_value(data).unwrap(),
            metadata: None,
            tags: None,
        })
        .await
        .unwrap()
}

fn output(message: RuntimeMessage) -> EventData {
    OutputMessageCompletedData::new(message).into()
}

fn sent(text: &str) -> EventData {
    everruns_core::conversation::sent_message(
        "send_message",
        "send-1",
        &json!({"text": text}),
        Some(&json!({"sent": true, "message_id": everruns_contracts::typed_id::MessageId::new()})),
    )
    .unwrap()
    .into()
}

async fn session(server: &TestServer) -> String {
    let response = mcp_tool_call(
        server,
        "agent_run",
        json!({"message": "Tell me a dad joke"}),
    )
    .await;
    assert!(!tool_is_error(&response), "{}", tool_text(&response));
    tool_json(&response)["session_id"].as_str().unwrap().into()
}

async fn status(server: &TestServer, args: Value) -> Value {
    let response = mcp_tool_call(server, "session_get_status", args).await;
    assert!(!tool_is_error(&response), "{}", tool_text(&response));
    tool_json(&response)
}

#[tokio::test]
async fn latest_output_finds_sent_reply_beyond_event_page_and_preserves_cursor() {
    let server = TestServer::new().await;
    let session_id = session(&server).await;
    let reply = "I'm reading a book on anti-gravity. It's impossible to put down.";
    let delivery = emit(&server, &session_id, sent(reply)).await;
    assert_eq!(
        status(&server, json!({"session_id": session_id})).await["latest_output"],
        reply
    );
    let mut tool_events = Vec::new();
    for i in 0..55 {
        tool_events.push(
            emit(
                &server,
                &session_id,
                ToolCompletedData::success(format!("tool-{i}"), "search".into(), vec![], None)
                    .into(),
            )
            .await,
        );
    }
    let idle = emit(
        &server,
        &session_id,
        SessionIdledData {
            turn_id: everruns_contracts::typed_id::TurnId::new(),
            iterations: Some(1),
            usage: None,
        }
        .into(),
    )
    .await;
    server
        .db
        .set_session_status(DEFAULT_ORG_ID, session_id.parse().unwrap(), "idle")
        .await
        .unwrap();
    let first = status(&server, json!({"session_id": session_id})).await;
    assert_eq!(first["status"], "idle");
    assert_eq!(first["latest_output"], reply);
    assert_eq!(first["event_count"], 50);
    assert!(
        first["events"]
            .as_array()
            .unwrap()
            .iter()
            .all(|event| event["type"].is_string())
    );
    assert_eq!(first["last_event_id"], first["events"][49]["id"]);
    let page = status(
        &server,
        json!({"session_id": session_id, "since_event_id": delivery.id}),
    )
    .await;
    assert_eq!(page["event_count"], 50);
    assert_eq!(page["events"][0]["id"], tool_events[0].id.to_string());
    assert_eq!(page["last_event_id"], tool_events[49].id.to_string());
    assert_eq!(page["latest_output"], reply);
    let next = status(
        &server,
        json!({"session_id": session_id, "since_event_id": page["last_event_id"]}),
    )
    .await;
    assert_eq!(next["event_count"], 6);
    assert_eq!(next["events"][0]["id"], tool_events[50].id.to_string());
    assert_eq!(next["last_event_id"], idle.id.to_string());
    assert_eq!(next["latest_output"], reply);
    for args in [
        json!({"session_id": session_id, "event_types": ["session.idled"]}),
        json!({"session_id": session_id, "since_event_id": idle.id}),
    ] {
        let result = status(&server, args.clone()).await;
        assert_eq!(result["latest_output"], reply);
        if args.get("since_event_id").is_some() {
            assert_eq!(result["events"], json!([]));
            assert_eq!(result["last_event_id"], Value::Null);
        } else {
            assert_eq!(result["event_count"], 1);
            assert_eq!(result["last_event_id"], idle.id.to_string());
        }
    }
}

#[tokio::test]
async fn latest_output_skips_private_empty_and_undelivered_messages_across_pages() {
    let server = TestServer::new().await;
    let session_id = session(&server).await;
    assert_eq!(
        status(&server, json!({"session_id": session_id})).await["latest_output"],
        Value::Null
    );
    let mut direct = RuntimeMessage::assistant("First part");
    direct.content.push(ContentPart::text("Second part"));
    emit(&server, &session_id, output(direct)).await;
    for _ in 0..55 {
        emit(
            &server,
            &session_id,
            output(
                RuntimeMessage::assistant("Private notes").with_phase(ExecutionPhase::Commentary),
            ),
        )
        .await;
    }
    emit(&server, &session_id, sent("  ")).await;
    emit(&server, &session_id, output(RuntimeMessage::assistant(""))).await;
    let mut unsent = RuntimeMessage::assistant("");
    unsent.content.push(ContentPart::tool_call(
        "failed-send",
        "send_message",
        json!({"text": "Not delivered"}),
    ));
    emit(&server, &session_id, output(unsent)).await;
    emit(
        &server,
        &session_id,
        ToolCompletedData::failure(
            "failed-send".into(),
            "send_message".into(),
            "error".into(),
            "Delivery failed".into(),
            None,
        )
        .into(),
    )
    .await;
    assert_eq!(
        status(&server, json!({"session_id": session_id})).await["latest_output"],
        "First part\nSecond part"
    );
    emit(&server, &session_id, sent("Latest sent reply")).await;
    assert_eq!(
        status(&server, json!({"session_id": session_id})).await["latest_output"],
        "Latest sent reply"
    );
    emit(
        &server,
        &session_id,
        output(RuntimeMessage::assistant("Latest direct reply")),
    )
    .await;
    assert_eq!(
        status(&server, json!({"session_id": session_id})).await["latest_output"],
        "Latest direct reply"
    );
}

#[tokio::test]
async fn latest_output_is_null_when_only_private_messages_exist() {
    let server = TestServer::new().await;
    let session_id = session(&server).await;
    for _ in 0..55 {
        emit(
            &server,
            &session_id,
            output(
                RuntimeMessage::assistant("Private notes").with_phase(ExecutionPhase::Commentary),
            ),
        )
        .await;
    }
    emit(&server, &session_id, sent("  ")).await;
    assert_eq!(
        status(&server, json!({"session_id": session_id})).await["latest_output"],
        Value::Null
    );
}

// ============================================================================
// Tier 1: session_get_status
// ============================================================================

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_session_get_status() {
    let server = TestServer::new().await;

    // Create a session
    let run_resp = mcp_tool_call(&server, "agent_run", json!({ "message": "Status test" })).await;
    assert!(
        !tool_is_error(&run_resp),
        "agent_run failed: {}",
        tool_text(&run_resp)
    );
    let session_id = tool_json(&run_resp)["session_id"]
        .as_str()
        .unwrap()
        .to_string();

    // Get status
    let resp = mcp_tool_call(
        &server,
        "session_get_status",
        json!({ "session_id": session_id }),
    )
    .await;

    assert!(
        !tool_is_error(&resp),
        "session_get_status failed: {}",
        tool_text(&resp)
    );
    let result = tool_json(&resp);
    assert_eq!(result["session_id"], session_id);
    assert!(result["status"].is_string());
    assert!(result["events"].is_array());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_session_get_status_with_event_filter() {
    let server = TestServer::new().await;

    // Create a session
    let run_resp = mcp_tool_call(&server, "agent_run", json!({ "message": "Filter test" })).await;
    assert!(
        !tool_is_error(&run_resp),
        "agent_run failed: {}",
        tool_text(&run_resp)
    );
    let session_id = tool_json(&run_resp)["session_id"]
        .as_str()
        .unwrap()
        .to_string();

    // Get status with event type filter
    let resp = mcp_tool_call(
        &server,
        "session_get_status",
        json!({
            "session_id": session_id,
            "event_types": ["session.idled"]
        }),
    )
    .await;

    assert!(
        !tool_is_error(&resp),
        "session_get_status failed: {}",
        tool_text(&resp)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_session_get_status_missing_session_id() {
    let server = TestServer::new().await;
    let resp = mcp_tool_call(&server, "session_get_status", json!({})).await;
    assert!(tool_is_error(&resp));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_session_get_status_nonexistent() {
    let server = TestServer::new().await;
    let resp = mcp_tool_call(
        &server,
        "session_get_status",
        json!({ "session_id": "session_00000000000000000000000000000000" }),
    )
    .await;
    assert!(
        tool_is_error(&resp),
        "Expected error for nonexistent session"
    );
}

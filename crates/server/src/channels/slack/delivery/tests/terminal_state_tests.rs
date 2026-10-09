use super::*;
use crate::storage::StorageBackend;
use crate::storage::{CreateEventRow, CreateSessionRow};
use everruns_contracts::typed_id::PrincipalId;
use everruns_contracts::typed_id::{AgentId, HarnessId};
use tokio::sync::broadcast;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const ORG: i64 = 30;
const INPUT_MSG: &str = "msg_turn_one";
const FRONTEND: &str = "https://app.example.com";
const CHANNEL: &str = "C_TERMINAL";
const THREAD_TS: &str = "1700000000.000100";

pub(super) async fn seed_session(db: &StorageBackend) -> everruns_contracts::typed_id::SessionId {
    db.create_session(CreateSessionRow {
        playground_user_id: None,
        source: crate::records::SessionSource::Api,
        workspace_id: None,
        org_id: ORG,
        app_id: None,
        channel_id: None,
        trigger_id: None,
        harness_id: Some(HarnessId::from_uuid(uuid::Uuid::nil())),
        agent_id: Some(AgentId::from_uuid(uuid::Uuid::nil())),
        agent_revision: None,
        virtual_user_id: None,
        owner_principal_id: PrincipalId::from_seed(1),
        resolved_owner_user_id: None,
        title: Some("terminal state test".to_string()),
        locale: None,
        tags: vec![],
        model_id: None,
        capabilities: serde_json::json!([]),
        tools: serde_json::json!([]),
        mcp_servers: serde_json::json!({}),
        system_prompt: None,
        initial_files: serde_json::Value::Array(vec![]),
        hints: None,
        max_iterations: None,
        parallel_tool_calls: None,
        blueprint_id: None,
        blueprint_config: None,
        network_access: None,
        parent_session_id: None,
        budget_root_session_id: None,
    })
    .await
    .expect("create session")
    .id
}

pub(super) async fn emit(
    db: &StorageBackend,
    session_id: everruns_contracts::typed_id::SessionId,
    event_type: &str,
    input_message_id: &str,
    data: serde_json::Value,
) {
    db.create_event(CreateEventRow {
        session_id,
        event_type: event_type.to_string(),
        ts: chrono::Utc::now(),
        context: serde_json::json!({ "input_message_id": input_message_id }),
        data,
        metadata: None,
        tags: None,
    })
    .await
    .expect("create event");
}

fn reply_event_data(text: &str) -> serde_json::Value {
    serde_json::json!({
        "message": { "content": [{ "type": "text", "text": text }] }
    })
}

/// Slack mock that accepts every post, plus a dispatcher pointed at it.
async fn dispatcher_against_slack(
    db: Arc<StorageBackend>,
) -> (Arc<SlackDeliveryDispatcher>, MockServer) {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat.postMessage"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({ "ok": true, "ts": "1.2" })),
        )
        .mount(&mock_server)
        .await;

    let (_tx, rx) = broadcast::channel::<EventNotificationPayload>(16);
    let dispatcher = SlackDeliveryDispatcher::start_with_adapter(
        db,
        rx,
        FRONTEND.to_string(),
        Arc::new(SlackDeliveryAdapter::with_api_base(mock_server.uri())),
    );
    (dispatcher, mock_server)
}

/// Text of every message the dispatcher posted, in order.
async fn posted_texts(mock_server: &MockServer) -> Vec<String> {
    mock_server
        .received_requests()
        .await
        .unwrap_or_default()
        .iter()
        .map(|req| {
            let body: serde_json::Value =
                serde_json::from_slice(&req.body).expect("slack post body is json");
            body["text"].as_str().unwrap_or_default().to_string()
        })
        .collect()
}

async fn register_turn(dispatcher: &SlackDeliveryDispatcher, session_id: uuid::Uuid) {
    register_turn_with_mode(dispatcher, session_id, SlackReplyMode::AllMessages).await;
}

async fn register_turn_with_mode(
    dispatcher: &SlackDeliveryDispatcher,
    session_id: uuid::Uuid,
    reply_mode: SlackReplyMode,
) {
    dispatcher
        .register(DeliveryRegistration {
            session_id,
            input_message_id: INPUT_MSG.to_string(),
            bot_token: "xoxb-test-token".to_string(),
            channel: CHANNEL.to_string(),
            thread_ts: THREAD_TS.to_string(),
            reply_mode,
            surface: SlackSurface::Channel,
            recipient_user_id: None,
            recipient_team_id: None,
            tool_visibility: PublicToolVisibility::default(),
            generic_tool_text: crate::records::agent_channel::DEFAULT_AG_UI_GENERIC_TOOL_TEXT
                .to_string(),
            approvals_enabled: true,
        })
        .await;
}

#[tokio::test]
async fn turn_failed_without_reply_posts_one_notice() {
    let db = Arc::new(StorageBackend::test_database());
    let session_id = seed_session(&db).await;
    let (dispatcher, mock_server) = dispatcher_against_slack(db.clone()).await;
    register_turn(&dispatcher, session_id.uuid()).await;

    emit(
        &db,
        session_id,
        "turn.failed",
        INPUT_MSG,
        serde_json::json!({ "error": "provider exploded: secret-bearing detail" }),
    )
    .await;
    dispatcher.process_session_events(session_id.uuid()).await;

    let texts = posted_texts(&mock_server).await;
    assert_eq!(texts.len(), 1, "expected exactly one notice, got {texts:?}");
    assert!(
        texts[0].contains("could not finish"),
        "unexpected notice: {}",
        texts[0]
    );
    assert!(
        texts[0].contains(&format!("{FRONTEND}/sessions/{session_id}/chat")),
        "notice must link back to the session: {}",
        texts[0]
    );
    assert!(
        !texts[0].contains("secret-bearing detail"),
        "notice must not leak internal error text: {}",
        texts[0]
    );
    assert_eq!(
        dispatcher.active_delivery_count().await,
        0,
        "a failed turn must release its registration"
    );
}

#[tokio::test]
async fn turn_completed_without_output_posts_notice() {
    let db = Arc::new(StorageBackend::test_database());
    let session_id = seed_session(&db).await;
    let (dispatcher, mock_server) = dispatcher_against_slack(db.clone()).await;
    register_turn(&dispatcher, session_id.uuid()).await;

    emit(
        &db,
        session_id,
        "turn.completed",
        INPUT_MSG,
        serde_json::json!({}),
    )
    .await;
    dispatcher.process_session_events(session_id.uuid()).await;

    let texts = posted_texts(&mock_server).await;
    assert_eq!(texts.len(), 1, "expected exactly one notice, got {texts:?}");
    assert!(
        texts[0].contains("without a reply"),
        "unexpected notice: {}",
        texts[0]
    );
}

#[tokio::test]
async fn delivered_reply_suppresses_notice() {
    let db = Arc::new(StorageBackend::test_database());
    let session_id = seed_session(&db).await;
    let (dispatcher, mock_server) = dispatcher_against_slack(db.clone()).await;
    register_turn(&dispatcher, session_id.uuid()).await;

    emit(
        &db,
        session_id,
        "output.message.completed",
        INPUT_MSG,
        reply_event_data("Here is your answer."),
    )
    .await;
    emit(
        &db,
        session_id,
        "turn.completed",
        INPUT_MSG,
        serde_json::json!({}),
    )
    .await;
    dispatcher.process_session_events(session_id.uuid()).await;

    let texts = posted_texts(&mock_server).await;
    assert_eq!(texts, vec!["Here is your answer.".to_string()]);
    assert_eq!(dispatcher.active_delivery_count().await, 0);
}

/// The reply and the terminal event usually arrive in separate
/// notifications, so `delivered` has to survive between passes or every
/// answered turn would be chased by a spurious "no reply" notice.
#[tokio::test]
async fn reply_in_earlier_pass_still_suppresses_notice() {
    let db = Arc::new(StorageBackend::test_database());
    let session_id = seed_session(&db).await;
    let (dispatcher, mock_server) = dispatcher_against_slack(db.clone()).await;
    register_turn(&dispatcher, session_id.uuid()).await;

    emit(
        &db,
        session_id,
        "output.message.completed",
        INPUT_MSG,
        reply_event_data("Answered early."),
    )
    .await;
    dispatcher.process_session_events(session_id.uuid()).await;

    emit(
        &db,
        session_id,
        "turn.completed",
        INPUT_MSG,
        serde_json::json!({}),
    )
    .await;
    dispatcher.process_session_events(session_id.uuid()).await;

    let texts = posted_texts(&mock_server).await;
    assert_eq!(texts, vec!["Answered early.".to_string()]);
}

/// Both cancel paths mint a fresh `input_message_id`, so the delivery used
/// to sit registered forever and the user was never told.
#[tokio::test]
async fn turn_cancelled_notifies_and_unregisters() {
    let db = Arc::new(StorageBackend::test_database());
    let session_id = seed_session(&db).await;
    emit(
        &db,
        session_id,
        "input.message",
        INPUT_MSG,
        serde_json::json!({}),
    )
    .await;
    let (dispatcher, mock_server) = dispatcher_against_slack(db.clone()).await;
    register_turn(&dispatcher, session_id.uuid()).await;

    emit(
        &db,
        session_id,
        "turn.cancelled",
        "msg_freshly_minted_by_cancel",
        serde_json::json!({ "reason": "User requested cancellation" }),
    )
    .await;
    dispatcher.process_session_events(session_id.uuid()).await;

    let texts = posted_texts(&mock_server).await;
    assert_eq!(texts.len(), 1, "expected exactly one notice, got {texts:?}");
    assert!(
        texts[0].contains("cancelled"),
        "unexpected notice: {}",
        texts[0]
    );
    assert_eq!(
        dispatcher.active_delivery_count().await,
        0,
        "cancelling must not leak the delivery registration"
    );
}

#[tokio::test]
async fn historical_cancellation_does_not_unregister_a_follow_up() {
    let db = Arc::new(StorageBackend::test_database());
    let session_id = seed_session(&db).await;

    emit(
        &db,
        session_id,
        "turn.cancelled",
        "msg_previous_turn",
        serde_json::json!({ "reason": "User requested cancellation" }),
    )
    .await;
    emit(
        &db,
        session_id,
        "input.message",
        INPUT_MSG,
        serde_json::json!({}),
    )
    .await;

    let (dispatcher, mock_server) = dispatcher_against_slack(db.clone()).await;
    register_turn(&dispatcher, session_id.uuid()).await;
    dispatcher.process_session_events(session_id.uuid()).await;

    assert!(
        posted_texts(&mock_server).await.is_empty(),
        "a cancellation from an earlier turn must not affect the follow-up"
    );
    assert_eq!(dispatcher.active_delivery_count().await, 1);

    emit(
        &db,
        session_id,
        "output.message.completed",
        INPUT_MSG,
        reply_event_data("Follow-up answer."),
    )
    .await;
    emit(
        &db,
        session_id,
        "turn.completed",
        INPUT_MSG,
        serde_json::json!({}),
    )
    .await;
    dispatcher.process_session_events(session_id.uuid()).await;

    assert_eq!(
        posted_texts(&mock_server).await,
        vec!["Follow-up answer.".to_string()]
    );
    assert_eq!(dispatcher.active_delivery_count().await, 0);
}

/// A reply Slack refused is not a delivered reply: the user still needs to
/// be told the turn is over.
#[tokio::test]
async fn failed_delivery_still_yields_notice() {
    let db = Arc::new(StorageBackend::test_database());
    let session_id = seed_session(&db).await;

    // Reject every post with a permanent error so no retry budget burns.
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat.postMessage"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({ "ok": false, "error": "channel_not_found" })),
        )
        .mount(&mock_server)
        .await;

    let (_tx, rx) = broadcast::channel::<EventNotificationPayload>(16);
    let dispatcher = SlackDeliveryDispatcher::start_with_adapter(
        db.clone(),
        rx,
        FRONTEND.to_string(),
        Arc::new(SlackDeliveryAdapter::with_api_base(mock_server.uri())),
    );
    register_turn(&dispatcher, session_id.uuid()).await;

    emit(
        &db,
        session_id,
        "output.message.completed",
        INPUT_MSG,
        reply_event_data("Answer nobody will see."),
    )
    .await;
    emit(
        &db,
        session_id,
        "turn.completed",
        INPUT_MSG,
        serde_json::json!({}),
    )
    .await;
    dispatcher.process_session_events(session_id.uuid()).await;

    let texts = posted_texts(&mock_server).await;
    assert_eq!(
        texts.len(),
        2,
        "the lost reply and then the notice, got {texts:?}"
    );
    assert!(
        texts[1].contains("without a reply"),
        "unexpected notice: {}",
        texts[1]
    );
    assert_eq!(dispatcher.active_delivery_count().await, 0);
}

#[tokio::test]
async fn notice_without_frontend_url_omits_link() {
    let (_tx, rx) = broadcast::channel::<EventNotificationPayload>(16);
    let dispatcher = SlackDeliveryDispatcher::start(
        Arc::new(StorageBackend::test_database()),
        rx,
        String::new(),
    );

    let notice = dispatcher.terminal_notice("turn.failed", uuid::Uuid::nil());
    assert_eq!(notice, "The agent could not finish this request.");
}

#[tokio::test]
async fn explicit_posts_are_not_reposted_and_failures_still_get_a_terminal_notice() {
    for (event_type, expected) in [
        ("turn.completed", None),
        (
            "turn.failed",
            Some("The agent could not finish this request."),
        ),
        ("turn.cancelled", Some("This request was cancelled.")),
    ] {
        let db = Arc::new(StorageBackend::test_database());
        let session = seed_session(&db).await;
        let (dispatcher, slack) = dispatcher_against_slack(db.clone()).await;
        register_turn_with_mode(&dispatcher, session.uuid(), SlackReplyMode::ToolOnly).await;
        emit(&db, session, "tool.completed", INPUT_MSG, serde_json::json!({
            "tool_name":"channel_post_message", "success":true,
            "result":[{"type":"text","text":r#"{"delivered":true,"platform":"slack","channel":"C_TERMINAL","message_ref":"1.2"}"#}]
        })).await;
        emit(
            &db,
            session,
            "output.message.completed",
            INPUT_MSG,
            reply_event_data("Private assistant output"),
        )
        .await;
        emit(&db, session, event_type, INPUT_MSG, serde_json::json!({})).await;
        dispatcher.process_session_events(session.uuid()).await;
        dispatcher.process_session_events(session.uuid()).await;
        let posts = posted_texts(&slack).await;
        match expected {
            None => assert!(posts.is_empty(), "accepted tool post must not be repeated"),
            Some(headline) => {
                assert_eq!(posts.len(), 1);
                assert!(posts[0].starts_with(headline));
            }
        }
        assert_eq!(dispatcher.active_delivery_count().await, 0);
    }
}

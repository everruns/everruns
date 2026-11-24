use super::tests::Fixture;
use super::*;
use crate::storage::CreateEventRow;
use everruns_capabilities::channel_message_sender::SlackConversationSender;
use everruns_contracts::slack_action::SlackActionInvoker;
use everruns_contracts::typed_id::MessageId;
use everruns_core::{
    conversation::{ConversationSenderExt, SendMessageTool},
    events::EventContext,
    tool_context::ToolContext,
    tools::{Tool, ToolExecutionResult},
};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

async fn input(
    fixture: &Fixture,
    session: SessionId,
    app: &str,
    endpoint: &str,
    channel: &str,
    thread: &str,
    trusted: bool,
) -> MessageId {
    let id = MessageId::new();
    fixture.db.create_event(CreateEventRow {
        session_id:session, event_type:"input.message".into(), ts:chrono::Utc::now(),
        context:json!({"input_message_id":id}),
        data:json!({"message":{"id":id,"metadata":{"_app_channel_id":endpoint,"slack_channel":channel,"slack_thread_ts":thread}}}),
        metadata:Some(if trusted {json!({"initiator":{"type":"endpoint","endpoint_id":app}})} else {json!({"initiator":{"type":"user"}})}),
        tags:None,
    }).await.unwrap();
    id
}

async fn setup() -> (Fixture, SessionId, String, String) {
    let fixture = Fixture::new();
    let (app, endpoint, endpoint_public) = fixture
        .seed_app_with_channel(1, "slack", "xoxb-endpoint")
        .await;
    let session = fixture
        .seed_session(1, Some(app), Some(endpoint), vec![])
        .await;
    let app_public = crate::domains::apps::queries::get_by_internal_id(&fixture.db, None, 1, app)
        .await
        .unwrap()
        .unwrap()
        .public_id
        .to_string();
    (fixture, session, app_public, endpoint_public)
}

#[tokio::test]
async fn neutral_tool_posts_to_its_input_thread_and_returns_an_editable_receipt() {
    let (fixture, session, app, endpoint) = setup().await;
    let slack = crate::channels::slack::events::tests_support::mock_provider().await;
    Mock::given(method("POST"))
        .and(path("/chat.postMessage"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok":true,"ts":"9.8"})))
        .mount(&slack)
        .await;
    // The session context deliberately says C1 / 1.2. Two inputs in different
    // conversations must each route to their own trusted origin instead.
    for (channel, thread) in [("C_FIRST", "3.4"), ("C_SECOND", "5.6")] {
        let message = input(&fixture, session, &app, &endpoint, channel, thread, true).await;
        let mut ctx = ToolContext::new(session);
        ctx.event_context = Some(EventContext {
            input_message_id: Some(message),
            ..Default::default()
        });
        ctx.tool_call_id = Some(format!("call-{channel}"));
        let invoker = fixture.invoker(1, session).with_api_base(slack.uri());
        ctx.extensions
            .insert(Arc::new(ConversationSenderExt(Arc::new(
                SlackConversationSender(Arc::new(invoker)),
            ))));
        let result = SendMessageTool
            .execute_with_context(json!({"text":"**Hello**\nCan you confirm?"}), &ctx)
            .await;
        assert!(
            matches!(result, ToolExecutionResult::Success(value) if value["sent"] == true && value["delivery"] == json!({"platform":"slack","channel":channel,"message_ref":"9.8"}))
        );
    }
    let requests = crate::channels::slack::events::tests_support::provider_requests(&slack).await;
    assert_eq!(requests.len(), 2);
    for (request, (channel, thread)) in requests
        .iter()
        .zip([("C_FIRST", "3.4"), ("C_SECOND", "5.6")])
    {
        let payload: Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(payload["channel"], channel);
        assert_eq!(payload["thread_ts"], thread);
        assert_eq!(payload["blocks"][0]["text"], "**Hello**\nCan you confirm?");
        assert_eq!(
            payload["metadata"]["event_payload"]["tool_call_id"],
            format!("call-{channel}")
        );
        assert_eq!(
            payload["metadata"]["event_payload"]["session_id"],
            session.to_string()
        );
        assert_eq!(
            request.headers["authorization"].to_str().unwrap(),
            "Bearer xoxb-endpoint"
        );
    }
}

#[tokio::test]
async fn answer_goes_to_the_thread_of_the_message_that_steered_the_turn() {
    let (fixture, session, app, endpoint) = setup().await;
    let slack = crate::channels::slack::events::tests_support::mock_provider().await;
    Mock::given(method("POST"))
        .and(path("/chat.postMessage"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok":true,"ts":"9.8"})))
        .mount(&slack)
        .await;
    let invoker = fixture.invoker(1, session).with_api_base(slack.uri());
    let turn_input = input(&fixture, session, &app, &endpoint, "D1", "3.4", true).await;
    let post = || {
        invoker.invoke(SlackAction::PostMessage {
            input_message_id: turn_input.to_string(),
            tool_call_id: "call-1".into(),
            text: "hello".into(),
            files: vec![],
        })
    };
    // A second question steers the running turn; the answer belongs under it.
    input(&fixture, session, &app, &endpoint, "D1", "7.8", true).await;
    post().await.unwrap();
    // An input without signed Slack provenance cannot redirect the post.
    input(&fixture, session, &app, &endpoint, "D1", "9.9", false).await;
    post().await.unwrap();
    let threads: Vec<Value> = slack
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|r| serde_json::from_slice::<Value>(&r.body).unwrap()["thread_ts"].clone())
        .collect();
    assert_eq!(threads, vec![json!("7.8"), json!("3.4")]);
}

#[tokio::test]
async fn untrusted_missing_and_sibling_endpoint_inputs_cannot_authorize_posts() {
    let (fixture, session, app, endpoint) = setup().await;
    let slack = crate::channels::slack::events::tests_support::mock_provider().await;
    let invoker = fixture.invoker(1, session).with_api_base(slack.uri());
    let untrusted = input(&fixture, session, &app, &endpoint, "C_OTHER", "1.2", false).await;
    let sibling = input(
        &fixture,
        session,
        &app,
        "appchan_wrong",
        "C_OTHER",
        "1.2",
        true,
    )
    .await;
    for id in [untrusted, sibling, MessageId::new()] {
        let result = invoker
            .invoke(SlackAction::PostMessage {
                input_message_id: id.to_string(),
                tool_call_id: "call-1".into(),
                text: "hello".into(),
                files: vec![],
            })
            .await;
        assert!(matches!(result, Err(SlackActionError::NoSlackSession)));
    }
    assert!(
        crate::channels::slack::events::tests_support::provider_requests(&slack)
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn posting_reports_rate_limits_and_does_not_blindly_retry_uncertain_sends() {
    for (status, body, expected_rate_limit) in [
        (429, json!({"ok":false,"error":"ratelimited"}), true),
        (200, json!({"ok":true}), false),
    ] {
        let (fixture, session, app, endpoint) = setup().await;
        let message = input(&fixture, session, &app, &endpoint, "C1", "1.2", true).await;
        let slack = crate::channels::slack::events::tests_support::mock_provider().await;
        Mock::given(method("POST"))
            .and(path("/chat.postMessage"))
            .respond_with(
                ResponseTemplate::new(status)
                    .insert_header("Retry-After", "37")
                    .set_body_json(body),
            )
            .mount(&slack)
            .await;
        let result = fixture
            .invoker(1, session)
            .with_api_base(slack.uri())
            .invoke(SlackAction::PostMessage {
                input_message_id: message.to_string(),
                tool_call_id: "call-1".into(),
                text: "hello".into(),
                files: vec![],
            })
            .await;
        if expected_rate_limit {
            assert!(matches!(
                result,
                Err(SlackActionError::RateLimited {
                    retry_after_secs: Some(37)
                })
            ));
        } else {
            assert!(matches!(
                result,
                Err(SlackActionError::DeliveryUnknown { .. })
            ));
        }
        assert_eq!(
            crate::channels::slack::events::tests_support::provider_requests(&slack)
                .await
                .len(),
            1
        );
    }
}

#[tokio::test]
async fn slack_ingress_principal_provenance_authorizes_the_neutral_post() {
    use crate::api::messages::{CreateMessageRequest, InputContentPart, InputMessage, MessageRole};
    let (fixture, session, app_public, endpoint) = setup().await;
    let app = crate::domains::apps::queries::get_by_public_id(&fixture.db, None, 1, &app_public)
        .await
        .unwrap()
        .unwrap();
    let actor = everruns_core::ExternalActor {
        actor_id: "U_REQUESTER".into(),
        actor_name: None,
        source: "slack".into(),
        metadata: Some(
            [("team_id".into(), "T_REQUESTER".into())]
                .into_iter()
                .collect(),
        ),
    };
    let principal = crate::domains::users::PrincipalService::new(fixture.db.clone())
        .ensure_external_actor_principal(1, &actor)
        .await
        .unwrap();
    let service = crate::domains::messages::MessageService::new(
        fixture.db.clone(),
        Arc::new(crate::channels::slack::events::tests_support::NoopRunner),
        crate::live_updates::event_delivery::EventDelivery::in_memory(),
    );
    let message = service
        .create(
            crate::domains::messages::CreateMessageContext {
                runtime_subject_principal_id: Some(principal.id),
                org_id: 1,
                user_id: None,
                harness_id: app.harness_id.uuid(),
                agent_id: app.agent_id.map(|id| id.uuid()),
                session_id: session.uuid(),
                event_metadata: Some(crate::execution_metadata::channel_message_metadata(
                    app.public_id,
                    app.owner_principal_id,
                    app.virtual_user_id,
                )),
                request_id: None,
            },
            CreateMessageRequest {
                message: InputMessage {
                    role: MessageRole::User,
                    content: vec![InputContentPart::text("Proceed")],
                },
                addressed_participant_id: None,
                controls: None,
                metadata: Some(
                    [
                        ("_app_channel_id".into(), json!(endpoint)),
                        ("slack_channel".into(), json!("C_RESUME")),
                        ("slack_thread_ts".into(), json!("7.6")),
                    ]
                    .into_iter()
                    .collect(),
                ),
                tags: None,
                external_actor: Some(actor),
                client_message_id: None,
            },
        )
        .await
        .unwrap();
    let input = fixture
        .db
        .find_input_message_event(session, message.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        input.metadata.as_ref().unwrap()["source"]["initiator"]["type"],
        "channel"
    );
    let slack = crate::channels::slack::events::tests_support::mock_provider().await;
    Mock::given(method("POST"))
        .and(path("/chat.postMessage"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok":true,"ts":"8.9"})))
        .mount(&slack)
        .await;
    let result = fixture
        .invoker(1, session)
        .with_api_base(slack.uri())
        .invoke(SlackAction::PostMessage {
            input_message_id: message.id.to_string(),
            tool_call_id: "resume-call".into(),
            text: "Continuing".into(),
            files: vec![],
        })
        .await
        .unwrap();
    assert!(
        matches!(result, SlackActionOutcome::MessagePosted { channel, timestamp } if channel == "C_RESUME" && timestamp == "8.9")
    );
}

#[tokio::test]
async fn native_agent_channel_posts_and_edits_without_an_archival_app() {
    use crate::domains::agent_channels::record::ChannelType;
    use crate::domains::agent_channels::types::CreateAgentChannelRequest;
    use crate::domains::agent_channels::{CreateAgentChannel, PublishAgentChannel};
    use crate::domains::common::{Command, Ctx};
    let fixture = Fixture::new();
    let harness = fixture.seed_harness().await;
    let agent = fixture.seed_agent(1, harness).await;
    let ctx = Ctx::minimal_for_test(everruns_core::Caller::internal(1), fixture.db.clone(), None);
    let endpoint = CreateAgentChannel {
        agent_id: agent.to_string(),
        req: CreateAgentChannelRequest {
            channel_type: ChannelType::Slack,
            channel_config: json!({"bot_token":"xoxb-native", "signing_secret":"s"}),
            enabled: true,
        },
    }
    .run(&ctx)
    .await
    .unwrap();
    PublishAgentChannel {
        agent_id: agent.to_string(),
        channel_id: endpoint.public_id.to_string(),
    }
    .run(&ctx)
    .await
    .unwrap();
    let session = fixture
        .seed_session_for_agent(1, None, Some(endpoint.internal_id), vec![], Some(agent))
        .await;
    assert_eq!(
        fixture
            .db
            .get_agent_channel_public_id(1, endpoint.internal_id)
            .await
            .unwrap(),
        Some(endpoint.public_id.to_string())
    );
    assert!(
        fixture
            .db
            .get_agent_channel_public_id(2, endpoint.internal_id)
            .await
            .unwrap()
            .is_none()
    );
    let app = everruns_contracts::typed_id::AppId::from_uuid(endpoint.internal_id).to_string();
    let message = input(
        &fixture,
        session,
        &app,
        &endpoint.public_id.to_string(),
        "C1",
        "1.2",
        true,
    )
    .await;
    let slack = crate::channels::slack::events::tests_support::mock_provider().await;
    Mock::given(method("POST"))
        .and(path("/chat.postMessage"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok":true,"ts":"9.8"})))
        .expect(1)
        .mount(&slack)
        .await;
    Mock::given(method("POST"))
        .and(path("/chat.update"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok":true})))
        .expect(1)
        .mount(&slack)
        .await;
    let invoker = fixture.invoker(1, session).with_api_base(slack.uri());
    let action = SlackAction::PostMessage {
        input_message_id: message.to_string(),
        tool_call_id: "native-call".into(),
        text: "Hello".into(),
        files: vec![],
    };
    assert!(
        matches!(invoker.invoke(action.clone()).await.unwrap(), SlackActionOutcome::MessagePosted {channel,timestamp} if channel=="C1" && timestamp=="9.8")
    );
    invoker
        .invoke(SlackAction::UpdateMessage {
            channel: "C1".into(),
            timestamp: "9.8".into(),
            text: "Updated".into(),
        })
        .await
        .unwrap();
    for request in crate::channels::slack::events::tests_support::provider_requests(&slack).await {
        assert_eq!(
            request.headers["authorization"].to_str().unwrap(),
            "Bearer xoxb-native"
        );
    }
    fixture
        .db
        .update_session(
            1,
            session,
            crate::storage::UpdateSession {
                status: Some("active".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    // Endpoint ownership and current agent liveness remain required without an App FK.
    for (status, suspended) in [("archived", false), ("active", true)] {
        fixture
            .db
            .update_agent(
                1,
                agent,
                crate::storage::UpdateAgent {
                    status: Some(status.into()),
                    exposures_suspended: Some(suspended),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .expect("update agent liveness");
        assert!(matches!(
            invoker.invoke(action.clone()).await,
            Err(SlackActionError::ChannelUnavailable)
        ));
    }
    fixture
        .db
        .update_agent(
            1,
            agent,
            crate::storage::UpdateAgent {
                status: Some("active".into()),
                exposures_suspended: Some(false),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .expect("restore agent liveness");
    let other_agent = fixture.seed_agent(1, harness).await;
    let unrelated = fixture
        .seed_session_for_agent(
            1,
            None,
            Some(endpoint.internal_id),
            vec![],
            Some(other_agent),
        )
        .await;
    assert!(matches!(
        fixture
            .invoker(1, unrelated)
            .with_api_base(slack.uri())
            .invoke(action.clone())
            .await,
        Err(SlackActionError::NoSlackSession)
    ));
    assert!(matches!(
        fixture
            .invoker(2, session)
            .with_api_base(slack.uri())
            .invoke(action.clone())
            .await,
        Err(SlackActionError::NoSlackSession)
    ));
    fixture
        .db
        .update_agent_channel(
            1,
            agent.uuid(),
            &endpoint.public_id.to_string(),
            crate::storage::UpdateAgentChannelRow {
                enabled: Some(false),
                status: Some("disabled".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .expect("native endpoint must be disabled");
    assert!(matches!(
        invoker.invoke(action).await,
        Err(SlackActionError::ChannelUnavailable)
    ));
    assert_eq!(
        crate::channels::slack::events::tests_support::provider_requests(&slack)
            .await
            .len(),
        2
    );
}

#[tokio::test]
async fn a_top_level_dm_input_is_answered_in_the_dm_itself() {
    let (fixture, session, app, endpoint) = setup().await;
    let slack = crate::channels::slack::events::tests_support::mock_provider().await;
    Mock::given(method("POST"))
        .and(path("/chat.postMessage"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok":true,"ts":"9.8"})))
        .mount(&slack)
        .await;
    let id = MessageId::new();
    fixture.db.create_event(CreateEventRow {
        session_id:session, event_type:"input.message".into(), ts:chrono::Utc::now(),
        context:json!({"input_message_id":id}),
        data:json!({"message":{"id":id,"metadata":{"_app_channel_id":endpoint,"slack_channel":"D1","slack_thread_ts":"3.4","slack_reply_inline":true}}}),
        metadata:Some(json!({"initiator":{"type":"endpoint","endpoint_id":app}})),
        tags:None,
    }).await.unwrap();
    let invoker = fixture.invoker(1, session).with_api_base(slack.uri());
    invoker
        .invoke(SlackAction::PostMessage {
            input_message_id: id.to_string(),
            tool_call_id: "call-1".into(),
            text: "hello".into(),
            files: vec![],
        })
        .await
        .unwrap();
    let requests = crate::channels::slack::events::tests_support::provider_requests(&slack).await;
    let payload: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(payload["channel"], "D1");
    assert!(payload.get("thread_ts").is_none(), "{payload}");
}

/// A session on a Slack channel whose binding is `strategy`.
async fn setup_bound(strategy: &str) -> (Fixture, SessionId, String, String) {
    let fixture = Fixture::new();
    let (app, endpoint, endpoint_public) = fixture
        .seed_app_with_channel(1, "slack", "xoxb-endpoint")
        .await;
    fixture
        .db
        .update_channel_by_id(
            endpoint,
            crate::storage::UpdateChannelByIdRow {
                channel_config: Some(json!({
                    "signing_secret": "s",
                    "bot_token": "xoxb-endpoint",
                    "session_strategy": strategy,
                })),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let session = fixture
        .seed_session(1, Some(app), Some(endpoint), vec![])
        .await;
    let app_public = crate::domains::apps::queries::get_by_internal_id(&fixture.db, None, 1, app)
        .await
        .unwrap()
        .unwrap()
        .public_id
        .to_string();
    (fixture, session, app_public, endpoint_public)
}

/// The input a session schedule injects: schedule provenance, no Slack metadata.
async fn scheduled_input(fixture: &Fixture, session: SessionId) -> MessageId {
    let id = MessageId::new();
    let schedule_id = everruns_contracts::typed_id::ScheduleId::new();
    fixture
        .db
        .create_event(CreateEventRow {
            session_id: session,
            event_type: "input.message".into(),
            ts: chrono::Utc::now(),
            context: json!({"input_message_id": id}),
            data: json!({"message": {"id": id, "metadata": {
                "source": "schedule",
                "schedule_id": schedule_id.to_string(),
            }}}),
            metadata: Some(crate::execution_metadata::scheduled_run_metadata(
                schedule_id,
                everruns_contracts::typed_id::PrincipalId::from_seed(1),
                None,
            )),
            tags: None,
        })
        .await
        .unwrap();
    id
}

async fn post_scheduled(
    fixture: &Fixture,
    session: SessionId,
    slack: &MockServer,
    input_message_id: MessageId,
) -> Result<SlackActionOutcome, SlackActionError> {
    fixture
        .invoker(1, session)
        .with_api_base(slack.uri())
        .invoke(SlackAction::PostMessage {
            input_message_id: input_message_id.to_string(),
            tool_call_id: "call-1".into(),
            text: "07:00 briefing".into(),
            files: vec![],
        })
        .await
}

async fn slack_ok() -> MockServer {
    let slack = crate::channels::slack::events::tests_support::mock_provider().await;
    Mock::given(method("POST"))
        .and(path("/chat.postMessage"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok":true,"ts":"9.8"})))
        .mount(&slack)
        .await;
    slack
}

fn posted(requests: &[wiremock::Request]) -> Vec<Value> {
    requests
        .iter()
        .map(|r| serde_json::from_slice::<Value>(&r.body).unwrap())
        .collect()
}

#[tokio::test]
async fn a_scheduled_turn_in_a_dm_session_posts_inline_in_the_dm() {
    let (fixture, session, app, endpoint) = setup_bound("per_user").await;
    let slack = slack_ok().await;
    let id = MessageId::new();
    fixture.db.create_event(CreateEventRow {
        session_id:session, event_type:"input.message".into(), ts:chrono::Utc::now(),
        context:json!({"input_message_id":id}),
        data:json!({"message":{"id":id,"metadata":{"_app_channel_id":endpoint,"slack_channel":"D1","slack_thread_ts":"3.4","slack_reply_inline":true}}}),
        metadata:Some(json!({"initiator":{"type":"channel","channel_id":app}})),
        tags:None,
    }).await.unwrap();
    let scheduled = scheduled_input(&fixture, session).await;
    post_scheduled(&fixture, session, &slack, scheduled)
        .await
        .unwrap();
    let payloads =
        posted(&crate::channels::slack::events::tests_support::provider_requests(&slack).await);
    assert_eq!(payloads.len(), 1);
    assert_eq!(payloads[0]["channel"], "D1");
    assert!(payloads[0].get("thread_ts").is_none(), "{}", payloads[0]);
}

#[tokio::test]
async fn a_scheduled_turn_in_a_per_channel_session_posts_top_level() {
    let (fixture, session, app, endpoint) = setup_bound("per_channel").await;
    let slack = slack_ok().await;
    input(&fixture, session, &app, &endpoint, "C1", "3.4", true).await;
    // An unsigned input claiming another conversation cannot pick the target.
    input(&fixture, session, &app, &endpoint, "C_EVIL", "6.6", false).await;
    let scheduled = scheduled_input(&fixture, session).await;
    post_scheduled(&fixture, session, &slack, scheduled)
        .await
        .unwrap();
    let payloads =
        posted(&crate::channels::slack::events::tests_support::provider_requests(&slack).await);
    assert_eq!(payloads.len(), 1);
    assert_eq!(payloads[0]["channel"], "C1");
    assert!(payloads[0].get("thread_ts").is_none(), "{}", payloads[0]);
}

#[tokio::test]
async fn a_scheduled_turn_in_a_thread_session_posts_in_its_thread() {
    let (fixture, session, app, endpoint) = setup_bound("per_thread").await;
    let slack = slack_ok().await;
    input(&fixture, session, &app, &endpoint, "C1", "3.4", true).await;
    let scheduled = scheduled_input(&fixture, session).await;
    post_scheduled(&fixture, session, &slack, scheduled)
        .await
        .unwrap();
    let payloads =
        posted(&crate::channels::slack::events::tests_support::provider_requests(&slack).await);
    assert_eq!(payloads.len(), 1);
    assert_eq!(payloads[0]["channel"], "C1");
    assert_eq!(payloads[0]["thread_ts"], "3.4");
}

#[tokio::test]
async fn a_scheduled_turn_without_a_signed_slack_input_has_no_slack_session() {
    let (fixture, session, app, endpoint) = setup_bound("per_user").await;
    let slack = slack_ok().await;
    // Caller-controlled metadata alone, and a sibling endpoint, authorize nothing.
    input(&fixture, session, &app, &endpoint, "D1", "3.4", false).await;
    input(&fixture, session, &app, "appchan_wrong", "D2", "3.4", true).await;
    let scheduled = scheduled_input(&fixture, session).await;
    let result = post_scheduled(&fixture, session, &slack, scheduled).await;
    assert!(
        matches!(result, Err(SlackActionError::NoSlackSession)),
        "{result:?}"
    );
    assert!(
        crate::channels::slack::events::tests_support::provider_requests(&slack)
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn sent_files_follow_the_text_into_the_same_thread() {
    use everruns_core::host::InMemorySessionFileStore;
    use everruns_core::session_files::SessionFileSystem;
    let (fixture, session, app, endpoint) = setup().await;
    let slack = crate::channels::slack::events::tests_support::mock_provider().await;
    Mock::given(method("POST"))
        .and(path("/chat.postMessage"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok":true,"ts":"9.8"})))
        .mount(&slack)
        .await;
    Mock::given(method("POST"))
        .and(path("/files.getUploadURLExternal"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true, "upload_url": "https://files.slack.com/upload-here", "file_id": "F1",
        })))
        .mount(&slack)
        .await;
    Mock::given(method("POST"))
        .and(path("/upload-here"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&slack)
        .await;
    Mock::given(method("POST"))
        .and(path("/files.completeUploadExternal"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true, "files": [{ "id": "F1" }],
        })))
        .mount(&slack)
        .await;
    let message = input(&fixture, session, &app, &endpoint, "C_FIRST", "3.4", true).await;
    let store = Arc::new(InMemorySessionFileStore::new());
    store
        .write_file(session, "/out/chart.png", "iVBORw==", "base64")
        .await
        .unwrap();
    let mut ctx = ToolContext::new(session);
    ctx.file_store = Some(store);
    ctx.event_context = Some(EventContext {
        input_message_id: Some(message),
        ..Default::default()
    });
    ctx.tool_call_id = Some("call-1".into());
    let invoker = fixture
        .invoker(1, session)
        .with_api_base(slack.uri())
        .with_egress(Some(
            crate::channels::slack::events::tests_support::upload_egress(&slack.uri()),
        ));
    ctx.extensions
        .insert(Arc::new(ConversationSenderExt(Arc::new(
            SlackConversationSender(Arc::new(invoker)),
        ))));

    let result = SendMessageTool
        .execute_with_context(
            json!({"text":"Chart attached", "files":["/out/chart.png"]}),
            &ctx,
        )
        .await;

    let ToolExecutionResult::Success(value) = result else {
        panic!("send failed: {result:?}");
    };
    assert_eq!(value["files"], json!(["/out/chart.png"]));
    assert_eq!(value["delivery"]["message_ref"], "9.8");
    let requests = crate::channels::slack::events::tests_support::provider_requests(&slack).await;
    let paths: Vec<&str> = requests.iter().map(|r| r.url.path()).collect();
    assert_eq!(
        paths,
        vec![
            "/chat.postMessage",
            "/files.getUploadURLExternal",
            "/upload-here",
            "/files.completeUploadExternal"
        ]
    );
    assert_eq!(requests[2].body, b"\x89PNG");
    let reserve: std::collections::HashMap<_, _> = url::form_urlencoded::parse(&requests[1].body)
        .into_owned()
        .collect();
    assert_eq!(reserve["filename"], "chart.png");
    let complete: Value = serde_json::from_slice(&requests[3].body).unwrap();
    assert_eq!(complete["channel_id"], "C_FIRST");
    assert_eq!(complete["thread_ts"], "3.4");
}

#[tokio::test]
async fn a_failed_file_after_the_text_says_the_text_was_delivered() {
    let (fixture, session, app, endpoint) = setup().await;
    let slack = crate::channels::slack::events::tests_support::mock_provider().await;
    Mock::given(method("POST"))
        .and(path("/chat.postMessage"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok":true,"ts":"9.8"})))
        .mount(&slack)
        .await;
    Mock::given(method("POST"))
        .and(path("/files.getUploadURLExternal"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"ok": false, "error": "missing_scope"})),
        )
        .mount(&slack)
        .await;
    let message = input(&fixture, session, &app, &endpoint, "C_FIRST", "3.4", true).await;
    let error = fixture
        .invoker(1, session)
        .with_api_base(slack.uri())
        .invoke(SlackAction::PostMessage {
            input_message_id: message.to_string(),
            tool_call_id: "call-1".into(),
            text: "Chart attached".into(),
            files: vec![everruns_contracts::slack_action::SlackFile {
                filename: "chart.png".into(),
                content: b"\x89PNG".to_vec(),
            }],
        })
        .await
        .expect_err("the file upload failed");
    let SlackActionError::PartialDelivery {
        confirmed,
        reason,
        retry_after_secs,
    } = error
    else {
        panic!("expected a typed partial delivery, got {error:?}");
    };
    assert_eq!(confirmed, ["C_FIRST:9.8"]);
    assert_eq!(retry_after_secs, None);
    assert!(reason.contains("files:write"), "{reason}");
}

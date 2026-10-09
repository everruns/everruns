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
    let slack = MockServer::start().await;
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
    let requests = slack.received_requests().await.unwrap();
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
async fn untrusted_missing_and_sibling_endpoint_inputs_cannot_authorize_posts() {
    let (fixture, session, app, endpoint) = setup().await;
    let slack = MockServer::start().await;
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
            })
            .await;
        assert!(matches!(result, Err(SlackActionError::NoSlackSession)));
    }
    assert!(slack.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn posting_reports_rate_limits_and_does_not_blindly_retry_uncertain_sends() {
    for (status, body, expected_rate_limit) in [
        (429, json!({"ok":false,"error":"ratelimited"}), true),
        (200, json!({"ok":true}), false),
    ] {
        let (fixture, session, app, endpoint) = setup().await;
        let message = input(&fixture, session, &app, &endpoint, "C1", "1.2", true).await;
        let slack = MockServer::start().await;
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
            assert!(matches!(result, Err(SlackActionError::Transient(_))));
        }
        assert_eq!(slack.received_requests().await.unwrap().len(), 1);
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
        false,
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
    let slack = MockServer::start().await;
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
    let slack = MockServer::start().await;
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
    for request in slack.received_requests().await.unwrap() {
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
    let (_events, rx) = tokio::sync::broadcast::channel(16);
    let dispatcher = crate::channels::slack::delivery::SlackDeliveryDispatcher::start(
        fixture.db.clone(),
        rx,
        String::new(),
    );
    dispatcher.recover(None).await;
    assert_eq!(
        dispatcher.active_delivery_count().await,
        1,
        "native working feedback must survive recovery"
    );

    input(
        &fixture,
        session,
        &app,
        &endpoint.public_id.to_string(),
        "C_FORGED",
        "9.0",
        false,
    )
    .await;
    let (_api_events, api_rx) = tokio::sync::broadcast::channel(16);
    let api_dispatcher = crate::channels::slack::delivery::SlackDeliveryDispatcher::start(
        fixture.db.clone(),
        api_rx,
        String::new(),
    );
    api_dispatcher.recover(None).await;
    assert_eq!(
        api_dispatcher.active_delivery_count().await,
        0,
        "API metadata cannot redirect native recovery"
    );

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
    assert_eq!(slack.received_requests().await.unwrap().len(), 2);
}

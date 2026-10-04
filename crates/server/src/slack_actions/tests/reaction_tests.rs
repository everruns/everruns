use super::*;

#[tokio::test]
async fn missing_message_is_actionable_and_exact_timestamp_succeeds() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/reactions.add"))
        .and(wiremock::matchers::body_json(json!({
            "channel": "C1", "timestamp": "1791063223.000000", "name": "thumbsup"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": false, "error": "message_not_found"
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/reactions.add"))
        .and(wiremock::matchers::body_json(json!({
            "channel": "C1", "timestamp": "1791063223.531299", "name": "thumbsup"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "ok": true })))
        .mount(&server)
        .await;
    let fixture = Fixture::new();
    let (app_id, endpoint_id, _) = fixture
        .seed_app_with_channel(ORG, "slack", "xoxb-endpoint-secret")
        .await;
    let session_id = fixture
        .seed_session(ORG, Some(app_id), Some(endpoint_id), vec![])
        .await;
    let invoker = fixture.invoker(ORG, session_id).with_api_base(server.uri());
    let reaction = |timestamp: &str| SlackAction::AddReaction {
        channel: "C1".into(),
        timestamp: timestamp.into(),
        name: "thumbsup".into(),
    };
    let error = invoker
        .invoke(reaction("1791063223.000000"))
        .await
        .unwrap_err();
    assert!(
        matches!(error, SlackActionError::Rejected(ref message) if message.contains("message_not_found")),
        "{error:?}"
    );
    assert_eq!(
        invoker.invoke(reaction("1791063223.531299")).await.unwrap(),
        SlackActionOutcome::ReactionAdded {
            already_reacted: false
        }
    );
}

#[tokio::test]
async fn missing_reaction_scope_explains_how_to_reconnect() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/reactions.add"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": false, "error": "missing_scope"
        })))
        .mount(&server)
        .await;
    let fixture = Fixture::new();
    let (app, endpoint, _) = fixture
        .seed_app_with_endpoint(ORG, "slack", "test-token")
        .await;
    let session = fixture
        .seed_session(ORG, Some(app), Some(endpoint), vec![])
        .await;
    let error = fixture
        .invoker(ORG, session)
        .with_api_base(server.uri())
        .invoke(add_reaction_action())
        .await
        .unwrap_err();
    assert!(
        matches!(error, SlackActionError::Rejected(ref message)
        if message.contains("reconnect") && message.contains("reactions:write")),
        "{error:?}"
    );
}

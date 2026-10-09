use super::*;

#[tokio::test]
async fn typed_missing_scope_records_one_persistent_health_issue() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/reactions.add"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"ok":false,"error":"missing_scope"})),
        )
        .expect(10)
        .mount(&server)
        .await;
    let fixture = Fixture::new();
    let (app, channel, public_id) = fixture
        .seed_app_with_channel(ORG, "slack", "xoxb-test")
        .await;
    let session = fixture
        .seed_session(ORG, Some(app), Some(channel), vec![])
        .await;
    let invoker = fixture.invoker(ORG, session).with_api_base(server.uri());
    for _ in 0..10 {
        let error = invoker.invoke(add_reaction_action()).await.unwrap_err();
        assert!(matches!(error, SlackActionError::Rejected(_)));
    }
    let issues = fixture
        .db
        .list_health_issues(ORG, 0, 20, Some(&public_id))
        .await
        .unwrap();
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].missing_scopes, vec!["reactions:write"]);
}

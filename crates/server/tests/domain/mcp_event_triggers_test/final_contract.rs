use super::*;

#[tokio::test]
async fn signed_gap_and_permission_termination_do_not_start_runs() {
    let server = TestServer::in_memory().await;
    let fake = FakeEventsServer::install(&server);
    let agent_id = create_agent(&server).await;
    let trigger = create_trigger(&server, &agent_id).await;
    let subscription = fake.subscription();
    subscription
        .post(
            &server,
            "msg_gap",
            &json!({"type":"gap", "cursor":"new-position"}),
        )
        .await
        .assert_status(StatusCode::OK);
    let state = server
        .db
        .get_agent_trigger_mcp_subscription(trigger_id(&trigger))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(state.cursor.as_deref(), Some("new-position"));
    subscription
        .post(
            &server,
            "msg_null_gap",
            &json!({"type":"gap", "cursor":null}),
        )
        .await
        .assert_status(StatusCode::OK);
    let state = server
        .db
        .get_agent_trigger_mcp_subscription(trigger_id(&trigger))
        .await
        .unwrap()
        .unwrap();
    assert!(state.cursor.is_none());
    subscription
        .post(
            &server,
            "msg_terminated",
            &json!({"type":"terminated", "error":{"code":-32024,"message":"Forbidden"}}),
        )
        .await
        .assert_status(StatusCode::OK);
    let state = server
        .db
        .get_agent_trigger_mcp_subscription(trigger_id(&trigger))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(state.status, "failed");
    assert!(state.refresh_before.is_none());
    assert_eq!(server.mcp_event_triggers.refresh_due().await, 0);
    subscription
        .post(
            &server,
            "msg_after_termination",
            &event("must not run", "c1"),
        )
        .await
        .assert_status(StatusCode::GONE);
    let invocations: Value = server
        .get(&format!(
            "/v1/platform/agents/{agent_id}/triggers/{}/deliveries",
            trigger["id"].as_str().unwrap()
        ))
        .await
        .assert_success()
        .json();
    assert!(invocations.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn same_event_on_two_subscriptions_is_delivered_to_both() {
    let server = TestServer::in_memory().await;
    let fake = FakeEventsServer::install(&server);
    let agent_id = create_agent(&server).await;
    create_trigger(&server, &agent_id).await;
    let first = fake.subscription();
    create_trigger(&server, &agent_id).await;
    let second = fake.subscription();
    let body = event("Both subscriptions", "c1");
    for subscription in [first, second] {
        let result: Value = subscription
            .post(&server, "msg_same_event", &body)
            .await
            .assert_status(StatusCode::OK)
            .json();
        assert_eq!(result["delivery"], "dispatched");
    }
}

#[tokio::test]
async fn refresh_with_null_cursor_forgets_previous_replay_position() {
    let server = TestServer::in_memory().await;
    let fake = FakeEventsServer::install(&server);
    let agent_id = create_agent(&server).await;
    let trigger = create_trigger(&server, &agent_id).await;
    fake.subscription()
        .post(&server, "msg_before_null", &event("x", "old-position"))
        .await
        .assert_status(StatusCode::OK);
    fake.null_cursor.store(true, Ordering::SeqCst);
    let row = server
        .db
        .get_agent_trigger(DEFAULT_ORG_ID, trigger_id(&trigger))
        .await
        .unwrap()
        .unwrap();
    server
        .mcp_event_triggers
        .subscribe(&row, false)
        .await
        .unwrap();
    let state = server
        .db
        .get_agent_trigger_mcp_subscription(trigger_id(&trigger))
        .await
        .unwrap()
        .unwrap();
    assert!(state.cursor.is_none());
    server
        .mcp_event_triggers
        .subscribe(&row, false)
        .await
        .unwrap();
    assert!(
        fake.calls("events/subscribe")
            .last()
            .unwrap()
            .params
            .get("cursor")
            .is_none()
    );
}

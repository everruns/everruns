use super::*;
use everruns_server::auth::jwt::{JwtService, McpTokenGrant};

async fn token(server: &TestServer, user_id: uuid::Uuid, name: &str) -> (String, String) {
    let id = format!("mcp_client_{}", uuid::Uuid::now_v7().simple());
    server
        .db
        .create_oauth_client(everruns_server::storage::CreateOAuthClientRow {
            client_id: id.clone(),
            client_secret_hash: "unused".into(),
            client_name: name.into(),
            redirect_uris: json!(["http://localhost:9999/callback"]),
        })
        .await
        .unwrap();
    let grant = server.db.approve_oauth_grant(&id, user_id).await.unwrap();
    let metadata: Value = server
        .get("/.well-known/oauth-protected-resource/mcp")
        .await
        .assert_success()
        .json();
    let token = JwtService::new(Default::default())
        .generate_mcp_access_token_for_grant(
            user_id,
            "test@example.com",
            "Test",
            &[],
            metadata["resource"].as_str().unwrap(),
            McpTokenGrant {
                client_id: &id,
                grant_id: grant.id,
            },
        )
        .unwrap();
    (token, id)
}

async fn call(server: &TestServer, token: &str, name: &str, arguments: Value) -> Value {
    let bearer = format!("Bearer {token}");
    let response = mcp_call_with_headers(
        server,
        "tools/call",
        json!({
            "name": name, "arguments": arguments,
            "_meta": {"clientInfo": {"name":"Forged Trusted Brand", "version":"1"}}
        }),
        vec![("authorization", &bearer)],
    )
    .await;
    assert!(!tool_is_error(&response), "{response}");
    serde_json::from_str(&tool_text(&response)).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn creator_is_immutable_and_each_client_input_keeps_its_own_provenance() {
    let server = TestServer::in_memory_authenticated().await;
    let user = create_oauth_test_user(&server).await;
    let (a, a_id) = token(&server, user, "Client A").await;
    let (b, b_id) = token(&server, user, "Client B").await;
    let created = call(
        &server,
        &a,
        "execute",
        json!({"commands":"create_session --title Attribution"}),
    )
    .await;
    let session = created["id"].as_str().expect("session id");
    assert_eq!(created["mcp_creator"]["client_id"], a_id);
    for (credential, expected) in [(&a, &a_id), (&b, &b_id)] {
        call(
            &server,
            credential,
            "session_send_message",
            json!({"session_id":session,"message":"hello"}),
        )
        .await;
        let messages = call(
            &server,
            credential,
            "query",
            json!({"commands":format!("list_events --session_id {session} --types '[\"input.message\"]'")}),
        )
        .await;
        let inputs = messages
            .as_array()
            .or_else(|| messages["data"].as_array())
            .expect("messages");
        assert!(
            inputs
                .iter()
                .any(|m| m["metadata"]["mcp_client"]["client_id"] == *expected),
            "{messages}"
        );
    }
    let current = call(
        &server,
        &b,
        "query",
        json!({"commands":format!("get_session --session_id {session}")}),
    )
    .await;
    assert_eq!(current["mcp_creator"]["client_id"], a_id);
    assert_eq!(current["mcp_creator"]["client_name"], "Client A");
    let updated = call(
        &server,
        &b,
        "execute",
        json!({"commands":format!(
            "update_session --session_id {session} --title 'Updated by B'"
        )}),
    )
    .await;
    assert_eq!(updated["mcp_creator"]["client_id"], a_id);
    let history = call(
        &server,
        &b,
        "query",
        json!({"commands":format!(
            "list_entity_history --entity_ref {session} --kind session"
        )}),
    )
    .await;
    let changes = history
        .as_array()
        .or_else(|| history["data"].as_array())
        .expect("history");
    for expected in [&a_id, &b_id] {
        assert!(
            changes
                .iter()
                .any(|c| c["mcp_client"]["client_id"] == *expected
                    && c["actor_user_id"] == user.to_string()),
            "{history}"
        );
    }
    // A legacy OAuth credential remains usable but cannot borrow the creator's
    // attribution or invent it through metadata.
    let metadata: Value = server
        .get("/.well-known/oauth-protected-resource/mcp")
        .await
        .assert_success()
        .json();
    let legacy = JwtService::new(Default::default())
        .generate_mcp_access_token(
            user,
            "test@example.com",
            "Test",
            &[],
            metadata["resource"].as_str().unwrap(),
        )
        .unwrap();
    let legacy_session = call(
        &server,
        &legacy,
        "execute",
        json!({
            "commands":"create_session --title Legacy"
        }),
    )
    .await;
    assert!(legacy_session["mcp_creator"].is_null());
    let sent = call(&server, &legacy, "execute", json!({"commands":format!(
        "create_message --session_id {session} --message '{{\"content\":[{{\"type\":\"text\",\"text\":\"legacy\"}}]}}' --metadata '{{\"everruns_mcp_client\":{{\"client_id\":\"forged\"}}}}'"
    )})).await;
    assert!(sent["metadata"]["everruns_mcp_client"].is_null(), "{sent}");
    // Another tenant's membership cannot be fabricated through the override.
    let denied = mcp_call_with_headers(&server, "tools/call", json!({"name":"session_send_message", "arguments": {
        "session_id":session,"message":"denied","organization_id":"org_00000000000000000000000000000099"
    }}), vec![("authorization", &format!("Bearer {b}"))]).await;
    assert!(tool_is_error(&denied));
}

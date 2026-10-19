//! Agent Execution API per-caller daily spending limit and organization
//! members calling with their personal access token.

use crate::agent_api_channel_test::{api_channel, call, create_key, text_message};
use crate::test_harness;

use axum::http::{Method, StatusCode};
use chrono::Utc;
use everruns_contracts::typed_id::SessionId;
use everruns_core::DEFAULT_ORG_ID;
use everruns_server::storage::{CreatePersonalAccessTokenRow, CreateUserRow};
use serde_json::{Value, json};
use test_harness::TestServer;
use uuid::Uuid;

async fn spend(server: &TestServer, session_id: &str, usd: f64) {
    let session: SessionId = session_id.parse().expect("session id");
    server
        .db
        .create_llm_generation(
            DEFAULT_ORG_ID,
            Some(session.uuid()),
            None,
            None,
            "gpt-test".to_string(),
            Some("openai".to_string()),
            100,
            100,
            0,
            0,
            Some(usd),
            None,
            None,
            None,
            None,
            Default::default(),
            Utc::now(),
        )
        .await
        .expect("record generation");
}

#[tokio::test]
async fn a_caller_over_its_daily_limit_cannot_start_new_work() {
    let server = TestServer::in_memory().await;
    let channel = api_channel(&server, json!({ "daily_spend_limit_usd": 1.0 }), true).await;
    let key = create_key(&server, &channel, "Backend").await;
    let other = create_key(&server, &channel, "Other").await;
    let (secret, other) = (key["secret"].as_str(), other["secret"].as_str());
    let sessions = format!("/v1/channels/{}/sessions", channel.channel_id);

    let session: Value = call(&server, Method::POST, &sessions, secret, None)
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let id = session["id"].as_str().unwrap();
    spend(&server, id, 0.6).await;
    // Under the limit: work goes on.
    call(
        &server,
        Method::POST,
        &format!("{sessions}/{id}/messages"),
        secret,
        Some(text_message("hi")),
    )
    .await
    .assert_status(StatusCode::CREATED);

    spend(&server, id, 0.5).await;
    call(
        &server,
        Method::POST,
        &format!("{sessions}/{id}/messages"),
        secret,
        Some(text_message("again")),
    )
    .await
    .assert_status(StatusCode::TOO_MANY_REQUESTS);
    call(&server, Method::POST, &sessions, secret, None)
        .await
        .assert_status(StatusCode::TOO_MANY_REQUESTS);
    // Reading is not spending.
    call(
        &server,
        Method::GET,
        &format!("{sessions}/{id}"),
        secret,
        None,
    )
    .await
    .assert_status(StatusCode::OK);
    // The limit is per caller: another key still works.
    call(&server, Method::POST, &sessions, other, None)
        .await
        .assert_status(StatusCode::CREATED);
}

#[tokio::test]
async fn the_daily_limit_must_be_positive() {
    let server = TestServer::in_memory().await;
    let agent: Value = server
        .post(
            "/v1/agents",
            json!({ "name": "api-agent-spend", "system_prompt": "Test" }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    for limit in [json!(0), json!(-1), json!(2_000_000)] {
        server
            .post(
                &format!("/v1/agents/{}/channels", agent["id"].as_str().unwrap()),
                json!({ "channel_type": "api", "channel_config": { "daily_spend_limit_usd": limit } }),
            )
            .await
            .assert_status(StatusCode::BAD_REQUEST);
    }
}

/// A member of the default organization and a full-access personal access
/// token of theirs.
async fn member_token(server: &TestServer, member: bool) -> String {
    member_token_with_scopes(server, member, &["*"]).await
}

/// Like `member_token`, with the token's stored `scopes`. Creation accepts
/// only `*` until rollout step 4, so a narrow token is inserted directly.
async fn member_token_with_scopes(server: &TestServer, member: bool, scopes: &[&str]) -> String {
    let user = server
        .db
        .create_user(CreateUserRow {
            email: format!("api-member-{}@example.com", Uuid::now_v7()),
            name: "Api Member".to_string(),
            avatar_url: None,
            roles: vec!["user".to_string()],
            password_hash: None,
            email_verified: true,
            auth_provider: None,
            auth_provider_id: None,
            external_id: None,
        })
        .await
        .unwrap();
    if member {
        server
            .db
            .add_organization_member(DEFAULT_ORG_ID, user.id, "member")
            .await
            .unwrap();
    }
    let generated = everruns_server::auth::personal_access_token::generate_personal_access_token();
    server
        .db
        .create_personal_access_token(CreatePersonalAccessTokenRow {
            user_id: user.id,
            name: "cli".to_string(),
            token_hash: generated.token_hash.clone(),
            token_prefix: generated.token_prefix.clone(),
            scopes: scopes.iter().map(|scope| scope.to_string()).collect(),
            expires_at: None,
            metadata: json!({}),
        })
        .await
        .unwrap();
    generated.token
}

#[tokio::test]
async fn members_call_with_their_token_only_when_the_channel_allows() {
    let server = TestServer::in_memory().await;
    let closed = api_channel(&server, json!({}), true).await;
    let open = api_channel(&server, json!({ "org_members": true }), true).await;
    let token = member_token(&server, true).await;
    let stranger = member_token(&server, false).await;

    call(
        &server,
        Method::GET,
        &format!("/v1/channels/{}/sessions", closed.channel_id),
        Some(&token),
        None,
    )
    .await
    .assert_status(StatusCode::UNAUTHORIZED);

    let base = format!("/v1/channels/{}", open.channel_id);
    let card: Value = call(&server, Method::GET, &base, Some(&token), None)
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert!(
        card["auth"]
            .as_array()
            .unwrap()
            .contains(&json!({ "type": "personal_access_token" })),
        "{card}"
    );
    let session: Value = call(
        &server,
        Method::POST,
        &format!("{base}/sessions"),
        Some(&token),
        None,
    )
    .await
    .assert_status(StatusCode::CREATED)
    .json();

    // The member's sessions are theirs: a key acting as itself does not see them.
    let key = create_key(&server, &open, "Backend").await;
    let listed: Value = call(
        &server,
        Method::GET,
        &format!("{base}/sessions"),
        key["secret"].as_str(),
        None,
    )
    .await
    .assert_status(StatusCode::OK)
    .json();
    assert!(
        !listed["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["id"] == session["id"])
    );

    // A non-member's token is refused, and a member cannot assert someone else.
    call(&server, Method::GET, &base, Some(&stranger), None)
        .await
        .assert_status(StatusCode::UNAUTHORIZED);
    let authorization = format!("Bearer {token}");
    server
        .request_raw(
            Method::GET,
            &format!("{base}/sessions"),
            vec![
                ("authorization", authorization.as_str()),
                ("end-user", "someone-else"),
            ],
            Vec::new(),
        )
        .await
        .assert_status(StatusCode::FORBIDDEN);
}

/// knowledge/security/pat-scopes.md, Decision 5: only a `*` token counts as an
/// org member; running an agent is a write, so a `read` token is refused with
/// `insufficient_scope` on every route of the channel.
#[tokio::test]
async fn only_a_full_access_token_calls_as_an_org_member() {
    let server = TestServer::in_memory().await;
    let open = api_channel(&server, json!({ "org_members": true }), true).await;
    let closed = api_channel(&server, json!({}), true).await;
    let full = member_token(&server, true).await;
    let read = member_token_with_scopes(&server, true, &["read"]).await;
    let base = format!("/v1/channels/{}", open.channel_id);

    for (method, path) in [
        (Method::GET, base.clone()),
        (Method::GET, format!("{base}/sessions")),
        (Method::POST, format!("{base}/sessions")),
    ] {
        let response = call(&server, method.clone(), &path, Some(&read), None)
            .await
            .assert_status(StatusCode::FORBIDDEN);
        assert_eq!(
            response
                .headers()
                .get("www-authenticate")
                .and_then(|value| value.to_str().ok()),
            Some("Bearer error=\"insufficient_scope\""),
            "{method} {path}"
        );
        let body: Value = response.json();
        assert_eq!(
            body["code"], "insufficient_scope",
            "{method} {path}: {body}"
        );
    }

    // The same member's `*` token is accepted.
    call(
        &server,
        Method::POST,
        &format!("{base}/sessions"),
        Some(&full),
        None,
    )
    .await
    .assert_status(StatusCode::CREATED);

    // A channel that does not admit members says nothing about the scope.
    call(
        &server,
        Method::GET,
        &format!("/v1/channels/{}/sessions", closed.channel_id),
        Some(&read),
        None,
    )
    .await
    .assert_status(StatusCode::UNAUTHORIZED);
}

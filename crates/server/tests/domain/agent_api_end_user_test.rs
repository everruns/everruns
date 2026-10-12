//! Agent Execution API end users: a key acting for its application's users
//! with `End-User`, runtime tokens from `/runtime-auth`, and the customer's
//! own identity providers in `auth_methods`.

use crate::agent_api_channel_test::{ApiChannel, api_channel, call, create_key};
use crate::test_harness;

use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use test_harness::TestServer;

async fn end_user_key(server: &TestServer, channel: &ApiChannel) -> Value {
    let key: Value = server
        .post(
            &format!(
                "/v1/agents/{}/channels/{}/keys",
                channel.agent_id, channel.channel_id
            ),
            json!({ "name": "Support backend", "permissions": ["end_user"] }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    assert_eq!(key["permissions"], json!(["sessions", "end_user"]));
    key
}

async fn call_as(
    server: &TestServer,
    method: Method,
    path: &str,
    bearer: &str,
    end_user: Option<&str>,
) -> test_harness::TestResponse {
    let authorization = format!("Bearer {bearer}");
    let mut headers = vec![
        ("content-type", "application/json"),
        ("authorization", authorization.as_str()),
    ];
    if let Some(id) = end_user {
        headers.push(("end-user", id));
    }
    let body = if method == Method::POST {
        b"{}".to_vec()
    } else {
        Vec::new()
    };
    server.request_raw(method, path, headers, body).await
}

async fn session_ids(
    server: &TestServer,
    base: &str,
    bearer: &str,
    user: Option<&str>,
) -> Vec<String> {
    let page: Value = call_as(
        server,
        Method::GET,
        &format!("{base}/sessions"),
        bearer,
        user,
    )
    .await
    .assert_status(StatusCode::OK)
    .json();
    page["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|session| session["id"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn end_user_header_needs_a_key_that_holds_end_user() {
    let server = TestServer::in_memory().await;
    let channel = api_channel(&server, json!({}), true).await;
    let plain = create_key(&server, &channel, "Plain").await;
    let base = format!("/v1/channels/{}", channel.channel_id);

    call_as(
        &server,
        Method::POST,
        &format!("{base}/sessions"),
        plain["secret"].as_str().unwrap(),
        Some("customer-42"),
    )
    .await
    .assert_status(StatusCode::FORBIDDEN);

    // A blank or overlong id is refused too, whatever the key holds.
    let key = end_user_key(&server, &channel).await;
    let secret = key["secret"].as_str().unwrap();
    let long = "x".repeat(257);
    for bad in [" customer", long.as_str()] {
        call_as(
            &server,
            Method::GET,
            &format!("{base}/sessions"),
            secret,
            Some(bad),
        )
        .await
        .assert_status(StatusCode::FORBIDDEN);
    }
}

#[tokio::test]
async fn sessions_belong_to_the_end_user_not_the_key() {
    let server = TestServer::in_memory().await;
    let channel = api_channel(&server, json!({ "errors": "detailed" }), true).await;
    let key = end_user_key(&server, &channel).await;
    let secret = key["secret"].as_str().unwrap();
    let base = format!("/v1/channels/{}", channel.channel_id);

    let session: Value = call_as(
        &server,
        Method::POST,
        &format!("{base}/sessions"),
        secret,
        Some("customer-42"),
    )
    .await
    .assert_status(StatusCode::CREATED)
    .json();
    let session_id = session["id"].as_str().unwrap().to_string();

    assert_eq!(
        session_ids(&server, &base, secret, Some("customer-42")).await,
        vec![session_id.clone()]
    );
    // Neither another customer nor the key acting as itself reaches it.
    assert!(
        session_ids(&server, &base, secret, Some("customer-43"))
            .await
            .is_empty()
    );
    assert!(session_ids(&server, &base, secret, None).await.is_empty());
    call_as(
        &server,
        Method::GET,
        &format!("{base}/sessions/{session_id}"),
        secret,
        Some("customer-43"),
    )
    .await
    .assert_status(StatusCode::NOT_FOUND);

    // The session is owned by the end user, so `actsAs: user` uses their
    // connections rather than the key holder's.
    let full: Value = server
        .get(&format!("/v1/sessions/{session_id}"))
        .await
        .assert_status(StatusCode::OK)
        .json();
    let channel_owner: Value = call_as(
        &server,
        Method::POST,
        &format!("{base}/sessions"),
        secret,
        None,
    )
    .await
    .assert_status(StatusCode::CREATED)
    .json();
    let channel_session: Value = server
        .get(&format!(
            "/v1/sessions/{}",
            channel_owner["id"].as_str().unwrap()
        ))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_ne!(
        full["owner_principal_id"], channel_session["owner_principal_id"],
        "an end user's session must not run as the channel owner"
    );
}

#[tokio::test]
async fn runtime_tokens_reach_the_same_end_user_and_never_renew() {
    let server = TestServer::in_memory().await;
    let channel = api_channel(&server, json!({}), true).await;
    let other = api_channel(&server, json!({}), true).await;
    let key = end_user_key(&server, &channel).await;
    let secret = key["secret"].as_str().unwrap();
    let base = format!("/v1/channels/{}", channel.channel_id);

    // A key acting as itself is an application, not someone to mint for.
    call_as(
        &server,
        Method::POST,
        &format!("{base}/runtime-auth"),
        secret,
        None,
    )
    .await
    .assert_status(StatusCode::BAD_REQUEST);

    let exchanged: Value = call_as(
        &server,
        Method::POST,
        &format!("{base}/runtime-auth"),
        secret,
        Some("customer-42"),
    )
    .await
    .assert_status(StatusCode::OK)
    .json();
    let token = exchanged["access_token"].as_str().unwrap();

    // The browser, holding the token, and the backend, acting for the same
    // customer, share sessions.
    let session: Value = call_as(
        &server,
        Method::POST,
        &format!("{base}/sessions"),
        token,
        None,
    )
    .await
    .assert_status(StatusCode::CREATED)
    .json();
    assert_eq!(
        session_ids(&server, &base, secret, Some("customer-42")).await,
        vec![session["id"].as_str().unwrap().to_string()]
    );

    // A token cannot assert someone else, renew itself, or reach another
    // channel.
    call_as(
        &server,
        Method::GET,
        &format!("{base}/sessions"),
        token,
        Some("customer-43"),
    )
    .await
    .assert_status(StatusCode::FORBIDDEN);
    call_as(
        &server,
        Method::POST,
        &format!("{base}/runtime-auth"),
        token,
        None,
    )
    .await
    .assert_status(StatusCode::UNAUTHORIZED);
    call_as(
        &server,
        Method::GET,
        &format!("/v1/channels/{}/sessions", other.channel_id),
        token,
        None,
    )
    .await
    .assert_status(StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn auth_methods_accept_only_identity_providers() {
    let server = TestServer::in_memory().await;
    let agent: Value = server
        .post(
            "/v1/agents",
            json!({ "name": "api-agent-methods", "system_prompt": "Test" }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let agent_id = agent["id"].as_str().unwrap();
    let create = |config: Value| {
        let server = &server;
        async move {
            server
                .post(
                    &format!("/v1/agents/{agent_id}/channels"),
                    json!({ "channel_type": "api", "channel_config": config }),
                )
                .await
        }
    };
    create(json!({ "auth_methods": [{ "mode": "shared_secret" }] }))
        .await
        .assert_status(StatusCode::BAD_REQUEST);
    create(json!({ "auth_methods": [{ "mode": "oidc", "provider": { "type": "oidc", "issuer": "" } }] }))
        .await
        .assert_status(StatusCode::BAD_REQUEST);

    // An introspection secret is write-only and survives an update that
    // leaves it blank.
    let introspection = |secret: Option<&str>| {
        let mut provider = json!({
            "type": "oauth2_introspection",
            "introspection_url": "https://idp.example.com/introspect",
            "client_id": "everruns",
        });
        if let Some(secret) = secret {
            provider["client_secret"] = json!(secret);
        }
        json!({ "auth_methods": [{ "mode": "oauth2_introspection", "provider": provider }] })
    };
    let channel: Value = create(introspection(Some("s3cret")))
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let provider = &channel["channel_config"]["auth_methods"][0]["provider"];
    assert!(provider.get("client_secret").is_none(), "{channel}");
    assert_eq!(provider["client_secret_configured"], true);

    let channel_id = channel["id"].as_str().unwrap();
    let updated: Value = server
        .patch(
            &format!("/v1/agents/{agent_id}/channels/{channel_id}"),
            json!({ "channel_config": introspection(None) }),
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(
        updated["channel_config"]["auth_methods"][0]["provider"]["client_secret_configured"],
        true
    );
}

#[tokio::test]
async fn a_token_no_identity_provider_accepts_is_refused() {
    let server = TestServer::in_memory().await;
    // The issuer cannot be reached, so every token fails verification.
    let channel = api_channel(
        &server,
        json!({ "auth_methods": [{
            "mode": "oidc",
            "provider": { "type": "oidc", "issuer": "https://idp.invalid" },
            "requirements": { "audiences": ["api://support-agent"] },
        }] }),
        true,
    )
    .await;
    let base = format!("/v1/channels/{}", channel.channel_id);
    call(
        &server,
        Method::GET,
        &format!("{base}/sessions"),
        Some("eyJhbGciOiJSUzI1NiJ9.eyJzdWIiOiJ4In0.c2ln"),
        None,
    )
    .await
    .assert_status(StatusCode::UNAUTHORIZED);

    // The card advertises the provider next to keys and runtime tokens.
    let key = create_key(&server, &channel, "Backend").await;
    let card: Value = call(&server, Method::GET, &base, key["secret"].as_str(), None)
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(
        card["auth"],
        json!([
            { "type": "agent_key" },
            { "type": "runtime_token" },
            { "type": "oidc", "issuer": "https://idp.invalid" },
        ])
    );
}

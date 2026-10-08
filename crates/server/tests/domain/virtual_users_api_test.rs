//! Real extractors and routes: consumer self-service never grants management authority.
use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use everruns_core::DEFAULT_ORG_ID;
use everruns_server::{
    api,
    auth::{
        AuthState,
        config::{AuthConfig, AuthMode},
        jwt::JwtService,
    },
    storage::runtime_identity::VerifiedRuntimeIdentity,
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

async fn send(
    router: &Router,
    token: &str,
    method: &str,
    path: &str,
    body: Value,
) -> (StatusCode, Value) {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn runtime_token_is_self_only_and_revocation_is_live() {
    let server = crate::test_harness::TestServer::in_memory().await;
    let agent: Value = server
        .post(
            "/v1/agents",
            json!({"name":"consumer-api", "system_prompt":"Helpful", "harness_id":server.seed_generic_harness_id}),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let app = server
        .seed_app_channel(
            "consumer-api",
            agent["id"].as_str().unwrap(),
            "ag_ui",
            json!({"anonymous":true}),
        )
        .await;
    let app = server
        .set_app_channels_live(app["id"].as_str().unwrap(), true)
        .await;
    let endpoint = app["channels"][0]["id"].as_str().unwrap().to_string();
    let account = server
        .db
        .resolve_runtime_identity(VerifiedRuntimeIdentity {
            org_id: DEFAULT_ORG_ID,
            provider: "oidc".into(),
            realm: "https://issuer.example".into(),
            subject: "alice".into(),
            name: "Alice".into(),
            avatar_url: None,
            management_user_id: None,
        })
        .await
        .unwrap();
    let binding = server
        .db
        .list_virtual_user_bindings(DEFAULT_ORG_ID, account.id)
        .await
        .unwrap()[0]
        .id;
    let config = AuthConfig {
        mode: AuthMode::Full,
        ..Default::default()
    };
    let auth = AuthState::builtin(config.clone(), server.db.clone());
    let capabilities = Arc::new(everruns_server::services::CapabilityService::new(
        server.db.clone(),
        server.encryption.clone(),
    ));
    let router = api::virtual_users::routes(api::state::ApiState::basic(
        server.db.clone(),
        server.encryption.clone(),
        capabilities,
        auth.clone(),
    ))
    .merge(api::virtual_user_connections::routes(
        api::virtual_user_connections::AppState::new(
            server.db.clone(),
            server.encryption.clone(),
            auth,
            everruns_server::platform::oss_connector_registry(),
        ),
    ));
    let token = JwtService::new(config.jwt)
        .generate_runtime_token(DEFAULT_ORG_ID, account.id, endpoint.clone(), binding)
        .unwrap();
    let (status, me) = send(&router, &token, "GET", "/v1/virtual-users/me", Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(me["id"], account.id.to_string());
    assert_eq!(me["usage"], "end_user");
    let path = format!("/v1/virtual-users/{}", account.id);
    assert_eq!(
        send(&router, &token, "PATCH", &path, json!({"name":"Updated"}))
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        send(
            &router,
            &token,
            "PATCH",
            &path,
            json!({"status":"archived"})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        send(&router, &token, "GET", "/v1/virtual-users", Value::Null)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        send(
            &router,
            &token,
            "GET",
            &format!(
                "/v1/virtual-users/{}",
                everruns_contracts::typed_id::VirtualUserId::new()
            ),
            Value::Null
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let preferences = format!("{path}/preferences/theme");
    assert_eq!(
        send(
            &router,
            &token,
            "PUT",
            &preferences,
            json!({"value":"dark"})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        send(&router, &token, "GET", &preferences, Value::Null)
            .await
            .1["value"],
        "dark"
    );
    assert_eq!(
        send(&router, &token, "DELETE", &preferences, Value::Null)
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        send(
            &router,
            &token,
            "GET",
            &format!("{path}/connections"),
            Value::Null
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        send(
            &router,
            &token,
            "GET",
            &format!(
                "/v1/virtual-users/{}/connections",
                everruns_contracts::typed_id::VirtualUserId::from_uuid(Uuid::new_v4())
            ),
            Value::Null
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    // User MCP servers follow the same self-only rule as preferences.
    let (status, added) = send(
        &router,
        &token,
        "POST",
        "/v1/virtual-users/me/mcp-servers",
        json!({"name":"notes","url":"https://notes.example.com/mcp","auth_mode":"oauth"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{added}");
    assert_eq!(added["source"], "custom");
    assert_eq!(added["connection"]["status"], "not_connected");
    let (status, listed) = send(
        &router,
        &token,
        "GET",
        &format!("{path}/mcp-servers"),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(listed["data"][0]["name"], "notes");
    let server_path = format!(
        "/v1/virtual-users/me/mcp-servers/{}",
        added["id"].as_str().unwrap()
    );
    let (status, disabled) = send(
        &router,
        &token,
        "PATCH",
        &server_path,
        json!({"enabled":false}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(disabled["enabled"], false);
    // A person's server loads on demand until they choose to always load it.
    assert_eq!(added["deferred"], true);
    let (status, eager) = send(
        &router,
        &token,
        "PATCH",
        &server_path,
        json!({"deferred":false}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{eager}");
    assert_eq!(eager["deferred"], false);
    assert_eq!(eager["enabled"], false, "other fields are kept");
    let (_, read) = send(&router, &token, "GET", &server_path, Value::Null).await;
    assert_eq!(read["deferred"], false);
    assert_eq!(
        send(
            &router,
            &token,
            "GET",
            &format!(
                "/v1/virtual-users/{}/mcp-servers",
                everruns_contracts::typed_id::VirtualUserId::new()
            ),
            Value::Null
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        send(&router, &token, "DELETE", &server_path, Value::Null)
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        send(&router, &token, "GET", &server_path, Value::Null)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert!(
        server
            .db
            .revoke_virtual_user_binding(DEFAULT_ORG_ID, account.id, binding)
            .await
            .unwrap()
    );
    assert_eq!(
        send(&router, &token, "GET", "/v1/virtual-users/me", Value::Null)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn management_survives_runtime_archive_and_listing_is_filtered_and_paginated() {
    let server = crate::test_harness::TestServer::in_memory().await;
    let me: Value = server
        .get("/v1/virtual-users/me")
        .await
        .assert_status(StatusCode::OK)
        .json();
    let id = me["id"].as_str().unwrap();
    server
        .delete(&format!("/v1/virtual-users/{id}"))
        .await
        .assert_status(StatusCode::NO_CONTENT);
    server
        .patch(
            &format!("/v1/virtual-users/{id}"),
            json!({"status":"active"}),
        )
        .await
        .assert_status(StatusCode::OK);
    for name in ["paged-alex", "paged-sam"] {
        server
            .post("/v1/virtual-users", json!({"name":name,"usage":"end_user"}))
            .await
            .assert_status(StatusCode::CREATED);
    }
    server
        .post(
            "/v1/virtual-users",
            json!({"name":"paged-service","usage":"service"}),
        )
        .await
        .assert_status(StatusCode::CREATED);
    let page: Value = server
        .get("/v1/virtual-users?usage=end_user&search=paged&limit=1")
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(page["total"], 2);
    assert_eq!(page["data"].as_array().unwrap().len(), 1);
    let second: Value = server
        .get("/v1/virtual-users?usage=end_user&search=paged&limit=1&offset=1")
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_ne!(page["data"][0]["id"], second["data"][0]["id"]);
    assert_eq!(second["data"][0]["usage"], "end_user");
}

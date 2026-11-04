//! Codex OAuth builder on the wire: discovery and chat share one provider.
#![cfg(feature = "codex")]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
use everruns_contracts::{LlmCallConfig, Message, MessageRole};
use everruns_drivers::chatgpt::CodexAuth;
use everruns_drivers::codex::{self, MemoryTokenStore, TokenStore as _};
use futures::StreamExt;
use serde_json::{Value, json};
use std::sync::Arc;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn grant(expires_at: Option<i64>) -> CodexAuth {
    CodexAuth {
        access_token: "old-access".into(),
        refresh_token: Some("old-refresh".into()),
        expires_at,
        account_id: Some("acct_1".into()),
        email: None,
        client_id: Some("app_codex".into()),
        open_source: None,
    }
}

#[tokio::test]
async fn lists_models_with_oauth_bearer_and_codex_headers() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/codex/models"))
        .and(query_param("client_version", "9.9.9"))
        .and(header("authorization", "Bearer old-access"))
        .and(header("chatgpt-account-id", "acct_1"))
        .and(header("originator", "yolop"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"models":[
            {"slug":"gpt-6.1-sol","display_name":"GPT-6.1 Sol","visibility":"list"},
            {"slug":"internal","visibility":"hide"},
            {"slug":"gpt-6-luna","visibility":"list"}
        ]})))
        .expect(1)
        .mount(&server)
        .await;
    let provider = codex::driver::ephemeral(grant(None))
        .originator("yolop")
        .client_version("9.9.9")
        .base_url(format!("{}/codex", server.uri()))
        .build()
        .unwrap();
    let models = provider.list_models().await.unwrap().unwrap();
    assert_eq!(
        models
            .iter()
            .map(|m| m.model_id.as_str())
            .collect::<Vec<_>>(),
        ["gpt-6.1-sol", "gpt-6-luna"]
    );
    assert_eq!(models[0].display_name.as_deref(), Some("GPT-6.1 Sol"));
}

#[tokio::test]
async fn discovery_rotates_expired_grant_through_store_and_redacts_auth_failure() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"access_token":"new-access","refresh_token":"new-refresh","expires_in":3600}),
        ))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/models"))
        .and(header("authorization", "Bearer new-access"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"models":[{"slug":"gpt-6.1-sol","visibility":"list"}]})),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(ResponseTemplate::new(401).set_body_string("token-body-must-not-leak"))
        .mount(&server)
        .await;
    let store = Arc::new(MemoryTokenStore::new(grant(Some(0))));
    let provider = codex::driver(store.clone())
        .base_url(server.uri())
        .token_url(format!("{}/oauth/token", server.uri()))
        .build()
        .unwrap();
    let models = provider.list_models().await.unwrap().unwrap();
    assert_eq!(models[0].model_id, "gpt-6.1-sol");
    let saved = store.load().await.unwrap().unwrap();
    assert_eq!(saved.access_token, "new-access");
    assert_eq!(saved.refresh_token.as_deref(), Some("new-refresh"));
    assert_eq!(saved.account_id.as_deref(), Some("acct_1"));

    // A rejected bearer is an authentication error that omits the body.
    store
        .save(CodexAuth {
            access_token: "revoked".into(),
            expires_at: None,
            ..saved
        })
        .await
        .unwrap();
    let error = provider.list_models().await.unwrap_err();
    assert_eq!(error.http_status(), Some(401));
    assert!(error.is_auth_error(), "{error}");
    assert!(!error.to_string().contains("token-body"));
}

#[tokio::test]
async fn chat_and_discovery_share_one_authenticated_provider() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"models":[{"slug":"gpt-6.1-sol","visibility":"list"}]})),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/responses"))
        .respond_with(ResponseTemplate::new(200)
            .insert_header("content-type", "text/event-stream")
            .set_body_string("data: {\"type\":\"response.completed\",\"response\":{\"id\":\"r\",\"status\":\"completed\",\"output\":[]}}\n\n"))
        .mount(&server)
        .await;
    let provider = codex::driver::ephemeral(grant(None))
        .base_url(server.uri())
        .build()
        .unwrap();
    let model = provider.list_models().await.unwrap().unwrap()[0]
        .model_id
        .clone();
    let events: Vec<_> = provider
        .chat_completion_stream(
            vec![Message::text(MessageRole::User, "hi")],
            &LlmCallConfig::new(model),
        )
        .await
        .unwrap()
        .collect()
        .await;
    assert!(events.iter().all(Result::is_ok), "{events:?}");
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2);
    for request in &requests {
        assert_eq!(request.headers["authorization"], "Bearer old-access");
        assert_eq!(request.headers["chatgpt-account-id"], "acct_1");
        assert_eq!(request.headers["originator"], "everruns");
    }
    let body: Value = requests[1].body_json().unwrap();
    assert_eq!(body["model"], "gpt-6.1-sol");
}

#[tokio::test]
async fn plan_route_reaches_the_plan_catalog_through_the_same_builder() {
    use everruns_drivers::chatgpt::{OpenSourceGrant, oauth};
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .and(header("authorization", "Bearer old-access"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"models":[{"slug":"plan-model","visibility":"list"}]})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let mut plan = grant(None);
    plan.open_source = Some(OpenSourceGrant {
        id_token: None,
        scopes: vec![oauth::PLAN_SCOPE.into()],
        subject: None,
    });
    let provider = codex::driver::ephemeral(plan)
        .route(codex::TokenRoute::ChatGptPlan)
        .base_url(format!("{}/v1", server.uri()))
        .build()
        .unwrap();
    let models = provider.list_models().await.unwrap().unwrap();
    assert_eq!(models[0].model_id, "plan-model");
    let request = &server.received_requests().await.unwrap()[0];
    // Codex-only headers never reach the plan route.
    assert!(request.headers.get("originator").is_none());
    assert!(request.url.query().is_none());
}

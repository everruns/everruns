//! Managed Slack setup must finish before the endpoint accepts webhook traffic.

use crate::test_harness;

use std::sync::Arc;

use async_trait::async_trait;
use axum::{body::Body, http::Request};
use everruns_server::channels::slack::provisioning::SlackProvisioningSetup;
use everruns_server::domains::agent_channels::record::slack_provisioning::{
    SlackAppCredentials, SlackAppProvisioner, SlackProvisioningConnectionStatus,
    SlackProvisioningResult,
};
use everruns_server::{api, auth};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_string_contains, method, path},
};

struct Provisioner;

#[async_trait]
impl SlackAppProvisioner for Provisioner {
    async fn create_app(
        &self,
        _: i64,
        team: Option<&str>,
        manifest: &str,
    ) -> SlackProvisioningResult<SlackAppCredentials> {
        assert_eq!(team, Some("T1"));
        assert!(manifest.contains("chat:write"));
        Ok(SlackAppCredentials {
            app_id: "A1".into(),
            client_id: "c1".into(),
            client_secret: "s1".into(),
            signing_secret: "g1".into(),
        })
    }

    async fn update_permissions(
        &self,
        _: i64,
        _: Option<&str>,
        _: &str,
        scopes: &[&str],
    ) -> SlackProvisioningResult<()> {
        assert!(scopes.contains(&"reactions:write"));
        Ok(())
    }

    async fn delete_app(&self, _: i64, _: Option<&str>, _: &str) -> SlackProvisioningResult<()> {
        Ok(())
    }

    async fn connection_status(
        &self,
        _: i64,
    ) -> SlackProvisioningResult<SlackProvisioningConnectionStatus> {
        Ok(SlackProvisioningConnectionStatus {
            connected: true,
            reconnect_required: false,
        })
    }
}

#[tokio::test]
async fn draft_slack_channel_completes_oauth_without_accepting_webhooks() {
    exercise_install(test_harness::TestServer::in_memory().await).await;
}

#[tokio::test]
async fn draft_slack_channel_persists_oauth_with_postgres() {
    exercise_install(test_harness::TestServer::new().await).await;
}

async fn exercise_install(server: test_harness::TestServer) {
    let agent: Value = server
        .post(
            "/v1/agents",
            json!({"name":format!("slack-setup-{}", uuid::Uuid::now_v7().simple()), "system_prompt":"Test"}),
        )
        .await
        .assert_status(axum::http::StatusCode::CREATED)
        .json();
    let endpoint: Value = server
        .post(
            &format!("/v1/agents/{}/channels", agent["id"].as_str().unwrap()),
            json!({"channel_type":"slack", "channel_config":{}}),
        )
        .await
        .assert_status(axum::http::StatusCode::CREATED)
        .json();
    let id = endpoint["id"].as_str().unwrap();
    assert_eq!(endpoint["status"], "draft");

    let config = auth::AuthConfig::default();
    let backend = auth::BuiltinAuthBackend::new(
        config.clone(),
        server.db.clone(),
        Arc::new(everruns_server::oss_host_composition_for_grade(
            everruns_core::DeploymentGrade::Dev,
        )),
    );
    let auth = auth::AuthState::new(config, Arc::new(backend)).with_db(server.db.clone());
    let slack = everruns_server::channels::slack::events::SlackState::new(
        server.db.clone(),
        server.encryption.clone(),
        server.runner.clone(),
        everruns_server::EventDelivery::in_memory(),
        "https://example.com/api".into(),
    );
    let exchange = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth.v2.access"))
        .and(body_string_contains("code=code"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"ok":true,"app_id":"A1","access_token":"xoxb-installed","team":{"id":"T1"}}),
        ))
        .expect(1)
        .mount(&exchange)
        .await;
    Mock::given(method("POST"))
        .and(path("/oauth.v2.access"))
        .and(body_string_contains("code=refused"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"ok":false,"error":"invalid_code"})),
        )
        .expect(1)
        .mount(&exchange)
        .await;
    for (code, team, app) in [("wrong-team", "T2", "A1"), ("wrong-app", "T1", "A2")] {
        Mock::given(method("POST"))
            .and(path("/oauth.v2.access"))
            .and(body_string_contains(format!("code={code}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"ok":true,"app_id":app,"access_token":"wrong-token","team":{"id":team}}),
            ))
            .expect(1)
            .mount(&exchange)
            .await;
    }
    Mock::given(method("POST"))
        .and(path("/auth.test"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("x-oauth-scopes", "chat:write,reactions:write")
                .set_body_json(json!({"ok":true,"team_id":"T1"})),
        )
        .expect(1)
        .mount(&exchange)
        .await;
    let mut state = everruns_server::channels::slack::install::SlackInstallState::new(
        slack,
        auth,
        "https://example.com/".into(),
        SlackProvisioningSetup {
            provisioner: Some(Arc::new(Provisioner)),
            connection_manager: None,
        },
    );
    state.slack_api_base = exchange.uri();
    let router = everruns_server::channels::slack::install::routes(state);

    let begin = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/channels/{id}/slack/install"))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"team_id":"T1"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(begin.status(), axum::http::StatusCode::OK);
    let body: Value =
        serde_json::from_slice(&begin.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let consent = url::Url::parse(body["authorize_url"].as_str().unwrap()).unwrap();
    let params: std::collections::HashMap<_, _> = consent.query_pairs().into_owned().collect();
    assert!(params["scope"].contains("chat:write"));
    assert_eq!(params["team"], "T1");

    let callback = |nonce: &str| {
        Request::builder()
            .uri(format!(
                "/v1/channels/{id}/slack/oauth/callback?code=code&state={nonce}"
            ))
            .body(Body::empty())
            .unwrap()
    };
    let rejected = router
        .clone()
        .oneshot(callback("wrong-state"))
        .await
        .unwrap();
    let editor_url = format!(
        "https://example.com/agents/{}/channels/{id}",
        agent["id"].as_str().unwrap()
    );
    assert_eq!(
        rejected.headers()["location"].to_str().unwrap(),
        "https://example.com/agents?slack_install=failed"
    );
    // Every valid callback consumes its nonce, including declined consent.
    for failure in [
        "error=access_denied",
        "",
        "code=refused",
        "code=wrong-team",
        "code=wrong-app",
    ] {
        let retry = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/v1/channels/{id}/slack/install"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"team_id":"T1"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(retry.status(), axum::http::StatusCode::OK);
        let body: Value =
            serde_json::from_slice(&retry.into_body().collect().await.unwrap().to_bytes()).unwrap();
        let url = url::Url::parse(body["authorize_url"].as_str().unwrap()).unwrap();
        let nonce = url
            .query_pairs()
            .find(|(k, _)| k == "state")
            .unwrap()
            .1
            .into_owned();
        let failed = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/v1/channels/{id}/slack/oauth/callback?{failure}&state={nonce}"
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            failed.headers()["location"].to_str().unwrap(),
            format!("{editor_url}?slack_install=failed")
        );
        let replay = router.clone().oneshot(callback(&nonce)).await.unwrap();
        assert_eq!(
            replay.headers()["location"].to_str().unwrap(),
            "https://example.com/agents?slack_install=failed"
        );
    }
    let retry = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/channels/{id}/slack/install"))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"team_id":"T1"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    let body: Value =
        serde_json::from_slice(&retry.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let consent = url::Url::parse(body["authorize_url"].as_str().unwrap()).unwrap();
    let params: std::collections::HashMap<_, _> = consent.query_pairs().into_owned().collect();
    for uri in [
        format!("/v1/channels/{id}/slack/oauth/callback?error=access_denied&state=wrong"),
        format!("/v1/channels/{id}/slack/oauth/callback?code=code"),
        format!(
            "/v1/channels/missing/slack/oauth/callback?code=code&state={}",
            params["state"]
        ),
    ] {
        let failed = router
            .clone()
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(failed.status(), axum::http::StatusCode::SEE_OTHER);
        assert_eq!(
            failed.headers()["location"].to_str().unwrap(),
            "https://example.com/agents?slack_install=failed"
        );
    }
    let installed = router
        .clone()
        .oneshot(callback(&params["state"]))
        .await
        .unwrap();
    assert_eq!(
        installed.headers()["location"].to_str().unwrap(),
        format!("{editor_url}?slack_install=ok")
    );
    let replayed = router.oneshot(callback(&params["state"])).await.unwrap();
    assert_eq!(
        replayed.headers()["location"].to_str().unwrap(),
        "https://example.com/agents?slack_install=failed"
    );

    let (_, stored) =
        api::channel_ingress::resolve_channel(&server.db, server.encryption.as_ref(), id)
            .await
            .unwrap()
            .unwrap();
    let config: everruns_server::domains::agent_channels::record::slack_channel::SlackChannelConfig =
        serde_json::from_value(stored.channel_config).unwrap();
    assert_eq!(config.bot_token, "xoxb-installed");
    assert_eq!(config.team_id.as_deref(), Some("T1"));
    assert!(config.provisioned_app.unwrap().install_state.is_none());
    assert_eq!(
        stored.status,
        everruns_server::domains::agent_channels::record::ChannelStatus::Draft
    );
    server
        .post(
            &format!("/v1/channels/{id}/slack/events"),
            json!({"type":"url_verification","challenge":"test"}),
        )
        .await
        .assert_status(axum::http::StatusCode::NOT_FOUND);
}

/// A workspace connects by one Slack consent through the deployment's manager
/// app, and the callback refuses anything whose state it did not mint (EVE-1102).
#[tokio::test]
async fn manager_app_consent_connects_a_workspace_without_a_pasted_token() {
    use everruns_server::channels::slack::provisioning::{SlackApiProvisioner, SlackManagerApp};

    let server = test_harness::TestServer::new().await;
    let encryption = server.encryption.clone().expect("test encryption");
    let config = auth::AuthConfig::default();
    let backend = auth::BuiltinAuthBackend::new(
        config.clone(),
        server.db.clone(),
        Arc::new(everruns_server::oss_host_composition_for_grade(
            everruns_core::DeploymentGrade::Dev,
        )),
    );
    let auth = auth::AuthState::new(config, Arc::new(backend)).with_db(server.db.clone());
    let slack = everruns_server::channels::slack::events::SlackState::new(
        server.db.clone(),
        server.encryption.clone(),
        server.runner.clone(),
        everruns_server::EventDelivery::in_memory(),
        "https://example.com/api".into(),
    );
    let slack_api = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth.v2.access"))
        .and(body_string_contains("code=granted"))
        .and(body_string_contains("client_id=111.222"))
        .and(body_string_contains(
            "redirect_uri=https%3A%2F%2Fexample.com%2Fapi%2Fv1%2Fslack%2Fworkspaces%2Foauth%2Fcallback",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true,
            "app_id": "AMANAGER",
            "authed_user": {
                "id": "U1",
                "scope": "app_configurations:read,app_configurations:write",
                "access_token": "xoxe.xoxp-grant",
                "refresh_token": "xoxe-1-grant",
                "expires_in": 43200,
                "token_type": "user"
            },
            "team": {"id": "T0123", "name": "Acme"}
        })))
        .expect(1)
        .mount(&slack_api)
        .await;
    let manager = Arc::new(
        SlackApiProvisioner::with_api_base(server.db.clone(), encryption, &slack_api.uri())
            .unwrap()
            .with_manager_app(Some(SlackManagerApp {
                client_id: "111.222".into(),
                client_secret: "manager-secret".into(),
            })),
    );
    let state = everruns_server::channels::slack::install::SlackInstallState::new(
        slack,
        auth,
        "https://example.com/".into(),
        SlackProvisioningSetup {
            provisioner: Some(manager.clone()),
            connection_manager: Some(manager),
        },
    );
    let router = everruns_server::channels::slack::install::routes(state);
    let get = |uri: String| Request::builder().uri(uri).body(Body::empty()).unwrap();
    let json_of = |bytes: axum::body::Bytes| -> Value { serde_json::from_slice(&bytes).unwrap() };

    let capability = router
        .clone()
        .oneshot(get("/v1/slack/install".into()))
        .await
        .unwrap();
    let capability = json_of(capability.into_body().collect().await.unwrap().to_bytes());
    assert_eq!(capability["manager_app"], true);
    assert_eq!(capability["connected"], false);

    let begin = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/slack/workspaces/oauth")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(begin.status(), axum::http::StatusCode::OK);
    let body = json_of(begin.into_body().collect().await.unwrap().to_bytes());
    let consent = url::Url::parse(body["authorize_url"].as_str().unwrap()).unwrap();
    let params: std::collections::HashMap<_, _> = consent.query_pairs().into_owned().collect();
    assert_eq!(params["client_id"], "111.222");
    assert_eq!(params["scope"], "");
    assert_eq!(
        params["user_scope"],
        "app_configurations:read,app_configurations:write"
    );
    let sealed = &params["state"];

    let location = |response: axum::response::Response| {
        response.headers()["location"].to_str().unwrap().to_string()
    };
    let settings = "https://example.com/settings/slack";
    // A state this deployment did not mint reveals nothing, not even Slack's code.
    let forged = router
        .clone()
        .oneshot(get(
            "/v1/slack/workspaces/oauth/callback?code=granted&state=forged&error=x".into(),
        ))
        .await
        .unwrap();
    assert_eq!(location(forged), format!("{settings}?slack_connect=failed"));
    let declined = router
        .clone()
        .oneshot(get(format!(
            "/v1/slack/workspaces/oauth/callback?error=access_denied&state={sealed}"
        )))
        .await
        .unwrap();
    assert_eq!(
        location(declined),
        format!("{settings}?slack_connect=failed&slack_error=access_denied")
    );
    let connected = router
        .clone()
        .oneshot(get(format!(
            "/v1/slack/workspaces/oauth/callback?code=granted&state={sealed}"
        )))
        .await
        .unwrap();
    assert_eq!(location(connected), format!("{settings}?slack_connect=ok"));

    let workspaces = router
        .clone()
        .oneshot(get("/v1/slack/workspaces".into()))
        .await
        .unwrap();
    let workspaces = json_of(workspaces.into_body().collect().await.unwrap().to_bytes());
    assert_eq!(workspaces.as_array().unwrap().len(), 1);
    assert_eq!(workspaces[0]["team_id"], "T0123");
    assert_eq!(workspaces[0]["team_name"], "Acme");
    assert_eq!(workspaces[0]["status"], "connected");
    assert_eq!(workspaces[0]["connected_with"], "manager_app");
    let capability = router
        .oneshot(get("/v1/slack/install".into()))
        .await
        .unwrap();
    let capability = json_of(capability.into_body().collect().await.unwrap().to_bytes());
    assert_eq!(capability["connected"], true);
}

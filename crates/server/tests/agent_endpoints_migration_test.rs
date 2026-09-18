//! Tenancy and reuse evidence for re-parenting channels from App to Agent
//! (EVE-1003, migration 135).
//!
//! The migration moves `app_channels` rows into `agent_endpoints`, owned by an
//! Agent, and leaves `app_channels` behind as a compatibility view. The risk it
//! carries is not that endpoints stop being readable — it is that the
//! session-ownership anchor moves and `shared_session` reuse silently stops
//! keying on the values that stop cross-org and cross-app adoption.
//!
//! These tests pin the reuse keys (org + app + owner) and the tag-containment
//! semantics (`tags @> $tags`) against the real repository lookups, so a later
//! phase that re-homes the tags cannot quietly relax them.
//!
//! See knowledge/integrations/agent-exposure.md "Invariants that must not move",
//! knowledge/integrations/app-invocation-channels.md, and TM-AUTHZ-009 /
//! TM-A2A-007.
//!
//! Run with: cargo test -p everruns-server --test agent_endpoints_migration_test -- --test-threads=1

mod test_harness;
use std::sync::Arc;

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode},
    response::Response,
};
use everruns_durable::InMemoryWorkflowEventStore;

use everruns_platform::ChannelType;
use everruns_provider::typed_id::PrincipalId;
use everruns_server::EventDelivery;
use everruns_server::api;
use everruns_server::domains::apps::{hash_a2a_api_key, hash_app_api_key};
use everruns_server::storage::Database;
use everruns_server::storage::StorageBackend;
use everruns_worker::{RunnerBackend, create_runner_with_backend};
use hmac::{Hmac, KeyInit, Mac};
use serde_json::{Value, json};
use sha2::Sha256;
use sqlx::{PgPool, Row, postgres::PgPoolOptions};
use test_harness::get_database_url;
use tower::ServiceExt;
use uuid::Uuid;

async fn pool() -> PgPool {
    PgPool::connect(&get_database_url())
        .await
        .expect("Failed to connect to PostgreSQL")
}

#[tokio::test]
async fn legacy_ingress_routes_work_with_apps_and_compatibility_view_unreadable() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("everruns_server=error")
        .with_test_writer()
        .try_init();
    let pool = pool().await;
    let fixture = seed(&pool, "endpoint-no-app-reads", "published").await;
    let ag_ui_token = "migration-ag-ui-token";
    let public_chat_token = "migration-public-chat-token";
    let fcp_token = "migration-fcp-token";
    let webhook_token = "migration-webhook-token";
    let a2a_key = "evra2a_migration_key";
    let api_key = "evr_app_migration_key";
    let slack_signing_secret = "migration-slack-signing-secret";
    let migrated_webhook_token = "migration-trigger-webhook-token";
    seed_ingress_endpoint(
        &pool,
        &fixture,
        ChannelType::AgUi,
        json!({"anonymous": true, "token": ag_ui_token}),
    )
    .await;
    let migrated_webhook_id =
        seed_migrated_webhook_trigger(&pool, &fixture, migrated_webhook_token).await;
    seed_ingress_endpoint(
        &pool,
        &fixture,
        ChannelType::PublicChat,
        json!({"anonymous": true, "token": public_chat_token}),
    )
    .await;
    seed_ingress_endpoint(
        &pool,
        &fixture,
        ChannelType::Fcp,
        json!({
            "anonymous": false,
            "token": fcp_token,
            "response_timeout_seconds": 1
        }),
    )
    .await;
    let webhook_id = seed_ingress_endpoint(
        &pool,
        &fixture,
        ChannelType::Webhook,
        json!({
            "token": webhook_token,
            "message": "{{webhook.body}}",
            "session_mode": "session_per_invocation"
        }),
    )
    .await;
    let a2a_id = seed_ingress_endpoint(
        &pool,
        &fixture,
        ChannelType::A2a,
        json!({
            "api_key_hash": hash_a2a_api_key(a2a_key),
            "api_key_prefix": "evra2a_test...",
            "message": "{{a2a.text}}",
            "session_mode": "session_per_invocation"
        }),
    )
    .await;
    let api_id = seed_ingress_endpoint(
        &pool,
        &fixture,
        ChannelType::ApiEndpoint,
        json!({
            "api_key_hash": hash_app_api_key(api_key),
            "api_key_prefix": "evr_app_test...",
            "session_mode": "session_per_invocation"
        }),
    )
    .await;
    seed_ingress_endpoint(
        &pool,
        &fixture,
        ChannelType::Schedule,
        json!({
            "cron_expression": "0 * * * *",
            "timezone": "UTC",
            "message": "scheduled"
        }),
    )
    .await;
    sqlx::query(
        "UPDATE agent_endpoints
         SET channel_config = $1
         WHERE id = $2",
    )
    .bind(json!({
        "signing_secret": slack_signing_secret,
        "bot_token": "xoxb-migration-test"
    }))
    .bind(fixture.endpoint_id)
    .execute(&pool)
    .await
    .expect("configure Slack endpoint credentials");

    let role = format!("ingress_no_apps_{}", Uuid::new_v4().simple());
    let create_role = format!("CREATE ROLE {role} NOLOGIN");
    sqlx::query(sqlx::AssertSqlSafe(create_role.as_str()))
        .execute(&pool)
        .await
        .expect("create restricted ingress role");
    let grants = format!(
        "GRANT USAGE ON SCHEMA public TO {role};
         GRANT SELECT ON
             organizations, agents, agent_endpoints, agent_triggers,
             harnesses, harness_capabilities, agent_capabilities, agent_versions,
             principals, users, sessions, workspaces, session_participants,
             events, event_sequences, images, memories, models
         TO {role};
         GRANT INSERT ON
             sessions, workspaces, session_participants, events, images,
             event_sequences, memories, reporting_outbox, audit_logs
         TO {role};
         GRANT UPDATE ON sessions, agent_endpoints, event_sequences TO {role};"
    );
    sqlx::raw_sql(sqlx::AssertSqlSafe(grants.as_str()))
        .execute(&pool)
        .await
        .expect("grant endpoint-only ingress reads");

    let role_for_connect = role.clone();
    let restricted_pool = PgPoolOptions::new()
        .max_connections(1)
        .after_connect(move |connection, _metadata| {
            let statement = format!("SET ROLE {}", role_for_connect);
            Box::pin(async move {
                sqlx::query(sqlx::AssertSqlSafe(statement.as_str()))
                    .execute(connection)
                    .await?;
                Ok(())
            })
        })
        .connect(&get_database_url())
        .await
        .expect("connect with restricted ingress role");
    let restricted_db = Arc::new(StorageBackend::Postgres(Database::new(
        restricted_pool.clone(),
    )));

    assert!(
        sqlx::query("SELECT 1 FROM apps")
            .execute(&restricted_pool)
            .await
            .is_err(),
        "the regression role must not be able to read apps"
    );
    assert!(
        sqlx::query("SELECT 1 FROM app_channels")
            .execute(&restricted_pool)
            .await
            .is_err(),
        "the regression role must not be able to read app_channels"
    );
    let router = ingress_router(restricted_db).await;
    let app_id = &fixture.app_public_id;
    let initial_session_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM sessions WHERE app_id = $1")
            .bind(fixture.app_id)
            .fetch_one(&pool)
            .await
            .expect("count initial App sessions");
    let ag_ui_body = serde_json::to_vec(&json!({
        "threadId": Uuid::new_v4().to_string(),
        "runId": Uuid::new_v4().to_string(),
        "state": {},
        "messages": [{
            "id": Uuid::new_v4().to_string(),
            "role": "user",
            "content": "legacy AG-UI alias"
        }],
        "tools": [],
        "context": [],
        "forwardedProps": {}
    }))
    .expect("serialize AG-UI request");

    let ag_ui_authorization = format!("Bearer {ag_ui_token}");
    assert_route_status(
        &router,
        Method::POST,
        &format!("/v1/apps/{app_id}/ag-ui"),
        &[
            ("content-type", "application/json"),
            ("authorization", &ag_ui_authorization),
        ],
        &ag_ui_body,
        StatusCode::OK,
    )
    .await;

    let (image_content_type, image_body) = png_multipart_body();
    assert_route_status(
        &router,
        Method::POST,
        &format!("/v1/apps/{app_id}/ag-ui/images"),
        &[
            ("content-type", &image_content_type),
            ("authorization", &ag_ui_authorization),
        ],
        &image_body,
        StatusCode::CREATED,
    )
    .await;
    assert_route_status(
        &router,
        Method::GET,
        &format!("/v1/apps/{app_id}/public-chat/config"),
        &[],
        b"",
        StatusCode::OK,
    )
    .await;
    let public_chat_body = serde_json::to_vec(&json!({
        "threadId": Uuid::new_v4().to_string(),
        "runId": Uuid::new_v4().to_string(),
        "state": {},
        "messages": [{
            "id": Uuid::new_v4().to_string(),
            "role": "user",
            "content": "legacy Public Chat alias"
        }],
        "tools": [],
        "context": [],
        "forwardedProps": {}
    }))
    .expect("serialize Public Chat request");
    assert_route_status(
        &router,
        Method::POST,
        &format!("/v1/apps/{app_id}/public-chat"),
        &[
            ("content-type", "application/json"),
            ("x-everruns-public-chat-token", public_chat_token),
        ],
        &public_chat_body,
        StatusCode::OK,
    )
    .await;
    assert_route_status(
        &router,
        Method::GET,
        &format!("/v1/apps/{app_id}/fcp"),
        &[],
        b"",
        StatusCode::OK,
    )
    .await;
    let fcp_authorization = format!("Bearer {fcp_token}");
    let fcp_response = request_route(
        &router,
        Method::POST,
        &format!("/v1/apps/{app_id}/fcp"),
        &[
            ("content-type", "text/plain"),
            ("authorization", &fcp_authorization),
        ],
        b"hello",
    )
    .await;
    assert_eq!(
        fcp_response.status(),
        StatusCode::GATEWAY_TIMEOUT,
        "authenticated FCP alias must reach its bounded downstream timeout"
    );
    assert!(
        fcp_response.headers().contains_key("set-cookie"),
        "FCP must create a session before its downstream timeout"
    );
    assert_route_status(
        &router,
        Method::GET,
        &format!("/v1/apps/{app_id}/slack/manifest"),
        &[],
        b"",
        StatusCode::OK,
    )
    .await;

    let slack_event_body = serde_json::to_vec(&json!({
        "type": "url_verification",
        "challenge": "migration-challenge"
    }))
    .expect("serialize Slack event");
    let (slack_event_timestamp, slack_event_signature) =
        sign_slack_request(slack_signing_secret, &slack_event_body);
    let slack_event_response = request_route(
        &router,
        Method::POST,
        &format!("/v1/apps/{app_id}/slack/events"),
        &[
            ("content-type", "application/json"),
            ("x-slack-request-timestamp", &slack_event_timestamp),
            ("x-slack-signature", &slack_event_signature),
        ],
        &slack_event_body,
    )
    .await;
    assert_eq!(slack_event_response.status(), StatusCode::OK);
    assert_eq!(
        response_json(slack_event_response).await["challenge"],
        "migration-challenge"
    );

    let slack_interactivity_body = url::form_urlencoded::Serializer::new(String::new())
        .append_pair(
            "payload",
            &json!({
                "type": "view_submission",
                "user": {"id": "U_MIGRATION"}
            })
            .to_string(),
        )
        .finish()
        .into_bytes();
    let (slack_interactivity_timestamp, slack_interactivity_signature) =
        sign_slack_request(slack_signing_secret, &slack_interactivity_body);
    let slack_interactivity_response = request_route(
        &router,
        Method::POST,
        &format!("/v1/apps/{app_id}/slack/interactivity"),
        &[
            ("content-type", "application/x-www-form-urlencoded"),
            ("x-slack-request-timestamp", &slack_interactivity_timestamp),
            ("x-slack-signature", &slack_interactivity_signature),
        ],
        &slack_interactivity_body,
    )
    .await;
    assert_eq!(slack_interactivity_response.status(), StatusCode::OK);
    assert_eq!(
        response_json(slack_interactivity_response).await["ok"],
        true
    );

    let webhook_response = request_route(
        &router,
        Method::POST,
        &format!("/v1/apps/{app_id}/webhooks/{webhook_id}"),
        &[
            ("content-type", "application/json"),
            ("x-everruns-webhook-token", webhook_token),
        ],
        br#"{"event":"migration"}"#,
    )
    .await;
    assert_eq!(webhook_response.status(), StatusCode::ACCEPTED);
    let webhook_result = response_json(webhook_response).await;
    assert_eq!(webhook_result["accepted"], true);
    assert_eq!(webhook_result["created_session"], true);
    let migrated_webhook_headers = [("x-everruns-webhook-token", migrated_webhook_token)];
    assert_route_status(
        &router,
        Method::POST,
        &format!("/v1/e/{migrated_webhook_id}/webhook"),
        &migrated_webhook_headers,
        br#"{"event":"canonical"}"#,
        StatusCode::ACCEPTED,
    )
    .await;
    assert_route_status(
        &router,
        Method::POST,
        &format!("/v1/apps/{app_id}/webhooks/{migrated_webhook_id}"),
        &migrated_webhook_headers,
        br#"{"event":"legacy"}"#,
        StatusCode::ACCEPTED,
    )
    .await;
    assert_route_status(
        &router,
        Method::POST,
        &format!("/v1/apps/app_wrong_alias/webhooks/{migrated_webhook_id}"),
        &migrated_webhook_headers,
        br#"{"event":"mismatch"}"#,
        StatusCode::NOT_FOUND,
    )
    .await;
    assert_route_status(
        &router,
        Method::GET,
        &format!("/v1/apps/{app_id}/a2a/{a2a_id}/.well-known/agent-card.json"),
        &[],
        b"",
        StatusCode::OK,
    )
    .await;

    let a2a_body = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": "migration-request",
        "method": "message/send",
        "params": {
            "message": {
                "role": "user",
                "messageId": "migration-message",
                "parts": [{"kind": "text", "text": "legacy A2A alias"}]
            }
        }
    }))
    .expect("serialize A2A request");
    let a2a_authorization = format!("Bearer {a2a_key}");
    let a2a_response = request_route(
        &router,
        Method::POST,
        &format!("/v1/apps/{app_id}/a2a/{a2a_id}"),
        &[
            ("content-type", "application/json"),
            ("authorization", &a2a_authorization),
        ],
        &a2a_body,
    )
    .await;
    assert_eq!(a2a_response.status(), StatusCode::OK);
    let a2a_result = response_json(a2a_response).await;
    assert_eq!(a2a_result["result"]["status"]["state"], "submitted");
    assert!(a2a_result["result"]["contextId"].as_str().is_some());

    let api_authorization = format!("Bearer {api_key}");
    let api_create_response = request_route(
        &router,
        Method::POST,
        &format!("/v1/apps/{app_id}/api/{api_id}/sessions"),
        &[
            ("content-type", "application/json"),
            ("authorization", &api_authorization),
        ],
        br#"{"message":"legacy API alias"}"#,
    )
    .await;
    assert_eq!(api_create_response.status(), StatusCode::CREATED);
    let api_session = response_json(api_create_response).await;
    assert_eq!(api_session["created_session"], true);
    let api_session_id = api_session["session_id"]
        .as_str()
        .expect("API alias session id");
    assert_route_status(
        &router,
        Method::GET,
        &format!("/v1/apps/{app_id}/api/{api_id}/sessions/{api_session_id}"),
        &[("authorization", &api_authorization)],
        b"",
        StatusCode::OK,
    )
    .await;
    assert_route_status(
        &router,
        Method::POST,
        &format!("/v1/apps/{app_id}/api/{api_id}/sessions/{api_session_id}/messages"),
        &[
            ("content-type", "application/json"),
            ("authorization", &api_authorization),
        ],
        br#"{"message":"legacy API follow-up"}"#,
        StatusCode::ACCEPTED,
    )
    .await;
    assert_route_status(
        &router,
        Method::POST,
        &format!("/v1/apps/{app_id}/api/{api_id}/sessions/{api_session_id}/cancel"),
        &[("authorization", &api_authorization)],
        b"",
        StatusCode::OK,
    )
    .await;

    let final_session_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM sessions WHERE app_id = $1")
            .bind(fixture.app_id)
            .fetch_one(&pool)
            .await
            .expect("count final App sessions");
    assert!(
        final_session_count >= initial_session_count + 6,
        "AG-UI, Public Chat, FCP, Webhook, A2A, and API aliases must create sessions"
    );

    restricted_pool.close().await;
    let drop_role = format!(
        "DROP OWNED BY {role};
         DROP ROLE {role};"
    );
    sqlx::raw_sql(sqlx::AssertSqlSafe(drop_role.as_str()))
        .execute(&pool)
        .await
        .expect("drop restricted ingress role");
}

async fn seed_migrated_webhook_trigger(pool: &PgPool, fixture: &Fixture, token: &str) -> String {
    let ingress_id = format!("appchan_{}", hex32());
    sqlx::query(
        "INSERT INTO agent_triggers (
             id, org_id, agent_id, trigger_type, ingress_id, config, enabled,
             execution_harness_id, execution_owner_principal_id, execution_app_id,
             execution_app_public_id, execution_app_name,
             execution_agent_version_policy, execution_agent_version_id
         )
         SELECT $1, app.org_id, app.agent_id, 'webhook', $2, $3, true,
                app.harness_id, app.owner_principal_id, app.id, app.public_id, app.name,
                app.agent_version_policy, app.agent_version_id
         FROM apps AS app
         WHERE app.id = $4",
    )
    .bind(Uuid::now_v7())
    .bind(&ingress_id)
    .bind(json!({
        "token": token,
        "message": "{{webhook.body}}",
        "session_mode": "session_per_invocation"
    }))
    .bind(fixture.app_id)
    .execute(pool)
    .await
    .expect("seed migrated webhook trigger");
    ingress_id
}
async fn seed_ingress_endpoint(
    pool: &PgPool,
    fixture: &Fixture,
    channel_type: ChannelType,
    channel_config: Value,
) -> String {
    let endpoint_public_id = format!("appchan_{}", hex32());
    sqlx::query(
        "INSERT INTO agent_endpoints (
             id, agent_id, app_id, legacy_app_public_id, public_id, channel_type,
             channel_config, enabled, status, agent_version_policy, owner_principal_id
         )
         VALUES ($1, $2, $3, $4, $5, $6, $7, true, 'live', 'default', $8)",
    )
    .bind(Uuid::now_v7())
    .bind(fixture.agent_id)
    .bind(fixture.app_id)
    .bind(&fixture.app_public_id)
    .bind(&endpoint_public_id)
    .bind(channel_type.to_string())
    .bind(channel_config)
    .bind(fixture.owner_principal_id)
    .execute(pool)
    .await
    .expect("seed ingress endpoint");
    endpoint_public_id
}

async fn ingress_router(db: Arc<StorageBackend>) -> Router {
    let runner = create_runner_with_backend(RunnerBackend::SharedInMemory(Arc::new(
        InMemoryWorkflowEventStore::new(),
    )))
    .await
    .expect("create ingress test runner");
    let event_delivery = EventDelivery::in_memory();
    let sse_tracker = Arc::new(api::sse::SseConnectionTracker::new(
        api::sse::SseConnectionLimits::default(),
    ));
    let ag_ui_state = api::ag_ui::AgUiState::new(
        db.clone(),
        None,
        runner.clone(),
        false,
        event_delivery.clone(),
        sse_tracker.clone(),
        api::channel_rate_limit::ChannelRateLimiter::in_memory("migration-ag-ui"),
    );
    let public_chat_state = api::ag_ui::AgUiState::new(
        db.clone(),
        None,
        runner.clone(),
        false,
        event_delivery.clone(),
        sse_tracker.clone(),
        api::channel_rate_limit::ChannelRateLimiter::in_memory("migration-public-chat"),
    )
    .with_public_chat_enabled(true);
    let fcp_state = api::fcp::FcpState::new(
        db.clone(),
        None,
        runner.clone(),
        false,
        event_delivery.clone(),
        api::channel_rate_limit::ChannelRateLimiter::in_memory("migration-fcp"),
    );
    let slack_state = api::slack_events::SlackState::new(
        db.clone(),
        None,
        runner.clone(),
        None,
        false,
        event_delivery.clone(),
        "https://example.com/api".to_string(),
    );
    let webhook_state = api::app_webhooks::AppWebhookState::new(
        db.clone(),
        None,
        runner.clone(),
        false,
        event_delivery.clone(),
        api::channel_rate_limit::ChannelRateLimiter::in_memory("migration-webhook"),
    );
    let a2a_state = api::app_a2a::AppA2aState::new(
        db.clone(),
        None,
        runner.clone(),
        false,
        event_delivery.clone(),
        sse_tracker,
        api::channel_rate_limit::ChannelRateLimiter::in_memory("migration-a2a"),
        api::a2a_signing::A2aReplayStore::in_memory(),
    );
    let api_state = api::app_api::AppApiState::new(
        db,
        None,
        runner,
        false,
        event_delivery,
        api::channel_rate_limit::ChannelRateLimiter::in_memory("migration-api"),
    );

    Router::new()
        .merge(api::ag_ui::routes(ag_ui_state))
        .merge(api::public_chat::routes(public_chat_state))
        .merge(api::fcp::routes(fcp_state))
        .merge(api::slack_events::routes(slack_state))
        .merge(api::app_webhooks::routes(webhook_state))
        .merge(api::app_a2a::routes(a2a_state))
        .merge(api::app_api::routes(api_state))
}

async fn assert_route_status(
    router: &Router,
    method: Method,
    uri: &str,
    headers: &[(&str, &str)],
    body: &[u8],
    expected: StatusCode,
) {
    let response = request_route(router, method, uri, headers, body).await;
    assert_eq!(response.status(), expected, "{uri}");
}

async fn request_route(
    router: &Router,
    method: Method,
    uri: &str,
    headers: &[(&str, &str)],
    body: &[u8],
) -> Response {
    let mut request = Request::builder().method(method).uri(uri);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    router
        .clone()
        .oneshot(
            request
                .body(Body::from(body.to_vec()))
                .expect("build ingress request"),
        )
        .await
        .expect("run ingress request")
}

async fn response_json(response: Response) -> Value {
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    serde_json::from_slice(&body).expect("parse JSON response")
}

fn sign_slack_request(secret: &str, body: &[u8]) -> (String, String) {
    let timestamp = chrono::Utc::now().timestamp().to_string();
    let basestring = format!("v0:{timestamp}:{}", String::from_utf8_lossy(body));
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("valid HMAC key");
    mac.update(basestring.as_bytes());
    let signature = format!("v0={}", hex::encode(mac.finalize().into_bytes()));
    (timestamp, signature)
}

fn png_multipart_body() -> (String, Vec<u8>) {
    let boundary = format!("migration-{}", Uuid::new_v4().simple());
    let mut body = format!(
        "--{boundary}\r\n\
         Content-Disposition: form-data; name=\"file\"; filename=\"migration.png\"\r\n\
         Content-Type: image/png\r\n\r\n"
    )
    .into_bytes();
    body.extend_from_slice(&[
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90,
        0x77, 0x53, 0xde, 0x00, 0x00, 0x00, 0x0c, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0x60,
        0x60, 0x00, 0x00, 0x00, 0x02, 0x00, 0x01, 0xe2, 0x21, 0xbc, 0x33, 0x00, 0x00, 0x00, 0x00,
        0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ]);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

/// One isolated org + agent + app + endpoint + owner principal, seeded directly
/// so the test exercises the migrated schema rather than the App create path.
struct Fixture {
    org_id: i64,
    app_id: Uuid,
    app_public_id: String,
    agent_id: Uuid,
    endpoint_id: Uuid,
    owner_principal_id: Uuid,
    workspace_id: Uuid,
}

fn hex32() -> String {
    Uuid::new_v4().simple().to_string()
}

async fn seed(pool: &PgPool, org_name: &str, app_status: &str) -> Fixture {
    let org_id: i64 = sqlx::query_scalar(
        "INSERT INTO organizations (public_id, name) VALUES ($1, $2) RETURNING org_id",
    )
    .bind(format!("org_{}", hex32()))
    .bind(format!("{org_name}-{}", hex32()))
    .fetch_one(pool)
    .await
    .expect("seed organization");

    let owner_principal_id = Uuid::now_v7();
    sqlx::query("INSERT INTO principals (id, public_id, org_id, kind) VALUES ($1, $2, $3, 'user')")
        .bind(owner_principal_id)
        .bind(format!("principal_{}", hex32()))
        .bind(org_id)
        .execute(pool)
        .await
        .expect("seed principal");

    let workspace_id = Uuid::now_v7();
    sqlx::query("INSERT INTO workspaces (id, org_id, public_id, name) VALUES ($1, $2, $3, $4)")
        .bind(workspace_id)
        .bind(org_id)
        .bind(format!("wsp_{}", hex32()))
        .bind(format!("workspace-{}", hex32()))
        .execute(pool)
        .await
        .expect("seed workspace");

    let harness_id = Uuid::now_v7();
    sqlx::query("INSERT INTO harnesses (id, org_id, name) VALUES ($1, $2, $3)")
        .bind(harness_id)
        .bind(org_id)
        .bind(format!("harness-{}", hex32()))
        .execute(pool)
        .await
        .expect("seed harness");

    let agent_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO agents (id, org_id, public_id, name, system_prompt, harness_id)
         VALUES ($1, $2, $3, $4, '', $5)",
    )
    .bind(agent_id)
    .bind(org_id)
    .bind(format!("agent_{}", hex32()))
    .bind(format!("agent-{}", hex32()))
    .bind(harness_id)
    .execute(pool)
    .await
    .expect("seed agent");

    let app_id = Uuid::now_v7();
    let app_public_id = format!("app_{}", hex32());
    sqlx::query(
        "INSERT INTO apps (id, org_id, public_id, name, harness_id, agent_id, status,
                           agent_version_policy, owner_principal_id, channel_type, channel_config)
         VALUES ($1, $2, $3, $4, $5, $6, $7, 'default', $8, 'slack', '{}'::jsonb)",
    )
    .bind(app_id)
    .bind(org_id)
    .bind(&app_public_id)
    .bind(format!("app-{}", hex32()))
    .bind(harness_id)
    .bind(agent_id)
    .bind(app_status)
    .bind(owner_principal_id)
    .execute(pool)
    .await
    .expect("seed app");

    let endpoint_id = Uuid::now_v7();
    let endpoint_public_id = format!("appchan_{}", hex32());
    sqlx::query(
        "INSERT INTO agent_endpoints (id, agent_id, app_id, legacy_app_public_id, public_id, channel_type,
                                      channel_config, enabled, status, agent_version_policy,
                                      owner_principal_id)
         VALUES ($1, $2, $3, $4, $5, 'slack', '{}'::jsonb, true, 'live', 'default', $6)",
    )
    .bind(endpoint_id)
    .bind(agent_id)
    .bind(app_id)
    .bind(&app_public_id)
    .bind(&endpoint_public_id)
    .bind(owner_principal_id)
    .execute(pool)
    .await
    .expect("seed endpoint");

    Fixture {
        org_id,
        app_id,
        app_public_id,
        agent_id,
        endpoint_id,
        owner_principal_id,
        workspace_id,
    }
}

async fn seed_session(pool: &PgPool, fixture: &Fixture, owner: Uuid, tags: &[&str]) -> Uuid {
    let session_id = Uuid::now_v7();
    let tags: Vec<String> = tags.iter().map(|t| (*t).to_string()).collect();
    sqlx::query(
        "INSERT INTO sessions (id, org_id, workspace_id, app_id, owner_principal_id, tags, status)
         VALUES ($1, $2, $3, $4, $5, $6, 'started')",
    )
    .bind(session_id)
    .bind(fixture.org_id)
    .bind(fixture.workspace_id)
    .bind(fixture.app_id)
    .bind(owner)
    .bind(&tags)
    .execute(pool)
    .await
    .expect("seed session");
    session_id
}

/// Acceptance: every `app_channels` row resolves to an `agent_endpoints` row
/// with the same id and a non-null `agent_id`. Asserted over the whole database
/// so any rows an earlier test or the migration backfill produced are covered,
/// not just the ones this test seeds.
#[tokio::test]
async fn every_app_channel_has_an_endpoint_with_an_agent() {
    let pool = pool().await;
    let _fixture = seed(&pool, "endpoint-invariant", "published").await;

    let orphans: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
         FROM app_channels ac
         LEFT JOIN agent_endpoints ae ON ae.id = ac.id
         WHERE ae.id IS NULL OR ae.agent_id IS NULL",
    )
    .fetch_one(&pool)
    .await
    .expect("count orphaned channels");

    assert_eq!(
        orphans, 0,
        "every app_channels row must have an agent_endpoints row with the same id and an agent_id"
    );
}

/// The compatibility view must expose the former `app_channels` shape verbatim,
/// because App read paths still select these columns by name.
#[tokio::test]
async fn app_channels_view_mirrors_the_endpoint_row() {
    let pool = pool().await;
    let fixture = seed(&pool, "endpoint-view", "published").await;

    let row = sqlx::query(
        "SELECT id, app_id, public_id, channel_type, channel_config,
                channel_config_encrypted, durable_schedule_id, enabled, created_at, updated_at
         FROM app_channels WHERE id = $1",
    )
    .bind(fixture.endpoint_id)
    .fetch_one(&pool)
    .await
    .expect("read channel through the compatibility view");

    assert_eq!(row.get::<Uuid, _>("id"), fixture.endpoint_id);
    assert_eq!(row.get::<Uuid, _>("app_id"), fixture.app_id);
    assert_eq!(row.get::<String, _>("channel_type"), "slack");
    assert!(row.get::<bool, _>("enabled"));
}

/// `shared_session` reuse keys on org + app + owner + tag containment. After the
/// re-parenting it must still adopt the session it adopted before.
#[tokio::test]
async fn shared_session_reuse_still_matches_after_reparenting() {
    let pool = pool().await;
    let db = Database::new(pool.clone());
    let fixture = seed(&pool, "endpoint-reuse", "published").await;

    let routing_tags = [
        "__internal:app_channel:shared".to_string(),
        format!("app_channel:{}", fixture.endpoint_id),
    ];
    let expected = seed_session(
        &pool,
        &fixture,
        fixture.owner_principal_id,
        &[
            "__internal:app_channel:shared",
            &format!("app_channel:{}", fixture.endpoint_id),
            "slack:channel:C123",
        ],
    )
    .await;

    let found = db
        .find_app_session_by_tags_and_owner(
            fixture.org_id,
            fixture.app_id,
            PrincipalId::from(fixture.owner_principal_id),
            &routing_tags,
        )
        .await
        .expect("reuse lookup");

    assert_eq!(
        found.map(|s| s.id.uuid()),
        Some(expected),
        "the shared session must still be adopted after channels are re-parented onto the agent"
    );
}

/// TM-AUTHZ-009: a session carrying the same surface tags but owned by a
/// different principal must never be adopted. This is what makes
/// `owner_principal_id` load-bearing on the endpoint rather than collapsible
/// onto the agent.
#[tokio::test]
async fn session_owned_by_another_principal_is_not_adopted() {
    let pool = pool().await;
    let db = Database::new(pool.clone());
    let fixture = seed(&pool, "endpoint-owner", "published").await;

    let other_principal = Uuid::now_v7();
    sqlx::query("INSERT INTO principals (id, public_id, org_id, kind) VALUES ($1, $2, $3, 'user')")
        .bind(other_principal)
        .bind(format!("principal_{}", hex32()))
        .bind(fixture.org_id)
        .execute(&pool)
        .await
        .expect("seed second principal");

    let tag = format!("app_channel:{}", fixture.endpoint_id);
    // Same org, same app, same surface tags — only the owner differs.
    seed_session(&pool, &fixture, other_principal, &[&tag]).await;

    let found = db
        .find_app_session_by_tags_and_owner(
            fixture.org_id,
            fixture.app_id,
            PrincipalId::from(fixture.owner_principal_id),
            &[tag],
        )
        .await
        .expect("reuse lookup");

    assert!(
        found.is_none(),
        "a session owned by a different principal must not be adopted even when surface tags overlap"
    );
}

/// TM-A2A-007: reuse must not cross an org boundary, even with identical tags
/// and an identically-shaped app.
#[tokio::test]
async fn cross_org_reuse_fails() {
    let pool = pool().await;
    let db = Database::new(pool.clone());
    let mine = seed(&pool, "endpoint-org-a", "published").await;
    let theirs = seed(&pool, "endpoint-org-b", "published").await;

    let tag = "app_channel:shared-surface".to_string();
    seed_session(&pool, &theirs, theirs.owner_principal_id, &[&tag]).await;

    let found = db
        .find_app_session_by_tags_and_owner(
            mine.org_id,
            theirs.app_id,
            PrincipalId::from(theirs.owner_principal_id),
            &[tag],
        )
        .await
        .expect("reuse lookup");

    assert!(
        found.is_none(),
        "a session in another org must not be adopted (TM-A2A-007)"
    );
}

/// Reuse must not cross an app boundary within the same org and owner.
#[tokio::test]
async fn cross_app_reuse_fails() {
    let pool = pool().await;
    let db = Database::new(pool.clone());
    let first = seed(&pool, "endpoint-app-a", "published").await;

    // A second app in the same org, owned by the same principal.
    let second_app = Uuid::now_v7();
    let harness_id: Uuid = sqlx::query_scalar("SELECT harness_id FROM apps WHERE id = $1")
        .bind(first.app_id)
        .fetch_one(&pool)
        .await
        .expect("read harness");
    let agent_id: Uuid = sqlx::query_scalar("SELECT agent_id FROM apps WHERE id = $1")
        .bind(first.app_id)
        .fetch_one(&pool)
        .await
        .expect("read agent");
    sqlx::query(
        "INSERT INTO apps (id, org_id, public_id, name, harness_id, agent_id, status,
                           agent_version_policy, owner_principal_id, channel_type, channel_config)
         VALUES ($1, $2, $3, $4, $5, $6, 'published', 'default', $7, 'slack', '{}'::jsonb)",
    )
    .bind(second_app)
    .bind(first.org_id)
    .bind(format!("app_{}", hex32()))
    .bind(format!("app-{}", hex32()))
    .bind(harness_id)
    .bind(agent_id)
    .bind(first.owner_principal_id)
    .execute(&pool)
    .await
    .expect("seed second app");

    let tag = format!("app_channel:{}", first.endpoint_id);
    seed_session(&pool, &first, first.owner_principal_id, &[&tag]).await;

    let found = db
        .find_app_session_by_tags_and_owner(
            first.org_id,
            second_app,
            PrincipalId::from(first.owner_principal_id),
            &[tag],
        )
        .await
        .expect("reuse lookup");

    assert!(
        found.is_none(),
        "a session owned by a different app must not be adopted (TM-AUTHZ-009)"
    );
}

/// Containment, not equality: a candidate must carry *every* routing tag. A
/// session holding only a subset must not be adopted, which is what stops a
/// caller from being matched by seeding one broad tag.
#[tokio::test]
async fn reuse_requires_containment_of_every_routing_tag() {
    let pool = pool().await;
    let db = Database::new(pool.clone());
    let fixture = seed(&pool, "endpoint-containment", "published").await;

    let endpoint_tag = format!("app_channel:{}", fixture.endpoint_id);
    // Session carries only the first of the two tags the lookup requires.
    seed_session(
        &pool,
        &fixture,
        fixture.owner_principal_id,
        &[&endpoint_tag],
    )
    .await;

    let required = [endpoint_tag.clone(), "slack:thread:T999".to_string()];
    let found = db
        .find_app_session_by_tags_and_owner(
            fixture.org_id,
            fixture.app_id,
            PrincipalId::from(fixture.owner_principal_id),
            &required,
        )
        .await
        .expect("reuse lookup");

    assert!(
        found.is_none(),
        "reuse requires the candidate to contain every routing tag, not merely overlap"
    );

    // The superset case still matches, so containment did not become equality.
    let found = db
        .find_app_session_by_tags_and_owner(
            fixture.org_id,
            fixture.app_id,
            PrincipalId::from(fixture.owner_principal_id),
            &[endpoint_tag],
        )
        .await
        .expect("reuse lookup");
    assert!(
        found.is_some(),
        "a candidate carrying a superset of the routing tags must still be adopted"
    );
}

/// An endpoint is owned by an agent, so creating one against an agent-less App
/// must fail loudly rather than write a row with no owner.
#[tokio::test]
async fn endpoint_creation_requires_the_app_to_have_an_agent() {
    let pool = pool().await;
    let db = Database::new(pool.clone());
    let fixture = seed(&pool, "endpoint-no-agent", "draft").await;

    sqlx::query("UPDATE apps SET agent_id = NULL WHERE id = $1")
        .bind(fixture.app_id)
        .execute(&pool)
        .await
        .expect("clear agent");

    let result = db
        .create_app_channel(
            fixture.app_id,
            everruns_server::storage::CreateAppChannelRow {
                public_id: format!("appchan_{}", hex32()),
                channel_type: "slack".to_string(),
                channel_config: serde_json::json!({}),
                channel_config_encrypted: None,
                auth: None,
                auth_encrypted: None,
                durable_schedule_id: None,
                enabled: true,
            },
        )
        .await;

    assert!(
        result.is_err(),
        "creating an endpoint on an agent-less App must fail; an endpoint must be owned by an agent"
    );
}

/// The update path writes to `agent_endpoints` rather than the read-only view.
/// It must preserve endpoint identity and the authoritative endpoint status.
#[tokio::test]
async fn updating_an_endpoint_preserves_identity_and_keeps_status_honest() {
    let pool = pool().await;
    let db = Database::new(pool.clone());
    let fixture = seed(&pool, "endpoint-update", "published").await;

    let public_id: String =
        sqlx::query_scalar("SELECT public_id FROM agent_endpoints WHERE id = $1")
            .bind(fixture.endpoint_id)
            .fetch_one(&pool)
            .await
            .expect("read public_id");

    // Disabling must drive the derived status to 'disabled'.
    let updated = db
        .update_app_channel(
            fixture.endpoint_id,
            everruns_server::storage::UpdateAppChannel {
                channel_config: Some(serde_json::json!({"team_id": "T1"})),
                enabled: Some(false),
                ..Default::default()
            },
        )
        .await
        .expect("update endpoint")
        .expect("endpoint exists");

    assert_eq!(updated.id, fixture.endpoint_id, "row id must not change");
    assert_eq!(
        updated.public_id, public_id,
        "public_id is the ingress identity and must survive an update"
    );
    assert!(!updated.enabled);

    let status: String = sqlx::query_scalar("SELECT status FROM agent_endpoints WHERE id = $1")
        .bind(fixture.endpoint_id)
        .fetch_one(&pool)
        .await
        .expect("read status");
    assert_eq!(status, "disabled");

    // Re-enabling does not infer lifecycle from the frozen App row.
    db.update_app_channel(
        fixture.endpoint_id,
        everruns_server::storage::UpdateAppChannel {
            enabled: Some(true),
            ..Default::default()
        },
    )
    .await
    .expect("re-enable endpoint")
    .expect("endpoint exists");

    let status: String = sqlx::query_scalar("SELECT status FROM agent_endpoints WHERE id = $1")
        .bind(fixture.endpoint_id)
        .fetch_one(&pool)
        .await
        .expect("read status");
    assert_eq!(status, "disabled");

    db.update_app_channel(
        fixture.endpoint_id,
        everruns_server::storage::UpdateAppChannel {
            status: Some("live".to_string()),
            ..Default::default()
        },
    )
    .await
    .expect("set endpoint live")
    .expect("endpoint exists");

    let status: String = sqlx::query_scalar("SELECT status FROM agent_endpoints WHERE id = $1")
        .bind(fixture.endpoint_id)
        .fetch_one(&pool)
        .await
        .expect("read status");
    assert_eq!(status, "live");

    // The agent and owner the endpoint was created with are untouched by an update.
    let (agent_id, owner): (Uuid, Uuid) =
        sqlx::query_as("SELECT agent_id, owner_principal_id FROM agent_endpoints WHERE id = $1")
            .bind(fixture.endpoint_id)
            .fetch_one(&pool)
            .await
            .expect("read ownership");
    assert_eq!(owner, fixture.owner_principal_id);
    assert!(!agent_id.is_nil());
}

/// Deleting an endpoint removes it from the table and therefore from the view.
#[tokio::test]
async fn deleting_an_endpoint_removes_it_from_the_view() {
    let pool = pool().await;
    let db = Database::new(pool.clone());
    let fixture = seed(&pool, "endpoint-delete", "published").await;

    assert!(
        db.delete_app_channel(fixture.endpoint_id)
            .await
            .expect("delete endpoint")
    );

    let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM app_channels WHERE id = $1")
        .bind(fixture.endpoint_id)
        .fetch_one(&pool)
        .await
        .expect("count view rows");
    assert_eq!(remaining, 0);
}

#[tokio::test]
async fn endpoint_auth_migration_only_backfills_plaintext_transport_rows() {
    let pool = pool().await;
    let mut tx = pool.begin().await.expect("begin transaction");
    sqlx::raw_sql(
        "SET LOCAL search_path TO pg_temp;
         CREATE TABLE agent_endpoints (
             id UUID PRIMARY KEY,
             app_id UUID NOT NULL,
             public_id TEXT NOT NULL,
             channel_type TEXT NOT NULL,
             channel_config JSONB NOT NULL,
             channel_config_encrypted BYTEA,
             durable_schedule_id UUID,
             enabled BOOLEAN NOT NULL,
             created_at TIMESTAMPTZ NOT NULL,
             updated_at TIMESTAMPTZ NOT NULL
         );
         CREATE VIEW app_channels AS SELECT * FROM agent_endpoints;",
    )
    .execute(&mut *tx)
    .await
    .expect("create pre-migration schema");

    let plaintext_id = Uuid::now_v7();
    let encrypted_id = Uuid::now_v7();
    for (id, ciphertext) in [(plaintext_id, None), (encrypted_id, Some(vec![1_u8]))] {
        sqlx::query(
            "INSERT INTO agent_endpoints (
                 id, app_id, public_id, channel_type, channel_config,
                 channel_config_encrypted, enabled, created_at, updated_at
             ) VALUES ($1, $2, $3, 'ag_ui', $4, $5, true, NOW(), NOW())",
        )
        .bind(id)
        .bind(Uuid::now_v7())
        .bind(format!("appchan_{}", hex32()))
        .bind(serde_json::json!({
            "anonymous": false,
            "auth": {"mode": "google_oidc"}
        }))
        .bind(ciphertext)
        .execute(&mut *tx)
        .await
        .expect("seed pre-migration endpoint");
    }

    sqlx::raw_sql(include_str!("../migrations/139_agent_endpoint_auth.sql"))
        .execute(&mut *tx)
        .await
        .expect("apply endpoint auth migration");

    let (config, auth): (serde_json::Value, Option<serde_json::Value>) =
        sqlx::query_as("SELECT channel_config, auth FROM agent_endpoints WHERE id = $1")
            .bind(plaintext_id)
            .fetch_one(&mut *tx)
            .await
            .expect("read plaintext row");
    assert!(config.get("auth").is_none());
    assert_eq!(auth.unwrap()["mode"], "google_oidc");

    let (config, auth): (serde_json::Value, Option<serde_json::Value>) =
        sqlx::query_as("SELECT channel_config, auth FROM agent_endpoints WHERE id = $1")
            .bind(encrypted_id)
            .fetch_one(&mut *tx)
            .await
            .expect("read encrypted row");
    assert_eq!(config["auth"]["mode"], "google_oidc");
    assert!(auth.is_none());

    let _: (Option<serde_json::Value>, Option<Vec<u8>>) =
        sqlx::query_as("SELECT auth, auth_encrypted FROM app_channels WHERE id = $1")
            .bind(plaintext_id)
            .fetch_one(&mut *tx)
            .await
            .expect("compatibility view exposes auth columns");
}

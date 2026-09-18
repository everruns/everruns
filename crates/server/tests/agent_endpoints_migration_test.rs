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
    body::Body,
    http::{Method, Request, StatusCode},
};
use everruns_durable::InMemoryWorkflowEventStore;

use everruns_platform::ChannelType;
use everruns_provider::typed_id::PrincipalId;
use everruns_server::EventDelivery;
use everruns_server::api;
use everruns_server::storage::Database;
use everruns_server::storage::StorageBackend;
use everruns_worker::{RunnerBackend, create_runner_with_backend};
use serde_json::{Value, json};
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
    let pool = pool().await;
    let fixture = seed(&pool, "endpoint-no-app-reads", "published").await;
    seed_ingress_endpoint(
        &pool,
        &fixture,
        ChannelType::AgUi,
        json!({"anonymous": true, "token": "expected"}),
    )
    .await;
    seed_ingress_endpoint(
        &pool,
        &fixture,
        ChannelType::PublicChat,
        json!({"anonymous": true, "token": "expected"}),
    )
    .await;
    seed_ingress_endpoint(
        &pool,
        &fixture,
        ChannelType::Fcp,
        json!({"anonymous": false, "token": "expected"}),
    )
    .await;
    let webhook_id = seed_ingress_endpoint(
        &pool,
        &fixture,
        ChannelType::Webhook,
        json!({"token": "expected", "message": "{{body}}"}),
    )
    .await;
    let a2a_id = seed_ingress_endpoint(
        &pool,
        &fixture,
        ChannelType::A2a,
        json!({
            "api_key_hash": "never-matches",
            "api_key_prefix": "evra2a_test...",
            "message": "{{message}}"
        }),
    )
    .await;
    let api_id = seed_ingress_endpoint(
        &pool,
        &fixture,
        ChannelType::ApiEndpoint,
        json!({
            "api_key_hash": "never-matches",
            "api_key_prefix": "evr_app_test..."
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

    let role = format!("ingress_no_apps_{}", Uuid::new_v4().simple());
    let create_role = format!("CREATE ROLE {role} NOLOGIN");
    sqlx::query(sqlx::AssertSqlSafe(create_role.as_str()))
        .execute(&pool)
        .await
        .expect("create restricted ingress role");
    let grants = format!(
        "GRANT USAGE ON SCHEMA public TO {role};
         GRANT SELECT ON agents, agent_endpoints, agent_triggers TO {role};"
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

    assert_route_status(
        &router,
        Method::POST,
        &format!("/v1/apps/{app_id}/ag-ui"),
        &[("content-type", "application/json")],
        br#"{}"#,
        StatusCode::UNAUTHORIZED,
    )
    .await;
    assert_route_status(
        &router,
        Method::POST,
        &format!("/v1/apps/{app_id}/ag-ui/images"),
        &[("content-type", "multipart/form-data; boundary=empty")],
        b"--empty--\r\n",
        StatusCode::UNAUTHORIZED,
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
    assert_route_status(
        &router,
        Method::POST,
        &format!("/v1/apps/{app_id}/public-chat"),
        &[("content-type", "application/json")],
        br#"{}"#,
        StatusCode::UNAUTHORIZED,
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
    assert_route_status(
        &router,
        Method::POST,
        &format!("/v1/apps/{app_id}/fcp"),
        &[("content-type", "text/plain")],
        b"hello",
        StatusCode::UNAUTHORIZED,
    )
    .await;
    assert_route_status(
        &router,
        Method::GET,
        &format!("/v1/apps/{app_id}/slack/manifest"),
        &[],
        b"",
        StatusCode::OK,
    )
    .await;
    assert_route_status(
        &router,
        Method::POST,
        &format!("/v1/apps/{app_id}/slack/events"),
        &[("content-type", "application/json")],
        br#"{}"#,
        StatusCode::UNAUTHORIZED,
    )
    .await;
    assert_route_status(
        &router,
        Method::POST,
        &format!("/v1/apps/{app_id}/slack/interactivity"),
        &[("content-type", "application/x-www-form-urlencoded")],
        b"payload=%7B%7D",
        StatusCode::UNAUTHORIZED,
    )
    .await;
    assert_route_status(
        &router,
        Method::POST,
        &format!("/v1/apps/{app_id}/webhooks/{webhook_id}"),
        &[("content-type", "application/json")],
        br#"{}"#,
        StatusCode::UNAUTHORIZED,
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
    assert_route_status(
        &router,
        Method::POST,
        &format!("/v1/apps/{app_id}/a2a/{a2a_id}"),
        &[("content-type", "application/json")],
        br#"{"jsonrpc":"2.0","id":1,"method":"message/send","params":{}}"#,
        StatusCode::UNAUTHORIZED,
    )
    .await;
    for (method, suffix) in [
        (Method::POST, "sessions"),
        (Method::GET, "sessions/ses_missing"),
        (Method::POST, "sessions/ses_missing/messages"),
        (Method::POST, "sessions/ses_missing/cancel"),
    ] {
        assert_route_status(
            &router,
            method,
            &format!("/v1/apps/{app_id}/api/{api_id}/{suffix}"),
            &[("content-type", "application/json")],
            br#"{"message":"hello"}"#,
            StatusCode::UNAUTHORIZED,
        )
        .await;
    }

    restricted_pool.close().await;
    let drop_role = format!(
        "REVOKE ALL PRIVILEGES ON SCHEMA public FROM {role};
         REVOKE ALL PRIVILEGES ON agents, agent_endpoints, agent_triggers FROM {role};
         DROP ROLE {role};"
    );
    sqlx::raw_sql(sqlx::AssertSqlSafe(drop_role.as_str()))
        .execute(&pool)
        .await
        .expect("drop restricted ingress role");
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
    let mut request = Request::builder().method(method).uri(uri);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let response = router
        .clone()
        .oneshot(
            request
                .body(Body::from(body.to_vec()))
                .expect("build ingress request"),
        )
        .await
        .expect("run ingress request");
    assert_eq!(response.status(), expected, "{uri}");
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
/// It must preserve endpoint identity and keep the derived `status` column in
/// step with the `App.status × enabled` pair it is derived from, so the column
/// does not drift before the publish phase makes it authoritative.
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

    // Re-enabling under a published App must return it to 'live'.
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

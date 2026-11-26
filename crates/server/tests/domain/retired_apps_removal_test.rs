//! The contract migration removes the aggregate without deleting channels or history.
use axum::http::StatusCode;
use serde_json::json;

const VERSION: i64 = 20261125151326;
const MIGRATION: &str = include_str!("../../migrations/20261125151326_remove_retired_apps.sql");

#[tokio::test]
async fn upgrade_preserves_channels_bindings_history_and_unambiguous_attribution() {
    let (pool, _database) =
        everruns_server::storage::test_database::create_database_migrated_before(0)
            .await
            .unwrap();
    let mut migrator = everruns_server::setup::migrations::migrator();
    migrator.migrations = migrator
        .migrations
        .iter()
        .filter(|m| m.version < VERSION)
        .cloned()
        .collect::<Vec<_>>()
        .into();
    migrator.run(&pool).await.unwrap();
    sqlx::raw_sql(include_str!("fixtures/retired_apps_upgrade.sql"))
        .execute(&pool)
        .await
        .unwrap();
    sqlx::raw_sql(MIGRATION).execute(&pool).await.unwrap();
    let columns: i64 = sqlx::query_scalar("SELECT count(*) FROM information_schema.columns WHERE table_schema = 'public' AND table_name IN ('sessions','agent_channels','agent_triggers') AND column_name IN ('app_id','execution_app_id','legacy_alias_id','legacy_alias_name')").fetch_one(&pool).await.unwrap();
    assert_eq!(columns, 0);
    let absent: bool = sqlx::query_scalar(
        "SELECT to_regclass('apps') IS NULL AND to_regclass('app_channels') IS NULL",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(absent);
    let preserved: i64 = sqlx::query_scalar("SELECT count(*) FROM agent_channels WHERE channel_config_encrypted = '\\x010203' AND status = 'live'").fetch_one(&pool).await.unwrap();
    assert_eq!(preserved, 2);
    let attributed: bool = sqlx::query_scalar("SELECT channel_id = '00000000-0000-0000-0000-000000000005' AND tags @> ARRAY['channel:appchan_00000000000000000000000000000005','ingress:appchan_00000000000000000000000000000005'] FROM sessions WHERE id = '00000000-0000-0000-0000-000000000008'").fetch_one(&pool).await.unwrap();
    assert!(attributed);
    let ambiguous: bool = sqlx::query_scalar(
        "SELECT channel_id IS NULL FROM sessions WHERE id = '00000000-0000-0000-0000-000000000009'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(ambiguous, "never guess between sibling channels");
    let untrusted: bool = sqlx::query_scalar(
        "SELECT channel_id IS NULL FROM sessions WHERE id = '00000000-0000-0000-0000-000000000013'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        untrusted,
        "mutable transport hints must not grant channel authority"
    );
    let bindings: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM channel_bindings WHERE binding_key = 'thread:preserved'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(bindings, 1);
    let authority: bool = sqlx::query_scalar("SELECT execution_harness_id = '00000000-0000-0000-0000-000000000002' AND execution_owner_principal_id = '00000000-0000-0000-0000-000000000001' FROM agent_triggers WHERE org_id = 90").fetch_one(&pool).await.unwrap();
    assert!(
        authority,
        "trigger execution authority survives independently of the removed aggregate"
    );
    let trigger_tags: bool = sqlx::query_scalar("SELECT tags @> ARRAY['agent_trigger:trg_00000000000000000000000000000010','__internal:agent_trigger'] FROM sessions WHERE id = '00000000-0000-0000-0000-000000000011'").fetch_one(&pool).await.unwrap();
    assert!(trigger_tags);
    let scheduled: bool = sqlx::query_scalar("SELECT target_name = 'invoke_scheduled_channel' AND NOT (target_input ? 'app_id') AND target_input->>'channel_id' = 'appchan_00000000000000000000000000000005' FROM durable_schedules WHERE name = 'preserved-schedule'").fetch_one(&pool).await.unwrap();
    assert!(scheduled);
    let target: serde_json::Value =
        sqlx::query_scalar("SELECT target FROM evals WHERE org_id = 90")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(target["retired_target"]["type"], "app");
    let typed: everruns_server::domains::evals::record::EvalTarget =
        serde_json::from_value(target).unwrap();
    let public = serde_json::to_value(typed).unwrap();
    assert_eq!(
        public,
        json!({"type":"external", "provider":"everruns", "model":"retired-target"})
    );
    let history: serde_json::Value = sqlx::query_scalar(
        "SELECT metadata FROM events WHERE session_id = '00000000-0000-0000-0000-000000000008'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        history,
        json!({"initiator":{"type":"app","app_id":"historical"}})
    );
}

#[tokio::test]
async fn old_routes_are_absent_and_canonical_channel_still_serves() {
    let server = crate::test_harness::TestServer::in_memory().await;
    for path in [
        "/v1/platform/apps",
        "/v1/platform/apps/app_removed",
        "/v1/apps/app_removed/fcp",
        "/v1/apps/app_removed/ag-ui",
        "/v1/apps/app_removed/slack/events",
        "/v1/apps/app_removed/slack/interactivity",
        "/v1/apps/app_removed/public-chat/config",
        "/v1/apps/app_removed/a2a/appchan_removed",
        "/v1/apps/app_removed/webhooks/appchan_removed",
    ] {
        server.get(path).await.assert_status(StatusCode::NOT_FOUND);
        server
            .post(path, json!({}))
            .await
            .assert_status(StatusCode::NOT_FOUND);
    }
    let agent: serde_json::Value = server.post("/v1/platform/agents", json!({"name":"removal-channel", "system_prompt":"test", "harness_id":server.seed_generic_harness_id})).await.assert_status(StatusCode::CREATED).json();
    let fixture = server
        .seed_channel_bundle("channel", agent["id"].as_str().unwrap(), "fcp", json!({}))
        .await;
    let id = fixture["id"].as_str().unwrap();
    server.set_bundle_live(id, true).await;
    server
        .get(&format!("/v1/channels/{id}/fcp"))
        .await
        .assert_status(StatusCode::OK);
    let public_chat = server
        .seed_channel_bundle(
            "public chat",
            agent["id"].as_str().unwrap(),
            "public_chat",
            json!({"anonymous": true}),
        )
        .await;
    let public_id = public_chat["id"].as_str().unwrap();
    server.set_bundle_live(public_id, true).await;
    let bootstrap: serde_json::Value = server
        .get(&format!("/v1/channels/{public_id}/public-chat/config"))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(bootstrap["channel_id"], public_id);
    assert!(bootstrap.get("app_id").is_none());
}

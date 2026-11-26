use std::sync::Arc;

use everruns_core::{Caller, DEFAULT_ORG_ID};
use serde_json::{Value, json};

use crate::domains::common::{CommandError, Ctx, dispatch};
use crate::storage::StorageBackend;

async fn run(name: &str, input: Value, ctx: &Ctx) -> Result<Value, CommandError> {
    Ok(serde_json::from_str(&dispatch(name, input, ctx).await?).unwrap())
}

#[tokio::test]
async fn application_name_undo_preserves_current_grants_and_uses_current_version() {
    let ctx = Ctx::minimal_for_test(
        Caller::internal(DEFAULT_ORG_ID),
        Arc::new(StorageBackend::test_database()),
        None,
    );
    let slug = format!("history-{}", uuid::Uuid::now_v7().simple());
    let original = run(
        "create_application",
        json!({"name":"Original", "slug":slug}),
        &ctx,
    )
    .await
    .unwrap();
    let id = original["id"].as_str().unwrap();
    let memory_name = format!("handbook-{}", &uuid::Uuid::now_v7().simple().to_string()[..12]);
    let memory = run("create_memory", json!({"name": memory_name}), &ctx)
        .await
        .unwrap();
    let resources = json!([{
        "kind":"memory", "resource_id":memory["id"], "slug":"handbook",
        "name":"Handbook", "writable":true
    }]);
    let config =
        json!({"membership":"invites", "invites":["editor@example.com"], "show_in_sidebar":true});
    run(
        "update_application",
        json!({"application_id":id, "version":1, "slug":slug, "name":"Reviewed",
            "description":"Keep this description", "config":config, "resources":resources}),
        &ctx,
    )
    .await
    .unwrap();
    run(
        "restore_entity_revision",
        json!({"entity_ref":id, "revision":1, "expected_revision":2, "fields":["name"]}),
        &ctx,
    )
    .await
    .unwrap();
    let restored = run("get_application", json!({"application_id":id}), &ctx)
        .await
        .unwrap();
    assert_eq!(restored["name"], "Original");
    assert_eq!(restored["slug"], slug);
    assert_eq!(restored["description"], "Keep this description");
    assert_eq!(restored["resources"], resources);
    assert_eq!(restored["config"]["membership"], "invites");
    assert_eq!(restored["config"]["invites"], config["invites"]);
    assert_eq!(restored["config"]["show_in_sidebar"], true);
    assert_eq!(restored["version"], 3);
    assert_eq!(restored["status"], "draft");

    let stale = run(
        "restore_entity_revision",
        json!({"entity_ref":id, "revision":1, "expected_revision":2, "fields":["name"]}),
        &ctx,
    )
    .await
    .unwrap_err();
    assert_eq!(stale.status(), axum::http::StatusCode::CONFLICT);
    for field in ["slug", "version", "status"] {
        let refused = run(
            "restore_entity_revision",
            json!({"entity_ref":id, "revision":1, "expected_revision":3, "fields":[field]}),
            &ctx,
        )
        .await
        .unwrap_err();
        assert_eq!(refused.status(), axum::http::StatusCode::BAD_REQUEST);
    }
}

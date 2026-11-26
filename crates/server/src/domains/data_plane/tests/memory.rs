//! A v2 session's Memory (`memory.rs`), on a real database: the create
//! names the session's mounts, and a shard's `memory_file` call reads and
//! writes the shared Memory only inside them.

use super::*;
use crate::domains::data_plane::memory;
use crate::storage::CreateMemoryRow;

/// The org's internal caller, as the route builds it: no file service.
fn internal_ctx(cp: &ControlPlane, org_id: i64) -> crate::domains::common::Ctx {
    crate::domains::common::Ctx::new(
        everruns_core::Caller::internal(org_id),
        cp.db.clone(),
        Arc::new(crate::services::CapabilityService::new(cp.db.clone(), None)),
        cp.encryption.clone(),
        Arc::new(everruns_core::DefaultPermissionResolver),
    )
}

/// A session owned by a person with a Memory of their own.
async fn session_with_user_memory(cp: &ControlPlane) -> everruns_contracts::typed_id::SessionId {
    let person = cp.db.create_test_user(uuid::Uuid::new_v4()).await;
    cp.db
        .create_memory(
            ORG,
            CreateMemoryRow {
                public_id: everruns_contracts::typed_id::MemoryId::new().to_string(),
                name: format!("user-memory-{}", person.simple()),
                description: None,
                scope: "user".into(),
                owner_agent_id: None,
                owner_user_id: Some(person),
                source_type: "manual".into(),
                source_config: json!({}),
                is_readonly: false,
                sync_status: "idle".into(),
                owner_principal_id: None,
                resolved_owner_user_id: Some(person),
            },
        )
        .await
        .unwrap();
    let mut row = crate::storage::test_database::test_session_row(ORG);
    row.resolved_owner_user_id = Some(person);
    cp.db.create_session(row).await.unwrap().id
}

#[tokio::test]
async fn a_shard_reaches_only_its_sessions_own_memory() {
    let (cp, _, _) = control_plane().await;
    let session = session_with_user_memory(&cp).await;
    assert_eq!(
        memory::mounts(&cp, session).await.unwrap(),
        vec![
            "/memory/user".to_string(),
            "/application/memory".to_string()
        ]
    );
    let ctx = internal_ctx(&cp, ORG);
    let call = |op: &str, path: &str, extra: Value| {
        let mut params = json!({ "session_id": session.to_string(), "op": op, "path": path });
        params
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        params
    };

    let written = memory::answer(
        &ctx,
        &call(
            "write",
            "/memory/user/notes.md",
            json!({ "content": "likes tea" }),
        ),
    )
    .await;
    assert_eq!(
        written["ok"]["path"],
        json!("/memory/user/notes.md"),
        "{written}"
    );

    // Another session of the same person reads it: Memory is shared.
    let mut again = crate::storage::test_database::test_session_row(ORG);
    again.resolved_owner_user_id = cp
        .db
        .get_session(ORG, session)
        .await
        .unwrap()
        .unwrap()
        .resolved_owner_user_id;
    let other = cp.db.create_session(again).await.unwrap().id;
    let read = memory::answer(
        &ctx,
        &json!({ "session_id": other.to_string(), "op": "read", "path": "/memory/user/notes.md" }),
    )
    .await;
    assert_eq!(read["ok"]["content"], json!("likes tea"), "{read}");

    let listed = memory::answer(&ctx, &call("list", "/memory/user", json!({}))).await;
    assert_eq!(
        listed["ok"][0]["path"],
        json!("/memory/user/notes.md"),
        "{listed}"
    );
    let missing = memory::answer(&ctx, &call("read", "/memory/user/nothing.md", json!({}))).await;
    assert_eq!(missing, json!({ "ok": null }));

    // THREAT[TM-TENANT]: nothing outside the session's own mounts, whatever
    // the shard names: not its workspace, not a Memory it has none of.
    for path in [
        "/workspace/notes.md",
        "/memory/agent/x.md",
        "/memory/userx/y.md",
    ] {
        let refused = memory::answer(&ctx, &call("read", path, json!({}))).await;
        assert!(
            refused["error"]
                .as_str()
                .unwrap()
                .contains("not in this session's Memory"),
            "{path}: {refused}"
        );
    }
    // Another org's caller does not find the session.
    let stranger = memory::answer(
        &internal_ctx(&cp, everruns_core::DEFAULT_ORG_ID),
        &call("read", "/memory/user/notes.md", json!({})),
    )
    .await;
    assert_eq!(stranger["error"], json!("session not found"));
    let unknown = memory::answer(&ctx, &call("chmod", "/memory/user/notes.md", json!({}))).await;
    assert!(
        unknown["error"].as_str().unwrap().contains("unknown"),
        "{unknown}"
    );

    // The reserved routing mount alone confers no access.
    let plain = cp
        .db
        .create_session(crate::storage::test_database::test_session_row(ORG))
        .await
        .unwrap()
        .id;
    assert_eq!(
        memory::mounts(&cp, plain).await.unwrap(),
        vec!["/application/memory"]
    );
    let refused = memory::answer(
        &ctx,
        &json!({"session_id":plain,"op":"read","path":"/application/memory/handbook/notes.md"}),
    )
    .await;
    assert!(
        refused["error"]
            .as_str()
            .unwrap()
            .contains("not in this session's Memory")
    );
}

#[tokio::test]
async fn room_routes_exist_before_the_wake_binds_its_session() {
    let (cp, _, _) = control_plane().await;
    let mut row = crate::storage::test_database::test_session_row(ORG);
    row.source = "room".into();
    let session = cp.db.create_session(row).await.unwrap().id;
    assert_eq!(
        memory::mounts(&cp, session).await.unwrap(),
        vec!["/application/memory", "/room"]
    );
    let ctx = internal_ctx(&cp, ORG);
    let refused = memory::answer(
        &ctx,
        &json!({"session_id":session,"op":"read","path":"/room/notes.md"}),
    )
    .await;
    assert!(
        refused["error"]
            .as_str()
            .unwrap()
            .contains("not in this session's Memory")
    );
}

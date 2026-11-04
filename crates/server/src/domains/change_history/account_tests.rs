use crate::domains::common::*;
use crate::storage::{CreateUserRow, StorageBackend};
use everruns_core::{Caller, DEFAULT_ORG_ID, OrgRole};
use serde_json::{Value, json};
use std::sync::Arc;

async fn command(name: &str, params: Value, ctx: &Ctx) -> Result<Value, CommandError> {
    Ok(serde_json::from_str(&dispatch(name, params, ctx).await?).unwrap())
}

#[tokio::test]
async fn profile_history_is_private_global_to_its_owner_exported_and_erased_on_deletion() {
    let db = Arc::new(StorageBackend::test_database());
    let user = db
        .create_user(CreateUserRow {
            email: "profile-owner@example.test".into(),
            name: "Private original name".into(),
            avatar_url: None,
            roles: vec!["user".into()],
            password_hash: None,
            email_verified: true,
            auth_provider: None,
            auth_provider_id: None,
            external_id: None,
        })
        .await
        .unwrap();
    let ctx = Ctx::minimal_for_test(
        Caller {
            org_id: DEFAULT_ORG_ID,
            org_public_id: "org_00000000000000000000000000000001".into(),
            user_id: Some(user.id),
            role: OrgRole::Owner,
            is_platform_user: false,
            is_internal: false,
            read_only: false,
        },
        db.clone(),
        None,
    );
    let id = user.id.to_string();
    command(
        "update_account_profile",
        json!({"id":id,"name":"Reviewed name"}),
        &ctx,
    )
    .await
    .unwrap();
    let entries = command(
        "list_entity_history",
        json!({"entity_ref":id,"kind":"user_profile"}),
        &ctx,
    )
    .await
    .unwrap();
    assert_eq!(entries[0]["revision"], 2);
    let org_history = command("list_org_history", json!({}), &ctx).await.unwrap();
    assert!(
        !org_history
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["entity_kind"] == "user_profile")
    );
    assert_eq!(
        command("list_org_history", json!({"kind":"user_profile"}), &ctx)
            .await
            .unwrap(),
        json!([])
    );
    let mut other = ctx.clone();
    other.caller.user_id = Some(uuid::Uuid::from_u128(99));
    other.caller.role = OrgRole::Admin;
    for (name, params) in [
        (
            "list_entity_history",
            json!({"entity_ref":id,"kind":"user_profile"}),
        ),
        (
            "show_entity_revision",
            json!({"entity_ref":id,"kind":"user_profile","revision":1}),
        ),
        (
            "restore_entity_revision",
            json!({"entity_ref":id,"kind":"user_profile","revision":1,"expected_revision":2,"fields":["name"]}),
        ),
        (
            "update_account_profile",
            json!({"id":id,"name":"Forbidden"}),
        ),
    ] {
        assert!(
            command(name, params, &other).await.is_err(),
            "{name} must remain private"
        );
    }
    let mut another_org = ctx.clone();
    another_org.caller.org_id = DEFAULT_ORG_ID + 100;
    another_org.caller.org_public_id = "org_00000000000000000000000000000100".into();
    command(
        "update_account_profile",
        json!({"id":id,"name":"Name from another organization"}),
        &another_org,
    )
    .await
    .unwrap();
    let stale = command("restore_entity_revision", json!({"entity_ref":id,"kind":"user_profile","revision":1,"expected_revision":2,"fields":["name"]}), &ctx).await.unwrap_err();
    assert_eq!(stale.code.as_deref(), Some("entity_revision_changed"));
    let restored = command("restore_entity_revision", json!({"entity_ref":id,"kind":"user_profile","revision":1,"expected_revision":3,"fields":["name"]}), &ctx).await.unwrap();
    assert_eq!(restored["entity"]["name"], "Private original name");
    let export = db.export_user_data(user.id).await.unwrap().unwrap();
    assert!(export["profile_history"].as_array().unwrap().len() >= 4);
    let private_rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM account_entity_changes WHERE owner_user_id = $1")
            .bind(user.id)
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert!(private_rows >= 4);
    assert!(db.delete_user_account(user.id).await.unwrap());
    let remaining: i64 =
        sqlx::query_scalar("SELECT count(*) FROM account_entity_changes WHERE owner_user_id = $1")
            .bind(user.id)
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert_eq!(remaining, 0);
}

#[tokio::test]
async fn alternate_profile_uuid_spellings_share_one_revision_and_save_guard() {
    let db = Arc::new(StorageBackend::test_database());
    let user = db
        .create_user(CreateUserRow {
            email: "profile-alias@example.test".into(),
            name: "Original".into(),
            avatar_url: None,
            roles: vec!["user".into()],
            password_hash: None,
            email_verified: true,
            auth_provider: None,
            auth_provider_id: None,
            external_id: None,
        })
        .await
        .unwrap();
    let ctx = Ctx::minimal_for_test(
        Caller {
            org_id: DEFAULT_ORG_ID,
            org_public_id: "org_00000000000000000000000000000001".into(),
            user_id: Some(user.id),
            role: OrgRole::Owner,
            is_platform_user: false,
            is_internal: false,
            read_only: false,
        },
        db,
        None,
    );
    let canonical = user.id.to_string();
    let alternate = canonical.to_uppercase();
    command(
        "update_account_profile",
        json!({"id":alternate,"name":"First reviewed name"}),
        &ctx,
    )
    .await
    .unwrap();
    for id in [&canonical, &alternate] {
        let entries = command(
            "list_entity_history",
            json!({"entity_ref":id,"kind":"user_profile"}),
            &ctx,
        )
        .await
        .unwrap();
        assert_eq!(entries[0]["revision"], 2);
        assert_eq!(entries[0]["entity_ref"], canonical);
        assert_eq!(entries[0]["changed_fields"], json!(["name"]));
    }
    let guarded = ctx.clone().with_change_intent(super::intent::ChangeIntent {
        expected_change: Some(
            json!({"kind":"user_profile","id":canonical,"revision":2}).to_string(),
        ),
        ..Default::default()
    });
    command(
        "update_account_profile",
        json!({"id":alternate,"name":"Second reviewed name"}),
        &guarded,
    )
    .await
    .unwrap();
    let stale = ctx.clone().with_change_intent(super::intent::ChangeIntent {
        expected_change: Some(
            json!({"kind":"user_profile","id":canonical,"revision":2}).to_string(),
        ),
        ..Default::default()
    });
    let error = command(
        "update_account_profile",
        json!({"id":canonical,"name":"Must not overwrite"}),
        &stale,
    )
    .await
    .unwrap_err();
    assert_eq!(error.code.as_deref(), Some("entity_revision_changed"));
    assert_eq!(
        command("get_account_profile", json!({"id":canonical}), &ctx)
            .await
            .unwrap()["name"],
        "Second reviewed name"
    );
}

struct SelfServicePermissions;
impl everruns_core::PermissionResolver for SelfServicePermissions {
    fn has_permission(&self, _: &Caller, _: &everruns_core::Permission) -> bool {
        false
    }
    fn caller_permissions(&self, _: &Caller) -> Vec<everruns_core::Permission> {
        vec![]
    }
}

#[tokio::test]
async fn console_self_profile_shares_management_history_and_guards_without_management_permission() {
    let db = Arc::new(StorageBackend::test_database());
    let user = db
        .create_user(CreateUserRow {
            email: "self-profile@example.test".into(),
            name: "Original profile".into(),
            avatar_url: None,
            roles: vec!["user".into()],
            password_hash: None,
            email_verified: true,
            auth_provider: None,
            auth_provider_id: None,
            external_id: None,
        })
        .await
        .unwrap();
    db.add_organization_member(DEFAULT_ORG_ID, user.id, "member")
        .await
        .unwrap();
    let profile = db
        .default_virtual_user(DEFAULT_ORG_ID, user.id)
        .await
        .unwrap();
    // A self read inside Save/Undo must reuse the one available connection.
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(std::time::Duration::from_secs(2))
        .connect_with(db.pool().connect_options().as_ref().clone())
        .await
        .unwrap();
    let _fixture_db = db.clone();
    let db = Arc::new(StorageBackend::from_database(
        crate::storage::Database::new(pool),
    ));
    let id = profile.id.to_string();
    let ctx = Ctx::minimal_for_test(
        Caller {
            org_id: DEFAULT_ORG_ID,
            org_public_id: "org_00000000000000000000000000000001".into(),
            user_id: Some(user.id),
            role: OrgRole::Member,
            is_platform_user: false,
            is_internal: false,
            read_only: false,
        },
        db,
        None,
    )
    .with_permission_resolver(Arc::new(SelfServicePermissions));
    assert!(
        command("get_virtual_user", json!({"id":id}), &ctx)
            .await
            .is_err()
    );
    assert!(
        command(
            "update_virtual_user",
            json!({"id":id,"name":"Forbidden management write"}),
            &ctx
        )
        .await
        .is_err()
    );
    let original = command("get_own_virtual_user_profile", json!({"id":id}), &ctx)
        .await
        .unwrap();
    command(
        "update_own_virtual_user_profile",
        json!({"id":id,"name":"Reviewed profile","locale":"fr-FR"}),
        &ctx,
    )
    .await
    .unwrap();
    let history = command("list_entity_history", json!({"entity_ref":id}), &ctx)
        .await
        .unwrap();
    assert_eq!(history[0]["revision"], 2);
    command(
        "restore_entity_revision",
        json!({"entity_ref":id,"revision":1,"expected_revision":2,"fields":["name","locale"]}),
        &ctx,
    )
    .await
    .unwrap();
    let restored = command("get_own_virtual_user_profile", json!({"id":id}), &ctx)
        .await
        .unwrap();
    assert_eq!(restored["name"], original["name"]);
    assert_eq!(restored["locale"], original["locale"]);
    assert!(
        command(
            "restore_entity_revision",
            json!({"entity_ref":id,"revision":1,"expected_revision":2,"fields":["name"]}),
            &ctx
        )
        .await
        .is_err()
    );
    assert!(
        command(
            "update_own_virtual_user_profile",
            json!({"id":id,"status":"archived"}),
            &ctx
        )
        .await
        .is_err()
    );
    let mut stranger = ctx.clone();
    stranger.caller.user_id = Some(uuid::Uuid::from_u128(43));
    for (name, params) in [
        ("get_own_virtual_user_profile", json!({"id":id})),
        (
            "update_own_virtual_user_profile",
            json!({"id":id,"name":"Forbidden"}),
        ),
        ("list_entity_history", json!({"entity_ref":id})),
    ] {
        assert!(command(name, params, &stranger).await.is_err(), "{name}");
    }
    let mut read_only = ctx.clone();
    read_only.caller.read_only = true;
    assert!(
        command(
            "update_own_virtual_user_profile",
            json!({"id":id,"name":"Forbidden"}),
            &read_only
        )
        .await
        .is_err()
    );
}

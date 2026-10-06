// Revisions through `Command::run`: snapshots, no-op updates, show, diff,
// restore, secrets that never enter history, and snapshot retention.

use std::sync::Arc;

use everruns_core::{Caller, DEFAULT_ORG_ID, organization::OrgRole};
use serde_json::{Value, json};
use uuid::Uuid;

use super::intent::{ChangeIntent, ChangeSurface};
use super::registry::EntityKind;
use super::snapshot;
use crate::domains::common::*;
use crate::storage::StorageBackend;
use crate::storage::encryption::EncryptionService;
use crate::storage::entity_changes::{
    EntityRevisionKey, MAX_SNAPSHOTS_PER_ENTITY, NewEntityChange,
};

fn owner() -> Ctx {
    let caller = Caller {
        org_id: DEFAULT_ORG_ID,
        org_public_id: "org_00000000000000000000000000000001".to_string(),
        user_id: Some(Uuid::from_u128(7)),
        role: OrgRole::Owner,
        is_platform_user: false,
        is_internal: false,
    };
    let encryption =
        EncryptionService::new("kek-v1:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=", &[]).unwrap();
    Ctx::minimal_for_test(
        caller,
        Arc::new(StorageBackend::in_memory()),
        Some(Arc::new(encryption)),
    )
}

async fn run(name: &str, params: Value, ctx: &Ctx) -> Result<Value, CommandError> {
    let output = dispatch(name, params, ctx).await?;
    Ok(serde_json::from_str(&output).unwrap_or(Value::Null))
}

fn with_reason(ctx: &Ctx, reason: &str) -> Ctx {
    ctx.clone().with_change_intent(
        ChangeIntent::on(ChangeSurface::Commands).with_reason(Some(reason.into())),
    )
}

async fn history(ctx: &Ctx, id: &str) -> Vec<Value> {
    run("list_entity_history", json!({ "entity_ref": id }), ctx)
        .await
        .unwrap()
        .as_array()
        .unwrap()
        .clone()
}

#[tokio::test]
async fn changes_make_revisions_and_a_restore_brings_one_back_as_a_new_change() {
    let ctx = owner();
    let created = run("create_workspace", json!({ "name": "support" }), &ctx)
        .await
        .unwrap();
    let id = created["id"].as_str().unwrap().to_string();
    let rename = json!({ "workspace_id": id, "name": "support-eu" });
    run("update_workspace", rename.clone(), &ctx).await.unwrap();
    // The same values again: an entry, but no new revision.
    run("update_workspace", rename, &ctx).await.unwrap();

    let entries = history(&ctx, &id).await;
    let revisions: Vec<Value> = entries.iter().map(|e| e["revision"].clone()).collect();
    assert_eq!(revisions, [Value::Null, json!(2), json!(1)]);

    let first = run(
        "show_entity_revision",
        json!({ "entity_ref": id, "revision": 1 }),
        &ctx,
    )
    .await
    .unwrap();
    assert_eq!(first["snapshot"]["name"], "support");
    assert!(
        first["snapshot"].get("updated_at").is_none(),
        "volatile fields are dropped"
    );
    let latest = run("show_entity_revision", json!({ "entity_ref": id }), &ctx)
        .await
        .unwrap();
    assert_eq!(latest["revision"], 2);

    let diff = run(
        "diff_entity_revisions",
        json!({ "entity_ref": id, "from": 1 }),
        &ctx,
    )
    .await
    .unwrap();
    assert_eq!(
        diff,
        json!([{ "field": "name", "from": "support", "to": "support-eu" }])
    );

    let restored = run(
        "restore_entity_revision",
        json!({ "entity_ref": id, "revision": 1 }),
        &with_reason(&ctx, "the rename broke a dashboard"),
    )
    .await
    .unwrap();
    assert_eq!(restored["entity"]["name"], "support");
    let newest = &history(&ctx, &id).await[0];
    assert_eq!(newest["action"], "restored");
    assert_eq!(newest["restored_from_revision"], 1);
    assert_eq!(newest["revision"], 3);
    assert_eq!(newest["reason"], "the rename broke a dashboard");

    let missing = run(
        "restore_entity_revision",
        json!({ "entity_ref": id, "revision": 9 }),
        &ctx,
    )
    .await
    .unwrap_err();
    assert!(
        matches!(missing.kind, CommandErrorKind::NotFound(_)),
        "{missing:?}"
    );
}

#[tokio::test]
async fn restore_needs_the_kinds_manage_policy() {
    let ctx = owner();
    let id = run("create_workspace", json!({ "name": "ops" }), &ctx)
        .await
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let mut member = ctx.clone();
    member.caller.role = OrgRole::Member;
    let refused = run(
        "restore_entity_revision",
        json!({ "entity_ref": id, "revision": 1 }),
        &member,
    )
    .await
    .unwrap_err();
    assert!(
        matches!(refused.kind, CommandErrorKind::Forbidden(_)),
        "{refused:?}"
    );
}

/// THREAT[TM-API-030]: a secret set on an entity never reaches its history:
/// not the snapshot, the diff, nor the list. Restore keeps the current value
/// and says when it differs from the revision's.
#[tokio::test]
async fn secrets_enter_history_only_as_markers_and_are_never_restored() {
    let ctx = owner();
    let first_key = "sk-seeded-first-7f3c9a";
    let second_key = "sk-seeded-second-41d2e8";
    let created = run(
        "create_provider",
        json!({ "name": "openai-main", "provider_type": "openai", "api_key": first_key }),
        &ctx,
    )
    .await
    .unwrap();
    let id = created["id"].as_str().unwrap().to_string();
    run(
        "update_provider",
        json!({ "id": id, "api_key": second_key }),
        &ctx,
    )
    .await
    .unwrap();

    let first = run(
        "show_entity_revision",
        json!({ "entity_ref": id, "revision": 1 }),
        &ctx,
    )
    .await
    .unwrap();
    let marker = &first["snapshot"]["$secrets"]["api_key"];
    assert_eq!(marker["set"], true);
    assert!(
        marker["fingerprint"]
            .as_str()
            .is_some_and(|f| !f.is_empty())
    );
    let diff = run(
        "diff_entity_revisions",
        json!({ "entity_ref": id, "from": 1 }),
        &ctx,
    )
    .await
    .unwrap();
    assert!(
        diff.as_array()
            .unwrap()
            .iter()
            .any(|change| change["field"] == "$secrets.api_key"),
        "a rotated key shows as changed: {diff}"
    );

    let restored = run(
        "restore_entity_revision",
        json!({ "entity_ref": id, "revision": 1 }),
        &ctx,
    )
    .await
    .unwrap();
    let warnings = restored["warnings"].as_array().unwrap();
    assert!(warnings[0].as_str().unwrap().starts_with("api_key differs"));
    let row = ctx
        .db
        .get_provider(ctx.org_id(), snapshot::ref_uuid(&id).unwrap())
        .await
        .unwrap()
        .unwrap();
    let kept = ctx
        .encryption
        .as_ref()
        .unwrap()
        .decrypt_to_string(row.api_key_encrypted.as_deref().unwrap())
        .unwrap();
    assert_eq!(kept, second_key, "restore keeps the current secret");

    let mut everything = history(&ctx, &id)
        .await
        .into_iter()
        .map(|e| e.to_string())
        .collect::<Vec<_>>();
    everything.push(diff.to_string());
    for revision in 1..=3 {
        let shown = run(
            "show_entity_revision",
            json!({ "entity_ref": id, "revision": revision }),
            &ctx,
        )
        .await;
        everything.extend(shown.ok().map(|shown| shown.to_string()));
    }
    for text in everything {
        assert!(
            !text.contains(first_key) && !text.contains(second_key),
            "{text}"
        );
    }
}

#[test]
fn every_restorable_kind_names_an_update_command_that_takes_its_id() {
    let mut wrong = Vec::new();
    for kind in EntityKind::ALL {
        let Some((command, id_param)) = snapshot::restore_command(*kind) else {
            assert_eq!(*kind, EntityKind::Session);
            continue;
        };
        match snapshot::update_params(command) {
            None => wrong.push(format!("{}: no command `{command}`", kind.as_str())),
            Some(params) if !params.contains_key(id_param) => wrong.push(format!(
                "{}: `{command}` takes no `{id_param}`; it takes {:?}",
                kind.as_str(),
                params.keys().collect::<Vec<_>>()
            )),
            Some(_) => {}
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

#[tokio::test]
async fn only_the_newest_snapshots_are_kept() {
    let db = StorageBackend::in_memory();
    let change = |n: i64| NewEntityChange {
        org_id: DEFAULT_ORG_ID,
        entity_kind: "workspace".into(),
        entity_ref: "workspace_x".into(),
        command: "update_workspace".into(),
        action: "updated".into(),
        reason: None,
        changed_fields: vec!["name".into()],
        actor_kind: "user".into(),
        actor_user_id: None,
        via_session_id: None,
        via_agent_id: None,
        surface: "api".into(),
        request_id: None,
        idempotency_key: None,
        snapshot: Some(json!({ "name": n })),
        snapshot_hash: Some(n.to_string()),
        restored_from_revision: None,
    };
    for n in 1..=MAX_SNAPSHOTS_PER_ENTITY + 2 {
        db.record_entity_change(change(n)).await.unwrap();
    }
    let at = |revision| EntityRevisionKey {
        org_id: DEFAULT_ORG_ID,
        entity_kind: "workspace".into(),
        entity_ref: "workspace_x".into(),
        revision: Some(revision),
    };
    let pruned = db.get_entity_revision(&at(2)).await.unwrap().unwrap();
    assert_eq!(pruned.snapshot, None, "the entry stays, its snapshot goes");
    let kept = db.get_entity_revision(&at(3)).await.unwrap().unwrap();
    assert_eq!(kept.snapshot, Some(json!({ "name": 3 })));
}

#[test]
fn fingerprints_are_keyed_and_stable() {
    let a = EncryptionService::new("k1:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=", &[]).unwrap();
    let b = EncryptionService::new("k2:AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=", &[]).unwrap();
    assert_eq!(a.fingerprint(b"sk-1"), a.fingerprint(b"sk-1"));
    assert_ne!(a.fingerprint(b"sk-1"), a.fingerprint(b"sk-2"));
    assert_ne!(
        a.fingerprint(b"sk-1"),
        b.fingerprint(b"sk-1"),
        "keyed, not a plain hash"
    );
}

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
        read_only: false,
    };
    let encryption =
        EncryptionService::new("kek-v1:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=", &[]).unwrap();
    Ctx::minimal_for_test(
        caller,
        Arc::new(StorageBackend::test_database()),
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

#[tokio::test]
async fn guarded_harness_restore_preserves_other_fields_and_refuses_stale_undo() {
    let ctx = owner();
    crate::setup::org_init::initialize_org_harnesses(&ctx.db, DEFAULT_ORG_ID)
        .await
        .unwrap();
    let harness = run("create_harness", json!({"name":"parity-harness","system_prompt":"Original","description":"Original description"}), &ctx).await.unwrap();
    let id = harness["id"].as_str().unwrap();
    run(
        "update_harness",
        json!({"id":id,"system_prompt":"Chat prompt","description":"Keep this description"}),
        &ctx,
    )
    .await
    .unwrap();
    let restored = run(
        "restore_entity_revision",
        json!({"entity_ref":id,"revision":1,"expected_revision":2,"fields":["system_prompt"]}),
        &with_reason(&ctx, "Undo reviewed instructions"),
    )
    .await
    .unwrap();
    assert_eq!(restored["entity"]["system_prompt"], "Original");
    assert_eq!(restored["entity"]["description"], "Keep this description");
    assert_eq!(restored["revision"], 3);
    let error = run(
        "restore_entity_revision",
        json!({"entity_ref":id,"revision":1,"expected_revision":2,"fields":["description"]}),
        &ctx,
    )
    .await
    .unwrap_err();
    assert_eq!(error.code.as_deref(), Some("entity_revision_changed"));
    assert_eq!(history(&ctx, id).await[0]["revision"], 3);
}

#[tokio::test]
async fn reviewed_save_returns_its_exact_revision_and_rejects_stale_or_mismatched_guards() {
    let ctx = owner();
    crate::setup::org_init::initialize_org_harnesses(&ctx.db, DEFAULT_ORG_ID)
        .await
        .unwrap();
    let agent = run(
        "create_agent",
        json!({"name":"guarded-save","system_prompt":"Original"}),
        &ctx,
    )
    .await
    .unwrap();
    let id = agent["id"].as_str().unwrap();
    let intent = ChangeIntent {
        expected_change: Some(json!({"kind":"agent","id":id,"revision":1}).to_string()),
        ..ChangeIntent::default()
    };
    let receipt = intent.committed_change.clone();
    run(
        "update_agent",
        json!({"id":id,"system_prompt":"Reviewed"}),
        &ctx.clone().with_change_intent(intent),
    )
    .await
    .unwrap();
    assert_eq!(receipt.lock().as_ref().unwrap().revision, 2);
    for (kind, revision) in [("agent", 1), ("harness", 2)] {
        let intent = ChangeIntent {
            expected_change: Some(json!({"kind":kind,"id":id,"revision":revision}).to_string()),
            ..ChangeIntent::default()
        };
        assert!(
            run(
                "update_agent",
                json!({"id":id,"system_prompt":"Must not save"}),
                &ctx.clone().with_change_intent(intent)
            )
            .await
            .is_err()
        );
    }
    assert_eq!(
        run("get_agent", json!({"id":id}), &ctx).await.unwrap()["system_prompt"],
        "Reviewed"
    );
    assert_eq!(history(&ctx, id).await[0]["revision"], 2);
}

#[tokio::test]
async fn first_update_of_a_preexisting_entity_keeps_a_restorable_baseline() {
    let ctx = owner();
    crate::setup::org_init::initialize_org_harnesses(&ctx.db, DEFAULT_ORG_ID)
        .await
        .unwrap();
    let agent = run(
        "create_agent",
        json!({"name":"preexisting","system_prompt":"Original"}),
        &ctx,
    )
    .await
    .unwrap();
    let id = agent["id"].as_str().unwrap();
    sqlx::query("DELETE FROM entity_changes WHERE org_id = $1 AND entity_ref = $2")
        .bind(DEFAULT_ORG_ID)
        .bind(id)
        .execute(ctx.db.pool())
        .await
        .unwrap();
    let intent = ChangeIntent {
        expected_change: Some(json!({"kind":"agent","id":id,"revision":0}).to_string()),
        ..ChangeIntent::default()
    };
    run(
        "update_agent",
        json!({"id":id,"system_prompt":"First saved edit"}),
        &ctx.clone().with_change_intent(intent),
    )
    .await
    .unwrap();
    let entries = history(&ctx, id).await;
    assert_eq!(entries[0]["revision"], 2);
    assert_eq!(entries[1]["action"], "baseline");
    let restored = run(
        "restore_entity_revision",
        json!({"entity_ref":id,"revision":1,"expected_revision":2,"fields":["system_prompt"]}),
        &with_reason(&ctx, "Undo first reviewed edit"),
    )
    .await
    .unwrap();
    assert_eq!(restored["entity"]["system_prompt"], "Original");
}

#[tokio::test]
async fn guarded_agent_restore_undoes_one_field_and_refuses_a_stale_revision() {
    let ctx = owner();
    crate::setup::org_init::initialize_org_harnesses(&ctx.db, DEFAULT_ORG_ID)
        .await
        .unwrap();
    let agent = run(
        "create_agent",
        json!({"name":"chat-mode-test", "system_prompt":"Original", "tags":["original"]}),
        &ctx,
    )
    .await
    .unwrap();
    let id = agent["id"].as_str().unwrap();
    run(
        "update_agent",
        json!({"id":id, "system_prompt":"Changed", "tags":["new"]}),
        &ctx,
    )
    .await
    .unwrap();
    let ctx = with_reason(&ctx, "Undo only the Chat instructions change");
    let restored = run(
        "restore_entity_revision",
        json!({"entity_ref":id, "revision":1, "expected_revision":2, "fields":["system_prompt"]}),
        &ctx,
    )
    .await
    .unwrap();
    assert_eq!(restored["entity"]["system_prompt"], "Original");
    assert_eq!(restored["entity"]["tags"], json!(["new"]));
    let stale = run(
        "restore_entity_revision",
        json!({"entity_ref":id, "revision":1, "expected_revision":2, "fields":["tags"]}),
        &ctx,
    )
    .await
    .unwrap_err();
    assert_eq!(stale.code.as_deref(), Some("entity_revision_changed"));
    let entries = history(&ctx, id).await;
    assert_eq!(entries[0]["revision"], 3);
    assert_eq!(entries[0]["action"], "restored");
    assert_eq!(entries[0]["changed_fields"], json!(["system_prompt"]));
    for fields in [
        json!([]),
        json!(["$secrets"]),
        json!(["id"]),
        json!(["does_not_exist"]),
    ] {
        assert!(
            run(
                "restore_entity_revision",
                json!({"entity_ref":id, "revision":1, "expected_revision":3, "fields":fields}),
                &ctx
            )
            .await
            .is_err()
        );
    }
    assert!(
        run(
            "restore_entity_revision",
            json!({"entity_ref":id, "revision":1, "fields":["tags"]}),
            &ctx
        )
        .await
        .is_err()
    );
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
    let id = crate::domains::workspaces::seed_workspace(&ctx, "seeded").await;
    // Existing state is revision 1; the first edit is revision 2.
    run(
        "update_workspace",
        json!({ "workspace_id": id, "name": "support" }),
        &ctx,
    )
    .await
    .unwrap();
    let rename = json!({ "workspace_id": id, "name": "support-eu" });
    run("update_workspace", rename.clone(), &ctx).await.unwrap();
    // The same values again: an entry, but no new revision.
    run("update_workspace", rename, &ctx).await.unwrap();

    let entries = history(&ctx, &id).await;
    let revisions: Vec<Value> = entries.iter().map(|e| e["revision"].clone()).collect();
    assert_eq!(revisions, [Value::Null, json!(3), json!(2), json!(1)]);

    let first = run(
        "show_entity_revision",
        json!({ "entity_ref": id, "revision": 2 }),
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
    assert_eq!(latest["revision"], 3);

    let diff = run(
        "diff_entity_revisions",
        json!({ "entity_ref": id, "from": 2 }),
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
        json!({ "entity_ref": id, "revision": 2 }),
        &with_reason(&ctx, "the rename broke a dashboard"),
    )
    .await
    .unwrap();
    assert_eq!(restored["entity"]["name"], "support");
    let newest = &history(&ctx, &id).await[0];
    assert_eq!(newest["action"], "restored");
    assert_eq!(newest["restored_from_revision"], 2);
    assert_eq!(newest["revision"], 4);
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
    let id = crate::domains::workspaces::seed_workspace(&ctx, "seeded").await;
    run(
        "update_workspace",
        json!({ "workspace_id": id, "name": "ops" }),
        &ctx,
    )
    .await
    .unwrap();
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
            assert!(
                matches!(kind, EntityKind::Session | EntityKind::KnowledgeIndexer),
                "only conversation state and immutable search-account definitions have no update command"
            );
            continue;
        };
        match snapshot::update_params(command) {
            None => wrong.push(format!("{}: no command `{command}`", kind.as_str())),
            Some(params) if !id_param.is_empty() && !params.contains_key(id_param) => {
                wrong.push(format!(
                    "{}: `{command}` takes no `{id_param}`; it takes {:?}",
                    kind.as_str(),
                    params.keys().collect::<Vec<_>>()
                ))
            }
            Some(_) => {}
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

#[tokio::test]
async fn only_the_newest_snapshots_are_kept() {
    let db = StorageBackend::test_database();
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

#[tokio::test]
async fn model_selection_apply_and_undo_are_atomic_and_preserve_unrelated_fields() {
    let ctx = owner();
    let provider = run(
        "create_provider",
        json!({"name":"selection-fixture","provider_type":"openai"}),
        &ctx,
    )
    .await
    .unwrap();
    let provider = provider["id"].as_str().unwrap();
    let mut ids = Vec::new();
    for name in ["parity-a", "parity-b"] {
        let model = run(
            "create_model",
            json!({"provider_id":provider,"model_id":name,"display_name":name,"enabled":false}),
            &ctx,
        )
        .await
        .unwrap();
        ids.push(model["id"].as_str().unwrap().to_string());
    }
    let changes: Vec<_> = ids
        .iter()
        .map(|id| json!({"id":id,"provider_id":provider,"expected_enabled":false,"enabled":true}))
        .collect();
    let mut conflict = changes.clone();
    conflict[1]["expected_enabled"] = json!(true);
    assert!(
        run("apply_model_selection", json!({"changes":conflict}), &ctx)
            .await
            .is_err()
    );
    for id in &ids {
        assert_eq!(
            run("get_model", json!({"id":id}), &ctx).await.unwrap()["enabled"],
            false
        );
        assert_eq!(
            history(&ctx, id).await[0]["revision"],
            1,
            "failed batch must roll back child history"
        );
    }
    // A concurrent rename does not conflict with the reviewed boolean value.
    run(
        "update_model",
        json!({"id":ids[0],"display_name":"Keep my name"}),
        &ctx,
    )
    .await
    .unwrap();
    let receipts = run("apply_model_selection", json!({"changes":changes}), &ctx)
        .await
        .unwrap();
    assert_eq!(receipts.as_array().unwrap().len(), 2);
    let undo: Vec<_> = receipts.as_array().unwrap().iter().map(|row| json!({"id":row["id"],"from_revision":row["from_revision"],"expected_revision":row["revision"]})).collect();
    let mut stale = undo.clone();
    stale[1]["expected_revision"] = json!(0);
    assert!(
        run("undo_model_selection", json!({"changes":stale}), &ctx)
            .await
            .is_err()
    );
    for id in &ids {
        assert_eq!(
            run("get_model", json!({"id":id}), &ctx).await.unwrap()["enabled"],
            true
        );
    }
    run("undo_model_selection", json!({"changes":undo}), &ctx)
        .await
        .unwrap();
    for id in &ids {
        assert_eq!(
            run("get_model", json!({"id":id}), &ctx).await.unwrap()["enabled"],
            false
        );
    }
    assert_eq!(
        run("get_model", json!({"id":ids[0]}), &ctx).await.unwrap()["display_name"],
        "Keep my name"
    );
}

#[tokio::test]
async fn account_name_undo_preserves_current_credentials_and_is_tenant_scoped() {
    let ctx = owner();
    let account = ctx
        .db
        .create_organization_connection(crate::storage::CreateOrganizationConnectionRow {
            org_id: ctx.org_id(),
            name: "Original account".into(),
            provider: "daytona".into(),
            access_token_encrypted: Some(b"first-credential".to_vec()),
            ..Default::default()
        })
        .await
        .unwrap();
    let id = account.id.to_string();
    run(
        "update_organization_account_name",
        json!({"id":id,"name":"Reviewed account"}),
        &ctx,
    )
    .await
    .unwrap();
    // A credential rotation changes no historical name and must never be undone.
    ctx.db
        .update_organization_connection(
            ctx.org_id(),
            account.id,
            "Reviewed account",
            b"current-credential",
            Some("current-user"),
            None,
        )
        .await
        .unwrap();
    let restored = run("restore_entity_revision",json!({"entity_ref":id,"kind":"organization_account","revision":1,"expected_revision":2,"fields":["name"]}),&ctx).await.unwrap();
    assert_eq!(restored["entity"]["name"], "Original account");
    let current = ctx
        .db
        .get_organization_connection(ctx.org_id(), account.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        current.access_token_encrypted.as_deref(),
        Some(b"current-credential".as_slice())
    );
    assert_eq!(current.provider_username.as_deref(), Some("current-user"));
    let shown = run(
        "show_entity_revision",
        json!({"entity_ref":id,"kind":"organization_account","revision":1}),
        &ctx,
    )
    .await
    .unwrap()
    .to_string();
    for protected in ["first-credential", "current-credential", "current-user"] {
        assert!(!shown.contains(protected));
    }
    let mut other = ctx.clone();
    other.caller.org_id += 100;
    assert!(
        run(
            "update_organization_account_name",
            json!({"id":id,"name":"Forbidden"}),
            &other
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn sandbox_metadata_undo_keeps_current_execution_spec_and_clears_description() {
    let ctx = owner();
    let spec: crate::domains::sandbox_templates::record::SandboxTemplateSpec =
        serde_json::from_value(json!({"target":{"kind":"vfs","provider":"bashkit"}})).unwrap();
    let template = ctx
        .db
        .create_sandbox_template(
            ctx.org_id(),
            "parity-template",
            "Original template",
            None,
            &spec,
            false,
        )
        .await
        .unwrap();
    let id = template.public_id.to_string();
    run(
        "update_sandbox_template_metadata",
        json!({"id":id,"display_name":"Reviewed template","description":"Temporary description"}),
        &ctx,
    )
    .await
    .unwrap();
    let restored=run("restore_entity_revision",json!({"entity_ref":id,"revision":1,"expected_revision":2,"fields":["display_name","description"]}),&ctx).await.unwrap();
    assert_eq!(restored["entity"]["display_name"], "Original template");
    assert!(restored["entity"]["description"].is_null());
    assert!(restored["entity"].get("spec").is_none());
    assert_eq!(
        ctx.db
            .get_sandbox_template(ctx.org_id(), template.public_id)
            .await
            .unwrap()
            .unwrap()
            .current_revision
            .spec,
        spec
    );
}

#[tokio::test]
async fn channel_config_undo_preserves_current_token_and_auth_and_rejects_stale_changes() {
    let ctx = owner();
    crate::setup::org_init::initialize_org_harnesses(&ctx.db, DEFAULT_ORG_ID)
        .await
        .unwrap();
    let agent = run(
        "create_agent",
        json!({"name":"channel-parity","system_prompt":"Fixture"}),
        &ctx,
    )
    .await
    .unwrap();
    let agent_id = agent["id"].as_str().unwrap();
    let channel=run("create_agent_channel",json!({"agent_id":agent_id,"channel_type":"ag_ui","channel_config":{"generic_tool_text":"Original activity","token":"original-private-token"},"enabled":false}),&ctx).await.unwrap();
    let id = channel["id"].as_str().unwrap();
    run("update_agent_channel",json!({"agent_id":agent_id,"channel_id":id,"channel_config":{"generic_tool_text":"Reviewed activity","token":"current-private-token"}}),&ctx).await.unwrap();
    let restored = run(
        "restore_entity_revision",
        json!({"entity_ref":id,"revision":1,"expected_revision":2,"fields":["channel_config"]}),
        &ctx,
    )
    .await
    .unwrap();
    assert_eq!(
        restored["entity"]["channel_config"]["generic_tool_text"],
        "Original activity"
    );
    let row = ctx
        .db
        .get_agent_channel(ctx.org_id(), snapshot::ref_uuid(agent_id).unwrap(), id)
        .await
        .unwrap()
        .unwrap();
    let (_, channel) =
        crate::domains::agent_channels::ingress::row_to_ingress(ctx.encryption.as_ref(), row)
            .unwrap();
    assert_eq!(channel.channel_config["token"], "current-private-token");
    let history = run(
        "show_entity_revision",
        json!({"entity_ref":id,"revision":1}),
        &ctx,
    )
    .await
    .unwrap()
    .to_string();
    assert!(!history.contains("private-token"));
    assert!(
        run(
            "restore_entity_revision",
            json!({"entity_ref":id,"revision":1,"expected_revision":2,"fields":["channel_config"]}),
            &ctx
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn schedule_updates_share_the_history_transaction_and_undo_nullable_fields_across_orgs() {
    use everruns_durable::{PostgresWorkflowEventStore, Schedules};
    let mut ctx = owner();
    ctx.caller.is_platform_user = true;
    let store = Arc::new(PostgresWorkflowEventStore::new(ctx.db.pool().clone()));
    PostgresWorkflowEventStore::migrate(ctx.db.pool())
        .await
        .unwrap();
    ctx = ctx.with_workflow_store(Some(store.clone()));
    let schedule=run("create_schedule",json!({"name":"parity-schedule","cron_expression":"0 8 * * 1-5","target":{"type":"activity","name":"fixture","input":{}},"enabled":false}),&ctx).await.unwrap();
    let id = schedule["id"].as_str().unwrap();
    run(
        "update_schedule",
        json!({"schedule_id":id,"description":"Reviewed","max_concurrent":3}),
        &ctx,
    )
    .await
    .unwrap();
    let rollback: Result<(), CommandError> = crate::storage::transaction::scope(
        &ctx.db,
        async {
            run(
                "update_schedule",
                json!({"schedule_id":id,"description":"Must roll back"}),
                &ctx,
            )
            .await?;
            Err(CommandError::conflict(
                "fixture failure after schedule write",
            ))
        },
        CommandError::internal,
    )
    .await;
    assert!(rollback.is_err());
    assert_eq!(
        store
            .get_schedule(Uuid::parse_str(id).unwrap())
            .await
            .unwrap()
            .description
            .as_deref(),
        Some("Reviewed")
    );
    let mut other = ctx.clone();
    other.caller.org_id += 100;
    let restored=run("restore_entity_revision",json!({"entity_ref":id,"kind":"schedule","revision":1,"expected_revision":2,"fields":["description","max_concurrent"]}),&other).await.unwrap();
    assert!(restored["entity"]["description"].is_null());
    assert!(restored["entity"]["max_concurrent"].is_null());
}

#[tokio::test]
async fn nested_eval_case_restore_resolves_its_current_parent_locator() {
    let ctx = owner();
    let eval = run("create_eval", json!({"name":"parity-eval"}), &ctx)
        .await
        .unwrap();
    let eval_id = eval["id"].as_str().unwrap();
    let case=run("create_eval_case",json!({"eval_id":eval_id,"name":"Original case","conversation":[{"content":"Fixture input"}],"scorers":[]}),&ctx).await.unwrap();
    let case_id = case["id"].as_str().unwrap();
    run(
        "update_eval_case",
        json!({"eval_id":eval_id,"case_id":case_id,"name":"Reviewed case"}),
        &ctx,
    )
    .await
    .unwrap();
    let restored = run(
        "restore_entity_revision",
        json!({"entity_ref":case_id,"revision":1,"expected_revision":2,"fields":["name"]}),
        &ctx,
    )
    .await
    .unwrap();
    assert_eq!(restored["entity"]["name"], "Original case");
}

#[tokio::test]
async fn resource_guard_serializes_a_nontransactional_writer_and_releases_on_cancellation() {
    use std::time::Duration;
    use tokio::sync::oneshot;
    let ctx = owner();
    let key = super::mutation_guard::Key::new(
        &ctx,
        EntityKind::Model,
        "model_01933b5a000070008000000000000099",
    );
    let (entered, ready) = oneshot::channel();
    let (release, gate) = oneshot::channel::<()>();
    let writer_ctx = ctx.clone();
    let writer_key = key.clone();
    let writer = tokio::spawn(async move {
        super::mutation_guard::run(&writer_ctx, writer_key, false, async {
            let _ = entered.send(());
            let _ = gate.await;
            Ok(())
        })
        .await
    });
    ready.await.unwrap();
    let inverse_ctx = ctx.clone();
    let inverse_key = key.clone();
    let (inverse_entered, inverse_ready) = oneshot::channel();
    let inverse = tokio::spawn(async move {
        super::mutation_guard::run(&inverse_ctx, inverse_key, true, async {
            let _ = inverse_entered.send(());
            Ok(())
        })
        .await
    });
    let mut inverse_ready = Box::pin(inverse_ready);
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut inverse_ready)
            .await
            .is_err(),
        "inverse must wait for the ordinary writer"
    );
    writer.abort();
    drop(release);
    tokio::time::timeout(Duration::from_secs(3), inverse_ready)
        .await
        .unwrap()
        .unwrap();
    inverse.await.unwrap().unwrap();
    assert!(writer.await.unwrap_err().is_cancelled());
    // A nested command uses the same held key instead of acquiring another connection.
    tokio::time::timeout(
        Duration::from_secs(3),
        super::mutation_guard::run(
            &ctx,
            key.clone(),
            true,
            super::mutation_guard::run(&ctx, key, true, async { Ok::<_, CommandError>(()) }),
        ),
    )
    .await
    .unwrap()
    .unwrap();
}

#[tokio::test]
async fn organization_default_harness_undo_clears_to_the_initial_setting() {
    let mut ctx = owner();
    let user = ctx
        .db
        .create_user(crate::storage::CreateUserRow {
            email: "org-parity@example.test".into(),
            name: "Org reviewer".into(),
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
    let org = ctx
        .db
        .create_organization(crate::storage::CreateOrganizationRow {
            public_id: format!("org_{}", Uuid::now_v7().simple()),
            name: "Parity organization".into(),
            created_by: Some(user.id),
        })
        .await
        .unwrap();
    ctx.db
        .add_organization_member(org.org_id, user.id, "owner")
        .await
        .unwrap();
    ctx.caller.org_id = org.org_id;
    ctx.caller.org_public_id = org.public_id.clone();
    ctx.caller.user_id = Some(user.id);
    crate::setup::org_init::initialize_org_harnesses(&ctx.db, ctx.org_id())
        .await
        .unwrap();
    ctx.db
        .patch_organization_settings(
            ctx.org_id(),
            crate::storage::UpdateOrganizationSettings {
                default_harness_id: crate::storage::UpdateField::Clear,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let harness = run(
        "create_harness",
        json!({"name":"parity-default","system_prompt":"Fixture"}),
        &ctx,
    )
    .await
    .unwrap();
    run(
        "update_org",
        json!({"org":org.public_id,"default_harness_id":harness["id"]}),
        &ctx,
    )
    .await
    .unwrap();
    let restored=run("restore_entity_revision",json!({"entity_ref":org.public_id,"revision":1,"expected_revision":2,"fields":["default_harness_id"]}),&ctx).await.unwrap();
    assert!(restored["entity"]["default_harness_id"].is_null());
}

#[tokio::test]
async fn provider_diagnostics_undo_keeps_current_header_names_and_values() {
    let ctx = owner();
    let provider=run("create_provider",json!({"name":"header-parity","provider_type":"openai","request_options":{"headers":[{"name":"x-first-fixture","value":"first-private-header"}],"cache_diagnostics":false}}),&ctx).await.unwrap();
    let id = provider["id"].as_str().unwrap();
    run("update_provider",json!({"id":id,"request_options":{"headers":[{"name":"x-current-fixture","value":"current-private-header"}],"cache_diagnostics":true}}),&ctx).await.unwrap();
    let restored = run(
        "restore_entity_revision",
        json!({"entity_ref":id,"revision":1,"expected_revision":2,"fields":["request_options"]}),
        &ctx,
    )
    .await
    .unwrap();
    assert_eq!(
        restored["entity"]["request_options"]["cache_diagnostics"],
        false
    );
    let row = ctx
        .db
        .get_provider(ctx.org_id(), snapshot::ref_uuid(id).unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        row.settings["request_options"]["headers"],
        json!([{"name":"x-current-fixture","value":"current-private-header"}])
    );
    let shown = run(
        "show_entity_revision",
        json!({"entity_ref":id,"revision":1}),
        &ctx,
    )
    .await
    .unwrap()
    .to_string();
    assert!(!shown.contains("private-header"));
    assert!(!shown.contains("x-first-fixture"));
}

#[tokio::test]
async fn guarded_undo_refuses_a_definition_changed_without_a_journal_revision() {
    let ctx = owner();
    let account = ctx
        .db
        .create_organization_connection(crate::storage::CreateOrganizationConnectionRow {
            org_id: ctx.org_id(),
            name: "Original".into(),
            provider: "daytona".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let id = account.id.to_string();
    run(
        "update_organization_account_name",
        json!({"id":id,"name":"Reviewed"}),
        &ctx,
    )
    .await
    .unwrap();
    ctx.db
        .rename_organization_connection(ctx.org_id(), account.id, "Unrecorded foreign edit")
        .await
        .unwrap();
    let failure=run("restore_entity_revision",json!({"entity_ref":id,"kind":"organization_account","revision":1,"expected_revision":2,"fields":["name"]}),&ctx).await.unwrap_err();
    assert_eq!(failure.code.as_deref(), Some("entity_revision_changed"));
    assert_eq!(
        ctx.db
            .get_organization_connection(ctx.org_id(), account.id)
            .await
            .unwrap()
            .unwrap()
            .name
            .as_deref(),
        Some("Unrecorded foreign edit")
    );
}

#[tokio::test]
async fn guarded_undo_bypasses_a_live_stale_configuration_cache() {
    let ctx = owner();
    crate::setup::org_init::initialize_org_harnesses(&ctx.db, DEFAULT_ORG_ID)
        .await
        .unwrap();
    let harness = run(
        "create_harness",
        json!({"name":"cached-review","description":"Original"}),
        &ctx,
    )
    .await
    .unwrap();
    let id = harness["id"].as_str().unwrap();
    let harness_id = id
        .parse::<everruns_contracts::typed_id::HarnessId>()
        .unwrap();
    run(
        "update_harness",
        json!({"id":id,"description":"Reviewed"}),
        &ctx,
    )
    .await
    .unwrap();
    // Hold back invalidation delivery to exercise the interval after commit
    // when ordinary reads may still see the old configuration.
    ctx.db.config_row_cache().set_live(true);
    assert_eq!(
        ctx.db
            .get_harness(ctx.org_id(), harness_id)
            .await
            .unwrap()
            .unwrap()
            .description
            .as_deref(),
        Some("Reviewed")
    );
    sqlx::query(
        "UPDATE harnesses SET description='Unrecorded foreign edit' WHERE id=$1 AND org_id=$2",
    )
    .bind(harness_id)
    .bind(ctx.org_id())
    .execute(ctx.db.pool())
    .await
    .unwrap();
    assert_eq!(
        ctx.db
            .get_harness(ctx.org_id(), harness_id)
            .await
            .unwrap()
            .unwrap()
            .description
            .as_deref(),
        Some("Reviewed"),
        "the ordinary cache read must really be stale for this regression"
    );
    let failure = run(
        "restore_entity_revision",
        json!({"entity_ref":id,"revision":1,"expected_revision":2,"fields":["description"]}),
        &ctx,
    )
    .await
    .unwrap_err();
    assert_eq!(failure.code.as_deref(), Some("entity_revision_changed"));
    let actual: String =
        sqlx::query_scalar("SELECT description FROM harnesses WHERE id=$1 AND org_id=$2")
            .bind(harness_id)
            .bind(ctx.org_id())
            .fetch_one(ctx.db.pool())
            .await
            .unwrap();
    assert_eq!(actual, "Unrecorded foreign edit");
    assert_eq!(history(&ctx, id).await[0]["revision"], 2);
    ctx.db.config_row_cache().set_live(false);
}

#[tokio::test]
async fn skill_content_history_round_trips_yaml_and_restores_the_instructions() {
    let ctx = owner();
    let original = "---\nname: parity-skill\ndescription: 'A description: with punctuation'\nmetadata:\n  zebra: last\n  alpha: first\n---\n\nOriginal instructions.\n";
    let skill = run("create_skill", json!({"skill_md":original}), &ctx)
        .await
        .unwrap();
    let id = skill["id"].as_str().unwrap();
    let content = run("get_skill_content", json!({"id":id}), &ctx)
        .await
        .unwrap();
    let repeated = run("get_skill_content", json!({"id":id}), &ctx)
        .await
        .unwrap();
    assert_eq!(
        content, repeated,
        "frontmatter ordering must be stable for revision hashes"
    );
    run("update_skill",json!({"id":id,"skill_md":original.replace("Original instructions.","Reviewed instructions.")}),&ctx).await.unwrap();
    let restored = run(
        "restore_entity_revision",
        json!({"entity_ref":id,"revision":1,"expected_revision":2,"fields":["skill_md"]}),
        &ctx,
    )
    .await
    .unwrap();
    assert!(
        restored["entity"]["skill_md"]
            .as_str()
            .unwrap()
            .contains("Original instructions.")
    );
    assert_eq!(
        run("get_skill_content", json!({"id":id}), &ctx)
            .await
            .unwrap(),
        content
    );
}

#[tokio::test]
async fn organization_chat_default_undo_preserves_the_agent_default_and_rejects_stale_undo() {
    let mut ctx = owner();
    let user = ctx
        .db
        .create_user(crate::storage::CreateUserRow {
            email: "chat-default-review@example.test".into(),
            name: "Chat reviewer".into(),
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
    ctx.db
        .add_organization_member(ctx.org_id(), user.id, "owner")
        .await
        .unwrap();
    ctx.caller.user_id = Some(user.id);
    let provider = ctx
        .db
        .create_provider(
            ctx.org_id(),
            crate::storage::CreateProviderRow {
                name: "Chat review fixture".into(),
                provider_type: "llmsim".into(),
                base_url: None,
                api_key_encrypted: None,
                settings: None,
            },
        )
        .await
        .unwrap();
    let mut models = Vec::new();
    for name in ["agent-default", "chat-default"] {
        let mut profile = everruns_contracts::ModelProfile::minimal(name);
        profile.tool_call = true;
        let model = ctx
            .db
            .create_model(
                ctx.org_id(),
                crate::storage::CreateModelRow {
                    provider_id: provider.id,
                    model_id: name.into(),
                    display_name: name.into(),
                    capabilities: vec![],
                    is_favorite: false,
                    enabled: true,
                    source: "manual".into(),
                    provider_metadata: Some(json!({"discovered_profile": profile})),
                },
            )
            .await
            .unwrap();
        models.push(json!(model.id));
    }
    let org = ctx.caller.org_public_id.clone();
    let selected = run(
        "update_org",
        json!({"org":org,"default_model_id":models[0],"chat_model_id":models[1]}),
        &ctx,
    )
    .await
    .unwrap();
    assert_eq!(selected["chat_model_id"], models[1]);
    let cleared = run("update_org", json!({"org":org,"chat_model_id":null}), &ctx)
        .await
        .unwrap();
    assert!(cleared["chat_model_id"].is_null());
    let restored = run(
        "restore_entity_revision",
        json!({"entity_ref":org,"revision":2,"expected_revision":3,"fields":["chat_model_id"]}),
        &ctx,
    )
    .await
    .unwrap();
    assert_eq!(restored["entity"]["chat_model_id"], models[1]);
    assert_eq!(restored["entity"]["default_model_id"], models[0]);
    run(
        "update_org",
        json!({"org":org,"small_model_id":models[0]}),
        &ctx,
    )
    .await
    .unwrap();
    let stale = run(
        "restore_entity_revision",
        json!({"entity_ref":org,"revision":2,"expected_revision":4,"fields":["chat_model_id"]}),
        &ctx,
    )
    .await
    .unwrap_err();
    assert_eq!(stale.code.as_deref(), Some("entity_revision_changed"));
}

#[test]
fn pre_localization_snapshots_match_empty_overlays_but_detect_edits() {
    for kind in [EntityKind::Agent, EntityKind::Harness] {
        let old = json!({"name":"original"});
        let empty = json!({"name":"original", "localizations":{}});
        let translated =
            json!({"name":"original", "localizations":{"uk":{"display_name":"Назва"}}});
        assert!(super::snapshot::matches_current(kind, &old, &empty));
        assert!(super::snapshot::matches_current(kind, &empty, &old));
        assert!(!super::snapshot::matches_current(kind, &old, &translated));
        assert!(!super::snapshot::matches_current(kind, &translated, &empty));
    }
}

#[test]
fn restoring_pre_localization_history_clears_current_overlays() {
    for kind in [EntityKind::Agent, EntityKind::Harness] {
        let (_, params) = super::snapshot::restore_call(
            kind,
            "resource",
            &json!({"name":"original"}),
            &json!({"name":"original", "localizations":{"uk":{"display_name":"Назва"}}}),
        )
        .unwrap();
        assert_eq!(params["localizations"], json!({}));
    }
}

#[tokio::test]
async fn harness_localizations_follow_partial_updates_and_revision_restore() {
    let ctx = owner();
    let harness = run(
        "create_harness",
        json!({"name":"translated-harness","localizations":{"de":{"display_name":"Hilfe"}}}),
        &ctx,
    )
    .await
    .unwrap();
    let id = harness["id"].as_str().unwrap();
    run(
        "update_harness",
        json!({"id":id,"description":"New description"}),
        &ctx,
    )
    .await
    .unwrap();
    let current = run("get_harness", json!({"id":id}), &ctx).await.unwrap();
    assert_eq!(current["localizations"]["de"]["display_name"], "Hilfe");
    run("update_harness", json!({"id":id,"localizations":{}}), &ctx)
        .await
        .unwrap();
    let restored = run(
        "restore_entity_revision",
        json!({"entity_ref":id,"revision":1,"expected_revision":3,"fields":["localizations"]}),
        &ctx,
    )
    .await
    .unwrap();
    assert_eq!(
        restored["entity"]["localizations"]["de"]["display_name"],
        "Hilfe"
    );
    assert_eq!(restored["entity"]["description"], "New description");
}

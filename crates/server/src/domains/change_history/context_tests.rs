// Manager context through `Command::run`: who may read and write it, the
// revision contract, the self rule, and that it never reaches the entity.

use std::sync::Arc;

use everruns_core::{Caller, DEFAULT_ORG_ID, organization::OrgRole};
use serde_json::{Value, json};
use uuid::Uuid;

use super::intent::{ChangeIntent, ChangeSurface};
use super::registry::EntityKind;
use crate::domains::common::*;
use crate::storage::StorageBackend;

fn caller(role: OrgRole) -> Caller {
    Caller {
        org_id: DEFAULT_ORG_ID,
        org_public_id: "org_00000000000000000000000000000001".to_string(),
        user_id: Some(Uuid::from_u128(7)),
        role,
        is_platform_user: false,
        is_internal: false,
    }
}

fn owner() -> Ctx {
    Ctx::minimal_for_test(
        caller(OrgRole::Owner),
        Arc::new(StorageBackend::test_database()),
        None,
    )
}

async fn run(name: &str, params: Value, ctx: &Ctx) -> Result<Value, CommandError> {
    let output = dispatch(name, params, ctx).await?;
    Ok(serde_json::from_str(&output).unwrap_or(Value::Null))
}

async fn workspace(ctx: &Ctx, name: &str) -> String {
    run("create_workspace", json!({ "name": name }), ctx)
        .await
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string()
}

fn with_reason(ctx: &Ctx, reason: &str) -> Ctx {
    ctx.clone().with_change_intent(
        ChangeIntent::on(ChangeSurface::Commands).with_reason(Some(reason.into())),
    )
}

#[tokio::test]
async fn managers_set_append_and_clear_context_and_each_write_is_history() {
    let ctx = owner();
    let id = workspace(&ctx, "support").await;

    let empty = run("get_manager_context", json!({ "entity_ref": id }), &ctx)
        .await
        .unwrap();
    assert_eq!(empty["revision"], 0);
    assert_eq!(empty["content"], "");

    let ctx = with_reason(&ctx, "product review decisions");
    let set = run(
        "set_manager_context",
        json!({ "entity_ref": id, "content": "Owned by support.", "expected_revision": 0 }),
        &ctx,
    )
    .await
    .unwrap();
    assert_eq!(set["revision"], 1);
    let appended = run(
        "append_manager_context",
        json!({ "entity_ref": id, "text": "Keep it kid friendly." }),
        &ctx,
    )
    .await
    .unwrap();
    assert_eq!(appended["revision"], 2);
    assert_eq!(
        appended["content"],
        "Owned by support.\n\nKeep it kid friendly."
    );
    let cleared = run(
        "clear_manager_context",
        json!({ "entity_ref": id, "expected_revision": 2 }),
        &ctx,
    )
    .await
    .unwrap();
    assert_eq!(cleared["revision"], 3);
    assert_eq!(cleared["content"], "");

    let history = run(
        "list_entity_history",
        json!({ "entity_ref": id, "action": "context_updated" }),
        &ctx,
    )
    .await
    .unwrap();
    let history = history.as_array().unwrap();
    assert_eq!(history.len(), 3);
    assert!(
        history
            .iter()
            .all(|entry| entry["reason"] == "product review decisions")
    );
    assert_eq!(history[0]["command"], "clear_manager_context");
    assert_eq!(history[0]["changed_fields"], json!(["manager_context"]));
}

#[tokio::test]
async fn a_stale_expected_revision_is_refused_with_a_way_to_recover() {
    let ctx = owner();
    let id = workspace(&ctx, "w").await;
    run(
        "append_manager_context",
        json!({ "entity_ref": id, "text": "a" }),
        &ctx,
    )
    .await
    .unwrap();
    let err = run(
        "set_manager_context",
        json!({ "entity_ref": id, "content": "b", "expected_revision": 0 }),
        &ctx,
    )
    .await
    .expect_err("stale");
    assert_eq!(err.code.as_deref(), Some("manager_context_changed"));
    assert!(matches!(err.kind, CommandErrorKind::Conflict(_)), "{err}");
    assert_eq!(
        err.allowed_actions[0].operation_id.as_deref(),
        Some("get_manager_context")
    );
}

#[tokio::test]
async fn context_needs_the_kinds_manage_policy_and_an_existing_entity() {
    let ctx = owner();
    let id = workspace(&ctx, "w").await;
    let member = Ctx::minimal_for_test(caller(OrgRole::Member), ctx.db.clone(), None);
    let err = run("get_manager_context", json!({ "entity_ref": id }), &member)
        .await
        .expect_err("a member does not manage workspaces");
    assert!(matches!(err.kind, CommandErrorKind::Forbidden(_)), "{err}");

    let missing = "wsp_01933b5a000070008000000000000099";
    let err = run(
        "append_manager_context",
        json!({ "entity_ref": missing, "text": "x" }),
        &ctx,
    )
    .await
    .expect_err("no such workspace");
    assert!(matches!(err.kind, CommandErrorKind::NotFound(_)), "{err}");

    let err = run(
        "get_manager_context",
        json!({ "entity_ref": "kbe_01933b5a000070008000000000000001" }),
        &ctx,
    )
    .await
    .expect_err("entries keep notes on their parent");
    assert!(matches!(err.kind, CommandErrorKind::BadRequest(_)), "{err}");
}

#[tokio::test]
async fn a_change_is_held_to_the_context_revision_it_acknowledges() {
    let ctx = owner();
    let id = workspace(&ctx, "w").await;
    run(
        "append_manager_context",
        json!({ "entity_ref": id, "text": "Do not rename." }),
        &ctx,
    )
    .await
    .unwrap();

    // Stale: refused before anything changes.
    let stale = ctx.clone().with_change_intent(ChangeIntent {
        context_revision: Some(0),
        ..ChangeIntent::on(ChangeSurface::Commands)
    });
    let err = run(
        "update_workspace",
        json!({ "workspace_id": id, "name": "x" }),
        &stale,
    )
    .await
    .expect_err("stale context revision");
    assert_eq!(err.code.as_deref(), Some("manager_context_changed"));
    let fetched = run("get_workspace", json!({ "workspace_id": id }), &ctx)
        .await
        .unwrap();
    assert_eq!(fetched["name"], "w");

    // Unacknowledged: allowed, with a notice naming the revision.
    let silent = ctx
        .clone()
        .with_change_intent(ChangeIntent::on(ChangeSurface::Commands));
    let notices = silent.change_intent.as_ref().unwrap().notices.clone();
    run(
        "update_workspace",
        json!({ "workspace_id": id, "name": "y" }),
        &silent,
    )
    .await
    .unwrap();
    let notices = notices.take();
    assert_eq!(notices.len(), 1, "{notices:?}");
    assert!(notices[0].contains("--context-revision 1"), "{notices:?}");

    // Acknowledged: allowed, no notice.
    let acked = ctx.clone().with_change_intent(ChangeIntent {
        context_revision: Some(1),
        ..ChangeIntent::on(ChangeSurface::Commands)
    });
    let notices = acked.change_intent.as_ref().unwrap().notices.clone();
    run(
        "update_workspace",
        json!({ "workspace_id": id, "name": "z" }),
        &acked,
    )
    .await
    .unwrap();
    assert!(notices.take().is_empty());
}

#[tokio::test]
async fn deleting_an_entity_deletes_its_context() {
    let ctx = owner();
    let id = workspace(&ctx, "w").await;
    run(
        "append_manager_context",
        json!({ "entity_ref": id, "text": "note" }),
        &ctx,
    )
    .await
    .unwrap();
    run("delete_workspace", json!({ "workspace_id": id }), &ctx)
        .await
        .unwrap();
    let key = crate::storage::manager_context::ManagerContextKey {
        org_id: DEFAULT_ORG_ID,
        entity_kind: "workspace".into(),
        entity_ref: id,
    };
    assert_eq!(ctx.db.get_manager_context(&key).await.unwrap(), None);
}

#[tokio::test]
async fn a_session_acting_for_an_entity_cannot_read_its_own_context_or_history() {
    let ctx = owner();
    let id = workspace(&ctx, "w").await;
    let other = workspace(&ctx, "other").await;
    let acting = ctx.clone().with_change_intent(ChangeIntent {
        via_session_id: Some(Uuid::from_u128(5)),
        via_agent_id: Some(id.clone()),
        ..ChangeIntent::on(ChangeSurface::Platform)
    });
    for (name, params) in [
        ("get_manager_context", json!({ "entity_ref": id })),
        (
            "append_manager_context",
            json!({ "entity_ref": id, "text": "x" }),
        ),
        ("list_entity_history", json!({ "entity_ref": id })),
    ] {
        let err = run(name, params, &acting).await.expect_err(name);
        assert_eq!(
            err.code.as_deref(),
            Some("self_inspection_denied"),
            "{name}"
        );
    }
    // Another entity it manages stays reachable.
    run(
        "get_manager_context",
        json!({ "entity_ref": other }),
        &acting,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn context_never_reaches_the_entity_or_its_export() {
    let ctx = owner();
    let harness = ctx
        .db
        .create_harness(
            DEFAULT_ORG_ID,
            crate::storage::models::CreateHarnessRow {
                name: "h".into(),
                display_name: None,
                icon: None,
                description: None,
                intro_markdown: None,
                short_description: None,
                starters: json!([]),
                system_prompt: Some(String::new()),
                parent_harness_id: None,
                default_model_id: None,
                tags: vec![],
                initial_files: json!([]),
                mcp_servers: json!({}),
                network_access: None,
                embedder_metadata: json!({}),
                is_built_in: false,
            },
        )
        .await
        .unwrap();
    let agent = everruns_contracts::typed_id::AgentId::new().to_string();
    ctx.db
        .create_agent(
            DEFAULT_ORG_ID,
            crate::storage::models::CreateAgentRow {
                public_id: agent.clone(),
                name: "kids".into(),
                display_name: None,
                description: None,
                intro_markdown: None,
                short_description: None,
                starters: json!([]),
                system_prompt: "Be helpful.".into(),
                default_model_id: None,
                harness_id: harness.id,
                tags: vec![],
                initial_files: json!([]),
                tools: json!([]),
                mcp_servers: json!({}),
                network_access: None,
                max_iterations: None,
                parallel_tool_calls: None,
                environments: None,
                is_built_in: false,
            },
        )
        .await
        .unwrap();
    let marker = "MANAGER-ONLY-7f3a";
    run(
        "append_manager_context",
        json!({ "entity_ref": agent, "text": marker }),
        &ctx,
    )
    .await
    .unwrap();
    for (name, params) in [
        ("get_agent", json!({ "id": agent })),
        ("export_agent", json!({ "id": agent })),
    ] {
        let output = dispatch(name, params, &ctx).await.expect(name);
        assert!(!output.contains(marker), "{name} leaked manager context");
    }
}

/// Every lookup a kind declares names a read command that takes that param,
/// or confirming an entity exists would fail at runtime.
#[test]
fn every_kind_lookup_names_a_read_command_and_its_param() {
    let mut broken = Vec::new();
    for kind in EntityKind::ALL {
        let Some((name, param)) = kind.lookup() else {
            continue;
        };
        let desc = inventory::iter::<CommandDescriptor>
            .into_iter()
            .find(|desc| (desc.meta)().name == name);
        let Some(desc) = desc else {
            broken.push(format!("{}: no command {name}", kind.as_str()));
            continue;
        };
        let fields =
            crate::services::command_catalog::catalog::schema_field_paths(&(desc.param_schema)());
        if !(desc.read_only)() || !fields.iter().any(|field| field == param) {
            broken.push(format!(
                "{}: {name} {param} not in {fields:?}",
                kind.as_str()
            ));
        }
    }
    assert!(broken.is_empty(), "{broken:#?}");
}

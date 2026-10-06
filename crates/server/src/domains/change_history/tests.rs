// Entity history through `Command::run`, and the guards that keep every
// mutating command declared.

use std::sync::Arc;

use everruns_core::{Caller, DEFAULT_ORG_ID, organization::OrgRole};
use serde_json::{Value, json};
use uuid::Uuid;

use super::intent::{ChangeIntent, ChangeSurface, scope_http_intent};
use super::registry::{Change, SubjectId};
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

fn ctx_with(role: OrgRole) -> Ctx {
    Ctx::minimal_for_test(
        caller(role),
        Arc::new(StorageBackend::test_database()),
        None,
    )
}

async fn run(name: &str, params: Value, ctx: &Ctx) -> Result<Value, CommandError> {
    let output = dispatch(name, params, ctx).await?;
    Ok(serde_json::from_str(&output).unwrap_or(Value::Null))
}

async fn history(ctx: &Ctx, entity_ref: &str) -> Vec<Value> {
    run(
        "list_entity_history",
        json!({ "entity_ref": entity_ref }),
        ctx,
    )
    .await
    .expect("history lists")
    .as_array()
    .cloned()
    .unwrap_or_default()
}

#[tokio::test]
async fn a_change_is_recorded_with_its_reason_fields_actor_and_surface() {
    let ctx = ctx_with(OrgRole::Owner).with_change_intent(
        ChangeIntent::on(ChangeSurface::Commands)
            .with_reason(Some("  a home for the support team  ".into())),
    );
    let created = run(
        "create_workspace",
        json!({ "name": "support", "description": "team space" }),
        &ctx,
    )
    .await
    .unwrap();
    let id = created["id"].as_str().unwrap().to_string();

    let entries = history(&ctx, &id).await;
    assert_eq!(entries.len(), 1, "{entries:?}");
    let entry = &entries[0];
    assert_eq!(entry["entity_kind"], "workspace");
    assert_eq!(entry["entity_ref"], id.as_str());
    assert_eq!(entry["action"], "created");
    assert_eq!(entry["command"], "create_workspace");
    assert_eq!(entry["reason"], "a home for the support team");
    assert_eq!(entry["changed_fields"], json!(["description", "name"]));
    assert_eq!(entry["actor_kind"], "user");
    assert_eq!(entry["actor_user_id"], Uuid::from_u128(7).to_string());
    assert_eq!(entry["surface"], "commands");
}

#[tokio::test]
async fn updates_and_deletes_find_their_subject_in_the_params() {
    let ctx = ctx_with(OrgRole::Owner);
    let created = run("create_workspace", json!({ "name": "a" }), &ctx)
        .await
        .unwrap();
    let id = created["id"].as_str().unwrap().to_string();

    let ctx = ctx.with_change_intent(
        ChangeIntent::on(ChangeSurface::Mcp).with_reason(Some("rename for clarity".into())),
    );
    run(
        "update_workspace",
        json!({ "workspace_id": id, "name": "b" }),
        &ctx,
    )
    .await
    .unwrap();
    run("delete_workspace", json!({ "workspace_id": id }), &ctx)
        .await
        .unwrap();

    let entries = history(&ctx, &id).await;
    let actions: Vec<&str> = entries
        .iter()
        .map(|entry| entry["action"].as_str().unwrap())
        .collect();
    assert_eq!(actions, ["deleted", "updated", "created"]);
    assert_eq!(entries[1]["changed_fields"], json!(["name"]));
    assert_eq!(entries[1]["reason"], "rename for clarity");
    // A change without a reason is still recorded: people may omit one.
    assert_eq!(entries[2]["reason"], Value::Null);
}

#[tokio::test]
async fn rest_reaches_history_through_the_http_layer() {
    let ctx = ctx_with(OrgRole::Owner);
    let intent = ChangeIntent {
        reason: Some("from the UI".into()),
        surface: Some(ChangeSurface::Api),
        request_id: Some("req-42".into()),
        ..ChangeIntent::default()
    };
    let created = scope_http_intent(intent, async {
        crate::domains::workspaces::commands::CreateWorkspace {
            name: "ui".into(),
            description: None,
        }
        .run(&ctx)
        .await
    })
    .await
    .unwrap();

    let entries = history(&ctx, &created.id).await;
    assert_eq!(entries[0]["reason"], "from the UI");
    assert_eq!(entries[0]["surface"], "api");
    assert_eq!(entries[0]["request_id"], "req-42");
}

#[tokio::test]
async fn an_invalid_reason_fails_before_anything_changes() {
    let ctx = ctx_with(OrgRole::Owner).with_change_intent(
        ChangeIntent::on(ChangeSurface::Commands).with_reason(Some(
            "key is sk-ant-api03-abcdefghijklmnopqrstuvwxyz012345".into(),
        )),
    );
    let err = run("create_workspace", json!({ "name": "leaky" }), &ctx)
        .await
        .expect_err("a credential-shaped reason is rejected");
    assert_eq!(err.code.as_deref(), Some("invalid_change_reason"));

    let listed = run("list_workspaces", json!({}), &ctx).await.unwrap();
    assert_eq!(listed.as_array().map(Vec::len), Some(0), "{listed}");
}

#[tokio::test]
async fn a_failed_command_and_a_read_record_nothing() {
    let ctx = ctx_with(OrgRole::Owner).with_change_intent(
        ChangeIntent::on(ChangeSurface::Commands).with_reason(Some("x".into())),
    );
    run("create_workspace", json!({ "name": "dup" }), &ctx)
        .await
        .unwrap();
    run("create_workspace", json!({ "name": "dup" }), &ctx)
        .await
        .expect_err("names are unique");
    run("list_workspaces", json!({}), &ctx).await.unwrap();

    let all = run("list_org_history", json!({}), &ctx).await.unwrap();
    assert_eq!(all.as_array().map(Vec::len), Some(1), "{all}");
}

#[tokio::test]
async fn an_agent_session_is_recorded_as_the_actor() {
    let session = Uuid::from_u128(99);
    let ctx = ctx_with(OrgRole::Owner).with_change_intent(ChangeIntent {
        reason: Some("asked to make it kid friendly".into()),
        surface: Some(ChangeSurface::Platform),
        via_session_id: Some(session),
        via_agent_id: Some("agent_01933b5a000070008000000000000001".into()),
        ..ChangeIntent::default()
    });
    let created = run("create_workspace", json!({ "name": "kids" }), &ctx)
        .await
        .unwrap();
    let entry = history(&ctx, created["id"].as_str().unwrap())
        .await
        .remove(0);
    assert_eq!(entry["actor_kind"], "agent_session");
    assert_eq!(entry["surface"], "platform");
    assert_eq!(
        entry["via_session_id"],
        everruns_contracts::typed_id::SessionId::from_uuid(session).to_string()
    );
    assert_eq!(
        entry["via_agent_id"],
        "agent_01933b5a000070008000000000000001"
    );

    let by_agent = run(
        "list_org_history",
        json!({ "via_agent_id": "agent_01933b5a000070008000000000000001" }),
        &ctx,
    )
    .await
    .unwrap();
    assert_eq!(by_agent.as_array().map(Vec::len), Some(1));
}

#[tokio::test]
async fn history_is_read_with_the_kinds_view_policy() {
    let owner = ctx_with(OrgRole::Owner);
    let created = run("create_workspace", json!({ "name": "w" }), &owner)
        .await
        .unwrap();
    let id = created["id"].as_str().unwrap();

    // Org-wide history is the audit view; a member does not hold it.
    let member = Ctx::minimal_for_test(caller(OrgRole::Member), owner.db.clone(), None);
    let err = run("list_org_history", json!({}), &member)
        .await
        .expect_err("members cannot read org-wide history");
    assert!(matches!(err.kind, CommandErrorKind::Forbidden(_)), "{err}");

    // History of another org's entity is simply empty.
    let mut other = caller(OrgRole::Owner);
    other.org_id = DEFAULT_ORG_ID + 1;
    let other = Ctx::minimal_for_test(other, owner.db.clone(), None);
    assert!(history(&other, id).await.is_empty());
}

#[tokio::test]
async fn a_ref_without_a_prefix_needs_a_kind() {
    let ctx = ctx_with(OrgRole::Owner);
    let err = run(
        "list_entity_history",
        json!({ "entity_ref": "01933b5a-0000-7000-8000-000000000001" }),
        &ctx,
    )
    .await
    .expect_err("no prefix, no kind");
    assert!(err.to_string().contains("--kind"), "{err}");
    run(
        "list_entity_history",
        json!({ "entity_ref": "01933b5a-0000-7000-8000-000000000001", "kind": "saved_report" }),
        &ctx,
    )
    .await
    .unwrap();
}

// ---------------------------------------------------------------------------
// Guards
// ---------------------------------------------------------------------------

fn descriptors() -> impl Iterator<Item = &'static CommandDescriptor> {
    inventory::iter::<CommandDescriptor>
        .into_iter()
        .filter(|desc| !(desc.meta)().path.starts_with("/test/"))
}

/// Every mutating command says what it changes, or why it changes no entity.
/// A new command fails here until it is added to `registry::declared`.
#[test]
fn every_mutating_command_declares_what_it_changes() {
    let mut undeclared: Vec<&str> = descriptors()
        .filter(|desc| !(desc.read_only)())
        .filter(|desc| (desc.change)() == Change::Undeclared)
        .map(|desc| (desc.meta)().name)
        .collect();
    undeclared.sort();
    assert!(
        undeclared.is_empty(),
        "these mutating commands do not declare what they change; add them to \
         `change_history::registry::declared` as a Subject or an Exempt with its \
         reason: {undeclared:?}"
    );
}

#[test]
fn read_only_commands_record_nothing() {
    let recorded: Vec<&str> = descriptors()
        .filter(|desc| (desc.read_only)())
        .filter(|desc| (desc.change)() != Change::None)
        .map(|desc| (desc.meta)().name)
        .collect();
    assert!(recorded.is_empty(), "{recorded:?}");
}

/// A subject read from the params must name a real param, or the history
/// entry would silently go missing at runtime.
#[test]
fn every_param_subject_names_a_param_of_its_command() {
    let mut broken = Vec::new();
    for desc in descriptors() {
        let Change::Subject {
            id: SubjectId::Param(field),
            ..
        } = (desc.change)()
        else {
            continue;
        };
        let fields = crate::api::mcp_endpoint::catalog::schema_field_paths(&(desc.param_schema)());
        if !fields.iter().any(|path| path == field) {
            broken.push(format!("{}: {field} not in {fields:?}", (desc.meta)().name));
        }
    }
    assert!(broken.is_empty(), "{broken:#?}");
}

/// The invocation metadata names are reserved: a command param called
/// `reason` would be swallowed by the shells' global `--reason`.
#[test]
fn no_command_takes_a_param_named_like_invocation_metadata() {
    let mut clashes = Vec::new();
    for desc in descriptors() {
        let fields = crate::api::mcp_endpoint::catalog::schema_field_paths(&(desc.param_schema)());
        for reserved in super::intent::RESERVED_PARAMS {
            if fields.iter().any(|path| path == reserved) {
                clashes.push(format!("{}.{reserved}", (desc.meta)().name));
            }
        }
    }
    assert!(clashes.is_empty(), "{clashes:?}");
}

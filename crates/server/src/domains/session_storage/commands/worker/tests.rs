use crate::domains::common::{Ctx, dispatch, *};
use crate::storage::{CreateSessionRow, StorageBackend};
use everruns_contracts::typed_id::{PrincipalId, SessionId};
use everruns_core::organization::OrgRole;
use everruns_core::{Caller, DEFAULT_ORG_ID};
use serde_json::{Value, json};
use std::sync::Arc;

const COMMANDS: [&str; 5] = [
    "worker_set_session_storage_value",
    "worker_get_session_storage_value",
    "worker_take_session_storage_value",
    "worker_delete_session_storage_value",
    "worker_list_session_storage_keys",
];

async fn session(db: &Arc<StorageBackend>, org_id: i64) -> SessionId {
    db.create_session(CreateSessionRow {
        org_id,
        owner_principal_id: PrincipalId::from_seed(1),
        title: Some("storage".to_string()),
        ..Default::default()
    })
    .await
    .unwrap()
    .id
}

fn worker_ctx(db: Arc<StorageBackend>, org_id: i64) -> Ctx {
    Ctx::minimal_for_test(Caller::internal(org_id), db, None)
}

async fn run(ctx: &Ctx, name: &str, params: Value) -> Result<Value, CommandError> {
    dispatch(name, params, ctx)
        .await
        .map(|json| serde_json::from_str(&json).unwrap())
}

fn entry(session_id: SessionId, key: &str) -> Value {
    json!({ "session_id": session_id.to_string(), "key": key, "value": "v1" })
}

#[tokio::test]
async fn the_worker_value_store_round_trips_through_dispatch() {
    let db = Arc::new(StorageBackend::test_database());
    let session_id = session(&db, DEFAULT_ORG_ID).await;
    let ctx = worker_ctx(db, DEFAULT_ORG_ID);

    run(
        &ctx,
        "worker_set_session_storage_value",
        entry(session_id, "a"),
    )
    .await
    .expect("set");
    // Internal keys are the runtime's own; the worker reads and lists them.
    let internal = "tool_approval/decision";
    run(
        &ctx,
        "worker_set_session_storage_value",
        entry(session_id, internal),
    )
    .await
    .expect("set internal");

    let read = run(
        &ctx,
        "worker_get_session_storage_value",
        entry(session_id, "a"),
    )
    .await
    .expect("get");
    assert_eq!(read, json!("v1"));
    let keys = run(
        &ctx,
        "worker_list_session_storage_keys",
        json!({ "session_id": session_id.to_string() }),
    )
    .await
    .expect("list");
    let mut names: Vec<&str> = keys
        .as_array()
        .unwrap()
        .iter()
        .map(|key| key["key"].as_str().unwrap())
        .collect();
    names.sort();
    assert_eq!(names, ["a", internal]);
    assert!(keys[0]["created_at"].is_string());

    let taken = run(
        &ctx,
        "worker_take_session_storage_value",
        entry(session_id, internal),
    )
    .await
    .expect("take");
    assert_eq!(taken, json!("v1"));
    let again = run(
        &ctx,
        "worker_take_session_storage_value",
        entry(session_id, internal),
    )
    .await
    .expect("take again");
    assert!(again.is_null(), "a take removes the value");

    let deleted = run(
        &ctx,
        "worker_delete_session_storage_value",
        entry(session_id, "a"),
    )
    .await
    .expect("delete");
    assert_eq!(deleted, json!(true));
    let gone = run(
        &ctx,
        "worker_get_session_storage_value",
        entry(session_id, "a"),
    )
    .await
    .expect("get after delete");
    assert!(gone.is_null());
    let absent = run(
        &ctx,
        "worker_delete_session_storage_value",
        entry(session_id, "a"),
    )
    .await
    .expect("delete of an absent key");
    assert_eq!(absent, json!(false));
}

/// THREAT[TM-TENANT-001]: a worker acting for one org cannot write, read,
/// take, delete, or list another org's session storage by naming the session.
/// The RPCs these replace took the session alone.
#[tokio::test]
async fn another_orgs_session_is_not_found() {
    let db = Arc::new(StorageBackend::test_database());
    let foreign = session(&db, DEFAULT_ORG_ID).await;
    let owner_ctx = worker_ctx(db.clone(), DEFAULT_ORG_ID);
    run(
        &owner_ctx,
        "worker_set_session_storage_value",
        entry(foreign, "a"),
    )
    .await
    .expect("set in the owning org");
    let ctx = worker_ctx(db, DEFAULT_ORG_ID + 1);

    for name in COMMANDS {
        let error = run(&ctx, name, entry(foreign, "a")).await.expect_err(name);
        assert!(
            matches!(error.kind, CommandErrorKind::NotFound(_)),
            "{name}: {error:?}"
        );
    }

    let still = run(
        &owner_ctx,
        "worker_get_session_storage_value",
        entry(foreign, "a"),
    )
    .await
    .unwrap();
    assert_eq!(still, json!("v1"), "the foreign worker changed nothing");
}

/// THREAT[TM-AUTHZ-002]: the worker value store is not a person's API. An
/// owner, who passes every session policy, is still refused.
#[tokio::test]
async fn a_person_cannot_run_internal_commands() {
    let db = Arc::new(StorageBackend::test_database());
    let session_id = session(&db, DEFAULT_ORG_ID).await;
    let owner = Caller {
        is_internal: false,
        user_id: Some(uuid::Uuid::nil()),
        role: OrgRole::Owner,
        ..Caller::internal(DEFAULT_ORG_ID)
    };
    let ctx = Ctx::minimal_for_test(owner, db, None);

    for name in COMMANDS {
        let error = run(&ctx, name, entry(session_id, "a"))
            .await
            .expect_err(name);
        assert!(
            matches!(error.kind, CommandErrorKind::Forbidden(_)),
            "{name}: {error:?}"
        );
    }
}

/// Internal commands stay off every public surface: discovery, the scripted
/// toolset (MCP, Platform, `/v1/commands`), and the command tree. The public
/// `list_session_storage` stays where it was.
#[test]
fn internal_commands_are_on_no_public_surface() {
    let flags = all_feature_flags_for_test();
    let discovered: Vec<&str> = catalog_entries_with_schemas(false, &flags)
        .into_iter()
        .map(|entry| entry.name)
        .chain(catalog_entries().into_iter().map(|meta| meta.name))
        .collect();
    let contracts = crate::services::command_catalog::cli_tree::contracts();
    assert!(discovered.contains(&"list_session_storage"));

    for name in COMMANDS {
        let desc = inventory::iter::<CommandDescriptor>
            .into_iter()
            .find(|desc| (desc.meta)().name == name)
            .unwrap_or_else(|| panic!("{name} is registered"));
        assert!((desc.meta)().is_internal(), "{name}");
        assert!(!discovered.contains(&name), "{name} is discoverable");
        for mode in [
            crate::services::command_catalog::catalog::ToolsetMode::Full,
            crate::services::command_catalog::catalog::ToolsetMode::ReadOnly,
        ] {
            assert!(
                crate::services::command_catalog::catalog::scripted_descriptor(name, mode)
                    .is_none(),
                "{name} is scriptable"
            );
        }
        assert!(
            contracts.iter().all(|contract| contract.wire_name != name),
            "{name} is on the command line"
        );
    }
}

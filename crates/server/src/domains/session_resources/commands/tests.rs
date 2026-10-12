use crate::domains::common::{Ctx, dispatch, *};
use crate::storage::{CreateHarnessRow, CreateSessionRow, StorageBackend};
use everruns_contracts::typed_id::{PrincipalId, SessionId};
use everruns_core::organization::OrgRole;
use everruns_core::{Caller, DEFAULT_ORG_ID};
use serde_json::{Value, json};
use std::sync::Arc;

const COMMANDS: [&str; 4] = [
    "worker_register_session_resource",
    "worker_update_session_resource_status",
    "worker_list_session_resources",
    "worker_deregister_session_resource",
];

pub(super) async fn session(db: &Arc<StorageBackend>, org_id: i64) -> SessionId {
    // A session needs its org's built-in base harness.
    if db
        .get_harness_by_name(org_id, "base")
        .await
        .unwrap()
        .is_none()
    {
        db.create_harness(
            org_id,
            CreateHarnessRow {
                name: "base".to_string(),
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
                is_built_in: true,
                embedder_metadata: Default::default(),
            },
        )
        .await
        .unwrap();
    }
    db.create_session(CreateSessionRow {
        org_id,
        owner_principal_id: PrincipalId::from_seed(1),
        title: Some("resources".to_string()),
        ..Default::default()
    })
    .await
    .unwrap()
    .id
}

pub(super) fn worker_ctx(db: Arc<StorageBackend>, org_id: i64) -> Ctx {
    Ctx::minimal_for_test(Caller::internal(org_id), db, None)
}

pub(super) async fn run(ctx: &Ctx, name: &str, params: Value) -> Result<Value, CommandError> {
    dispatch(name, params, ctx)
        .await
        .map(|json| serde_json::from_str(&json).unwrap())
}

fn register(session_id: SessionId, resource_id: &str, kind: &str) -> Value {
    json!({
        "session_id": session_id.to_string(),
        "resource_id": resource_id,
        "kind": kind,
        "display_name": format!("{kind} {resource_id}"),
        "status": "active",
        "metadata": { "region": "eu" },
    })
}

#[tokio::test]
async fn the_worker_registry_round_trips_through_dispatch() {
    let db = Arc::new(StorageBackend::test_database());
    let session_id = session(&db, DEFAULT_ORG_ID).await;
    let ctx = worker_ctx(db, DEFAULT_ORG_ID);

    let entry = run(
        &ctx,
        "worker_register_session_resource",
        register(session_id, "sbx-1", "sandbox"),
    )
    .await
    .expect("register");
    assert_eq!(entry["resource_id"], "sbx-1");
    assert_eq!(entry["status"], "active");
    assert_eq!(entry["metadata"]["region"], "eu");
    run(
        &ctx,
        "worker_register_session_resource",
        register(session_id, "sub-1", "subagent"),
    )
    .await
    .expect("register second");

    let all = run(
        &ctx,
        "worker_list_session_resources",
        json!({ "session_id": session_id.to_string() }),
    )
    .await
    .expect("list");
    assert_eq!(all.as_array().map(Vec::len), Some(2));

    let updated = run(
        &ctx,
        "worker_update_session_resource_status",
        json!({ "session_id": session_id.to_string(), "resource_id": "sbx-1", "status": "released" }),
    )
    .await
    .expect("update");
    assert_eq!(updated["status"], "released");
    let missing = run(
        &ctx,
        "worker_update_session_resource_status",
        json!({ "session_id": session_id.to_string(), "resource_id": "nope", "status": "failed" }),
    )
    .await
    .expect("update of an unregistered resource");
    assert!(missing.is_null());

    let released = run(
        &ctx,
        "worker_list_session_resources",
        json!({ "session_id": session_id.to_string(), "kind": "sandbox", "status": "released" }),
    )
    .await
    .expect("filtered list");
    assert_eq!(released.as_array().map(Vec::len), Some(1));

    let deregister = |resource_id: &'static str| {
        run(
            &ctx,
            "worker_deregister_session_resource",
            json!({ "session_id": session_id.to_string(), "resource_id": resource_id }),
        )
    };
    assert_eq!(deregister("sbx-1").await.unwrap(), true);
    assert_eq!(deregister("sbx-1").await.unwrap(), false);
}

/// THREAT[TM-TENANT-001]: a worker acting for one org cannot register, change,
/// read, or remove another org's session resources by naming the session. The
/// RPCs these replace took the session alone.
#[tokio::test]
async fn another_orgs_session_is_not_found() {
    let db = Arc::new(StorageBackend::test_database());
    let foreign = session(&db, DEFAULT_ORG_ID).await;
    let owner_ctx = worker_ctx(db.clone(), DEFAULT_ORG_ID);
    run(
        &owner_ctx,
        "worker_register_session_resource",
        register(foreign, "sbx-1", "sandbox"),
    )
    .await
    .expect("register in the owning org");
    let ctx = worker_ctx(db, DEFAULT_ORG_ID + 1);

    for name in COMMANDS {
        let mut params = register(foreign, "sbx-1", "sandbox");
        params["status"] = json!("released");
        let error = run(&ctx, name, params).await.expect_err(name);
        assert!(
            matches!(error.kind, CommandErrorKind::NotFound(_)),
            "{name}: {error:?}"
        );
    }

    let still = run(
        &owner_ctx,
        "worker_list_session_resources",
        json!({ "session_id": foreign.to_string() }),
    )
    .await
    .unwrap();
    assert_eq!(
        still[0]["status"], "active",
        "the foreign worker changed nothing"
    );
}

/// THREAT[TM-AUTHZ-002]: the worker registry is not a person's API. An owner,
/// who passes every session policy, is still refused.
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
        let error = run(&ctx, name, register(session_id, "sbx-1", "sandbox"))
            .await
            .expect_err(name);
        assert!(
            matches!(error.kind, CommandErrorKind::Forbidden(_)),
            "{name}: {error:?}"
        );
    }
}

/// Internal commands stay off every public surface: discovery, the scripted
/// toolset (MCP, Platform, `/v1/commands`), and the command tree, the leased
/// resource ones included. The public `list_session_resources` stays where it
/// was.
#[test]
fn internal_commands_are_on_no_public_surface() {
    let flags = all_feature_flags_for_test();
    let discovered: Vec<&str> = catalog_entries_with_schemas(false, &flags)
        .into_iter()
        .map(|entry| entry.name)
        .chain(catalog_entries().into_iter().map(|meta| meta.name))
        .collect();
    let contracts = crate::services::command_catalog::cli_tree::contracts();
    assert!(discovered.contains(&"list_session_resources"));

    for name in COMMANDS.into_iter().chain(super::leased::tests::COMMANDS) {
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

use crate::domains::common::{Ctx, dispatch, *};
use crate::storage::{CreateHarnessRow, CreateSessionRow, StorageBackend};
use everruns_contracts::typed_id::{PrincipalId, SessionId};
use everruns_core::organization::OrgRole;
use everruns_core::{Caller, DEFAULT_ORG_ID};
use serde_json::{Value, json};
use std::sync::Arc;

const COMMANDS: [&str; 5] = [
    "worker_create_session_schedule",
    "worker_cancel_session_schedule",
    "worker_list_session_schedules",
    "worker_count_active_session_schedules",
    "worker_count_active_org_session_schedules",
];

async fn session(db: &Arc<StorageBackend>, org_id: i64) -> SessionId {
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
        title: Some("schedules".to_string()),
        ..Default::default()
    })
    .await
    .unwrap()
    .id
}

fn worker_ctx(db: Arc<StorageBackend>) -> Ctx {
    Ctx::minimal_for_test(Caller::internal(DEFAULT_ORG_ID), db, None)
}

async fn run(ctx: &Ctx, name: &str, params: Value) -> Result<Value, CommandError> {
    dispatch(name, params, ctx)
        .await
        .map(|json| serde_json::from_str(&json).unwrap())
}

#[tokio::test]
async fn the_worker_store_round_trips_through_dispatch() {
    let db = Arc::new(StorageBackend::test_database());
    let session_id = session(&db, DEFAULT_ORG_ID).await;
    let ctx = worker_ctx(db);
    let session = json!({ "session_id": session_id.to_string() });

    let created = run(
        &ctx,
        "worker_create_session_schedule",
        json!({
            "session_id": session_id.to_string(),
            "description": "check the build",
            "cron_expression": "0 0 * * * *",
            "timezone": "UTC",
        }),
    )
    .await
    .expect("create");
    assert_eq!(created["description"], "check the build");
    assert_eq!(created["enabled"], true);

    let listed = run(&ctx, "worker_list_session_schedules", session.clone())
        .await
        .expect("list");
    assert_eq!(listed.as_array().map(Vec::len), Some(1));
    let count = |name| run(&ctx, name, session.clone());
    assert_eq!(
        count("worker_count_active_session_schedules")
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        run(&ctx, "worker_count_active_org_session_schedules", json!({}))
            .await
            .unwrap(),
        1
    );

    let cancelled = run(
        &ctx,
        "worker_cancel_session_schedule",
        json!({ "session_id": session_id.to_string(), "schedule_id": created["id"] }),
    )
    .await
    .expect("cancel");
    assert_eq!(cancelled["enabled"], false);
    assert_eq!(
        count("worker_count_active_session_schedules")
            .await
            .unwrap(),
        0
    );
}

/// A limit rejection is the one failure the tool reports back to the model, so
/// it must stay distinguishable from a store failure on the wire.
#[tokio::test]
async fn a_limit_rejection_is_unprocessable() {
    let db = Arc::new(StorageBackend::test_database());
    let session_id = session(&db, DEFAULT_ORG_ID).await;
    let ctx = worker_ctx(db);

    let error = run(
        &ctx,
        "worker_create_session_schedule",
        json!({
            "session_id": session_id.to_string(),
            "description": "every minute",
            "cron_expression": "* * * * *",
            "timezone": "UTC",
        }),
    )
    .await
    .expect_err("a one-minute cron is under the minimum interval");
    assert!(
        matches!(error.kind, CommandErrorKind::Unprocessable(_)),
        "{error:?}"
    );
}

/// THREAT[TM-TENANT-001]: a worker acting for one org cannot read or count
/// another org's session schedules by naming the session.
#[tokio::test]
async fn another_orgs_session_is_not_found() {
    let db = Arc::new(StorageBackend::test_database());
    let foreign = session(&db, DEFAULT_ORG_ID).await;
    let ctx = Ctx::minimal_for_test(Caller::internal(DEFAULT_ORG_ID + 1), db, None);

    for name in [
        "worker_list_session_schedules",
        "worker_count_active_session_schedules",
    ] {
        let error = run(&ctx, name, json!({ "session_id": foreign.to_string() }))
            .await
            .expect_err(name);
        assert!(
            matches!(error.kind, CommandErrorKind::NotFound(_)),
            "{name}: {error:?}"
        );
    }
}

/// THREAT[TM-AUTHZ-002]: the worker store is not a person's API. An owner, who
/// passes every session policy, is still refused.
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
        let error = run(
            &ctx,
            name,
            json!({
                "session_id": session_id.to_string(),
                "schedule_id": "x",
                "description": "x",
                "timezone": "UTC",
            }),
        )
        .await
        .expect_err(name);
        assert!(
            matches!(error.kind, CommandErrorKind::Forbidden(_)),
            "{name}: {error:?}"
        );
    }
}

/// Internal commands stay off every public surface: discovery, the scripted
/// toolset (MCP, Platform, `/v1/commands`), and the command tree.
#[test]
fn internal_commands_are_on_no_public_surface() {
    let flags = all_feature_flags_for_test();
    let discovered: Vec<&str> = catalog_entries_with_schemas(false, &flags)
        .into_iter()
        .map(|entry| entry.name)
        .chain(catalog_entries().into_iter().map(|meta| meta.name))
        .collect();
    let contracts = crate::services::command_catalog::cli_tree::contracts();

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

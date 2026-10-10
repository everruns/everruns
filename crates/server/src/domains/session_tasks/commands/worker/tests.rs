use crate::domains::common::{Ctx, dispatch, *};
use crate::storage::{CreateSessionRow, StorageBackend};
use everruns_contracts::typed_id::{PrincipalId, SessionId};
use everruns_core::organization::OrgRole;
use everruns_core::session_task::{CreateSessionTask, TaskLinks, TaskWakePolicy};
use everruns_core::{Caller, DEFAULT_ORG_ID};
use serde_json::{Value, json};
use std::sync::Arc;

const COMMANDS: [&str; 7] = [
    "worker_create_session_task",
    "worker_update_session_task",
    "worker_get_session_task",
    "worker_list_session_tasks",
    "worker_request_cancel_session_task",
    "worker_record_session_task_message",
    "worker_list_session_task_messages",
];

async fn session(db: &Arc<StorageBackend>, org_id: i64) -> SessionId {
    db.create_session(CreateSessionRow {
        org_id,
        owner_principal_id: PrincipalId::from_seed(1),
        title: Some("tasks".to_string()),
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

/// A task whose spec carries a push-config secret, which public reads redact.
fn new_task(session_id: SessionId, id: &str) -> Value {
    serde_json::to_value(CreateSessionTask {
        session_id,
        id: Some(id.to_string()),
        kind: "background_tool".to_string(),
        display_name: "Build".to_string(),
        spec: json!({
            "tool": "build",
            "push_configs": [{ "url": "https://hooks.example.com", "secret": "s3cret" }],
        }),
        state: everruns_core::session_task::SessionTaskState::Queued,
        links: TaskLinks::default(),
        wake_policy: TaskWakePolicy::Silent,
    })
    .unwrap()
}

/// Params naming one task: every command but create and list accepts these
/// (the extra fields are ignored where they do not apply).
fn task_params(session_id: SessionId, task_id: &str) -> Value {
    json!({
        "session_id": session_id.to_string(),
        "task_id": task_id,
        "task": new_task(session_id, task_id),
        "update": { "state": "running" },
        "message": { "direction": "outbound", "content": [{ "type": "text", "text": "hi" }] },
    })
}

#[tokio::test]
async fn the_worker_task_registry_round_trips_through_dispatch() {
    let db = Arc::new(StorageBackend::test_database());
    let session_id = session(&db, DEFAULT_ORG_ID).await;
    let ctx = worker_ctx(db, DEFAULT_ORG_ID);
    let id = "task_worker_round_trip";

    let created = run(
        &ctx,
        "worker_create_session_task",
        json!({ "session_id": session_id.to_string(), "task": new_task(session_id, id) }),
    )
    .await
    .expect("create");
    assert_eq!(created["id"], id);
    assert_eq!(created["state"], "queued");
    // The worker gets the spec unredacted: executors deliver with the secret.
    assert_eq!(created["spec"]["push_configs"][0]["secret"], "s3cret");

    let updated = run(
        &ctx,
        "worker_update_session_task",
        task_params(session_id, id),
    )
    .await
    .expect("update");
    assert_eq!(updated["state"], "running");
    assert!(updated["started_at"].is_string(), "invariants applied");

    let read = run(&ctx, "worker_get_session_task", task_params(session_id, id))
        .await
        .expect("get");
    assert_eq!(read["state"], "running");
    assert_eq!(read["spec"]["push_configs"][0]["secret"], "s3cret");

    let listed = run(
        &ctx,
        "worker_list_session_tasks",
        json!({ "session_id": session_id.to_string(), "state": "running" }),
    )
    .await
    .expect("list");
    assert_eq!(listed.as_array().unwrap().len(), 1);
    let none = run(
        &ctx,
        "worker_list_session_tasks",
        json!({ "session_id": session_id.to_string(), "kind": "subagent" }),
    )
    .await
    .expect("list by kind");
    assert!(none.as_array().unwrap().is_empty());

    let first = run(
        &ctx,
        "worker_record_session_task_message",
        task_params(session_id, id),
    )
    .await
    .expect("record");
    assert_eq!(first["direction"], "outbound");
    run(
        &ctx,
        "worker_record_session_task_message",
        task_params(session_id, id),
    )
    .await
    .expect("record again");
    let messages = run(
        &ctx,
        "worker_list_session_task_messages",
        task_params(session_id, id),
    )
    .await
    .expect("list messages");
    assert_eq!(messages.as_array().unwrap().len(), 2);
    let newer = run(
        &ctx,
        "worker_list_session_task_messages",
        json!({ "session_id": session_id.to_string(), "task_id": id, "after_id": first["id"] }),
    )
    .await
    .expect("list messages after");
    assert_eq!(newer.as_array().unwrap().len(), 1, "after_id is a cursor");

    let canceled = run(
        &ctx,
        "worker_request_cancel_session_task",
        task_params(session_id, id),
    )
    .await
    .expect("cancel");
    assert!(canceled["cancel_requested_at"].is_string());

    let missing = run(
        &ctx,
        "worker_get_session_task",
        task_params(session_id, "task_absent"),
    )
    .await
    .expect("get an absent task");
    assert!(missing.is_null());
}

#[tokio::test]
async fn a_task_for_another_session_is_refused() {
    let db = Arc::new(StorageBackend::test_database());
    let session_id = session(&db, DEFAULT_ORG_ID).await;
    let other = session(&db, DEFAULT_ORG_ID).await;
    let ctx = worker_ctx(db, DEFAULT_ORG_ID);

    let error = run(
        &ctx,
        "worker_create_session_task",
        json!({ "session_id": session_id.to_string(), "task": new_task(other, "task_x") }),
    )
    .await
    .expect_err("mismatched session");
    assert!(
        matches!(error.kind, CommandErrorKind::BadRequest(_)),
        "{error:?}"
    );
}

/// THREAT[TM-TENANT-001]: a worker acting for one org cannot create, read,
/// update, cancel, list, or message another org's session tasks, whether it
/// names the foreign session or names the foreign task under a session of its
/// own. The RPCs these replace took the session alone.
#[tokio::test]
async fn another_orgs_session_and_task_are_not_found() {
    let db = Arc::new(StorageBackend::test_database());
    let foreign = session(&db, DEFAULT_ORG_ID).await;
    let owner_ctx = worker_ctx(db.clone(), DEFAULT_ORG_ID);
    let id = "task_foreign";
    run(
        &owner_ctx,
        "worker_create_session_task",
        json!({ "session_id": foreign.to_string(), "task": new_task(foreign, id) }),
    )
    .await
    .expect("create in the owning org");

    let other_org = DEFAULT_ORG_ID + 1;
    let own = session(&db, other_org).await;
    let ctx = worker_ctx(db, other_org);

    for name in COMMANDS {
        let error = run(&ctx, name, task_params(foreign, id))
            .await
            .expect_err(name);
        assert!(
            matches!(error.kind, CommandErrorKind::NotFound(_)),
            "{name}: {error:?}"
        );
    }

    // The foreign task id under the worker's own session finds nothing.
    for name in [
        "worker_get_session_task",
        "worker_update_session_task",
        "worker_request_cancel_session_task",
    ] {
        let answer = run(&ctx, name, task_params(own, id)).await.expect(name);
        assert!(answer.is_null(), "{name}: {answer}");
    }
    for name in [
        "worker_record_session_task_message",
        "worker_list_session_task_messages",
    ] {
        let error = run(&ctx, name, task_params(own, id)).await.expect_err(name);
        assert!(
            matches!(error.kind, CommandErrorKind::NotFound(_)),
            "{name}: {error:?}"
        );
    }

    let still = run(
        &owner_ctx,
        "worker_get_session_task",
        task_params(foreign, id),
    )
    .await
    .unwrap();
    assert_eq!(
        still["state"], "queued",
        "the foreign worker changed nothing"
    );
    assert!(still.get("cancel_requested_at").is_none());
    let messages = run(
        &owner_ctx,
        "worker_list_session_task_messages",
        task_params(foreign, id),
    )
    .await
    .unwrap();
    assert!(messages.as_array().unwrap().is_empty());
}

/// THREAT[TM-AUTHZ-002]: the worker task registry is not a person's API. An
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
        let error = run(&ctx, name, task_params(session_id, "task_a"))
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
/// task commands stay where they were.
#[test]
fn internal_commands_are_on_no_public_surface() {
    let flags = all_feature_flags_for_test();
    let discovered: Vec<&str> = catalog_entries_with_schemas(false, &flags)
        .into_iter()
        .map(|entry| entry.name)
        .chain(catalog_entries().into_iter().map(|meta| meta.name))
        .collect();
    let contracts = crate::services::command_catalog::cli_tree::contracts();
    assert!(discovered.contains(&"list_session_tasks"));
    assert!(discovered.contains(&"cancel_session_task"));

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

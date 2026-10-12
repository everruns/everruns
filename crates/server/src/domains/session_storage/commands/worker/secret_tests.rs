use crate::domains::change_history::Change;
use crate::domains::common::{Ctx, dispatch, *};
use crate::storage::{CreateSessionRow, EncryptionService, StorageBackend};
use everruns_contracts::typed_id::{PrincipalId, SessionId};
use everruns_core::organization::OrgRole;
use everruns_core::{Caller, DEFAULT_ORG_ID};
use serde_json::{Value, json};
use std::sync::Arc;

const COMMANDS: [&str; 4] = [
    "worker_set_session_secret",
    "worker_get_session_secret",
    "worker_delete_session_secret",
    "worker_list_session_secrets",
];

const PLAINTEXT: &str = "pl41ntext-secret-value";

async fn session(db: &Arc<StorageBackend>, org_id: i64) -> SessionId {
    db.create_session(CreateSessionRow {
        org_id,
        owner_principal_id: PrincipalId::from_seed(1),
        title: Some("secrets".to_string()),
        ..Default::default()
    })
    .await
    .unwrap()
    .id
}

fn encryption() -> Arc<EncryptionService> {
    Arc::new(
        EncryptionService::new("kek-v1:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=", &[]).unwrap(),
    )
}

fn worker_ctx(db: Arc<StorageBackend>, org_id: i64) -> Ctx {
    Ctx::minimal_for_test(Caller::internal(org_id), db, Some(encryption()))
}

async fn run(ctx: &Ctx, name: &str, params: Value) -> Result<Value, CommandError> {
    dispatch(name, params, ctx)
        .await
        .map(|json| serde_json::from_str(&json).unwrap())
}

fn secret(session_id: SessionId, name: &str) -> Value {
    json!({ "session_id": session_id.to_string(), "name": name, "value": PLAINTEXT })
}

#[tokio::test]
async fn the_worker_secret_store_round_trips_through_dispatch() {
    let db = Arc::new(StorageBackend::test_database());
    let session_id = session(&db, DEFAULT_ORG_ID).await;
    let ctx = worker_ctx(db.clone(), DEFAULT_ORG_ID);

    run(
        &ctx,
        "worker_set_session_secret",
        secret(session_id, "TOKEN"),
    )
    .await
    .expect("set");
    // Internal names are the runtime's own; the worker reads and lists them.
    let internal = "__sandbox_state";
    run(
        &ctx,
        "worker_set_session_secret",
        secret(session_id, internal),
    )
    .await
    .expect("set internal");

    let stored = db
        .get_session_secret(session_id.uuid(), "TOKEN")
        .await
        .unwrap()
        .unwrap();
    assert!(
        !String::from_utf8_lossy(&stored.value_encrypted).contains(PLAINTEXT),
        "stored encrypted"
    );
    let read = run(
        &ctx,
        "worker_get_session_secret",
        secret(session_id, "TOKEN"),
    )
    .await
    .expect("get");
    assert_eq!(read, json!(PLAINTEXT));
    let absent = run(
        &ctx,
        "worker_get_session_secret",
        secret(session_id, "NOPE"),
    )
    .await
    .expect("get absent");
    assert!(absent.is_null());

    let listed = run(
        &ctx,
        "worker_list_session_secrets",
        json!({ "session_id": session_id.to_string() }),
    )
    .await
    .expect("list");
    let mut names: Vec<&str> = listed
        .as_array()
        .unwrap()
        .iter()
        .map(|secret| secret["name"].as_str().unwrap())
        .collect();
    names.sort();
    assert_eq!(names, ["TOKEN", internal]);
    assert!(
        listed[0].get("value").is_none(),
        "listing carries no values"
    );

    let deleted = run(
        &ctx,
        "worker_delete_session_secret",
        secret(session_id, "TOKEN"),
    )
    .await
    .expect("delete");
    assert_eq!(deleted, json!(true));
    let again = run(
        &ctx,
        "worker_delete_session_secret",
        secret(session_id, "TOKEN"),
    )
    .await
    .expect("delete absent");
    assert_eq!(again, json!(false));
}

#[tokio::test]
async fn secrets_need_the_deployment_key() {
    let db = Arc::new(StorageBackend::test_database());
    let session_id = session(&db, DEFAULT_ORG_ID).await;
    let ctx = Ctx::minimal_for_test(Caller::internal(DEFAULT_ORG_ID), db, None);
    let error = run(
        &ctx,
        "worker_set_session_secret",
        secret(session_id, "TOKEN"),
    )
    .await
    .expect_err("no key");
    assert!(
        matches!(error.kind, CommandErrorKind::BadRequest(_)),
        "{error:?}"
    );
}

/// THREAT[TM-TENANT-001]: a worker acting for one org cannot write, read,
/// delete, or list another org's session secrets by naming the session. The
/// RPCs these replace took the session alone.
#[tokio::test]
async fn another_orgs_session_is_not_found() {
    let db = Arc::new(StorageBackend::test_database());
    let foreign = session(&db, DEFAULT_ORG_ID).await;
    let owner_ctx = worker_ctx(db.clone(), DEFAULT_ORG_ID);
    run(
        &owner_ctx,
        "worker_set_session_secret",
        secret(foreign, "TOKEN"),
    )
    .await
    .expect("set in the owning org");
    let ctx = worker_ctx(db, DEFAULT_ORG_ID + 1);

    for name in COMMANDS {
        let error = run(&ctx, name, secret(foreign, "TOKEN"))
            .await
            .expect_err(name);
        assert!(
            matches!(error.kind, CommandErrorKind::NotFound(_)),
            "{name}: {error:?}"
        );
    }

    let still = run(
        &owner_ctx,
        "worker_get_session_secret",
        secret(foreign, "TOKEN"),
    )
    .await
    .unwrap();
    assert_eq!(
        still,
        json!(PLAINTEXT),
        "the foreign worker changed nothing"
    );
}

/// THREAT[TM-AUTHZ-002]: the worker secret store is not a person's API. An
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
    let ctx = Ctx::minimal_for_test(owner, db, Some(encryption()));

    for name in COMMANDS {
        let error = run(&ctx, name, secret(session_id, "TOKEN"))
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
/// `list_session_secrets` stays where it was.
#[test]
fn internal_commands_are_on_no_public_surface() {
    let flags = all_feature_flags_for_test();
    let discovered: Vec<&str> = catalog_entries_with_schemas(false, &flags)
        .into_iter()
        .map(|entry| entry.name)
        .chain(catalog_entries().into_iter().map(|meta| meta.name))
        .collect();
    let contracts = crate::services::command_catalog::cli_tree::contracts();
    assert!(discovered.contains(&"list_session_secrets"));

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

/// THREAT[TM-AUTHZ-023]: a secret's plaintext is never logged or kept in
/// entity history. Every command run here, failures included, goes through
/// `Command::run` with a TRACE subscriber capturing spans and events; none of
/// them may print the value. Only `Change::Subject` captures params, and no
/// secret command declares one.
#[tokio::test]
async fn secret_values_stay_out_of_logs_and_history() {
    use std::sync::Mutex;
    use tracing::instrument::WithSubscriber;

    #[derive(Clone, Default)]
    struct Sink(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for Sink {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let sink = Sink::default();
    let writer = sink.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_span_events(tracing_subscriber::fmt::format::FmtSpan::FULL)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    let db = Arc::new(StorageBackend::test_database());
    let session_id = session(&db, DEFAULT_ORG_ID).await;
    let exercise = async {
        let ctx = worker_ctx(db.clone(), DEFAULT_ORG_ID);
        run(
            &ctx,
            "worker_set_session_secret",
            secret(session_id, "TOKEN"),
        )
        .await
        .unwrap();
        run(
            &ctx,
            "worker_get_session_secret",
            secret(session_id, "TOKEN"),
        )
        .await
        .unwrap();
        // A failure is logged; it must not carry the value either.
        let keyless = Ctx::minimal_for_test(Caller::internal(DEFAULT_ORG_ID), db.clone(), None);
        run(
            &keyless,
            "worker_get_session_secret",
            secret(session_id, "TOKEN"),
        )
        .await
        .unwrap_err();
        run(
            &keyless,
            "worker_set_session_secret",
            secret(session_id, "TOKEN"),
        )
        .await
        .unwrap_err();
    };
    // While a single dispatcher is registered, tracing-core caches a new
    // callsite's interest from whichever thread registers it first, so a
    // parallel test touching the same commands could cache them as disabled
    // for this capture. A second live dispatcher makes registration consult
    // every registered one.
    let _second = tracing::Dispatch::new(tracing_subscriber::registry());
    exercise.with_subscriber(subscriber).await;

    let logged = String::from_utf8(sink.0.lock().unwrap().clone()).unwrap();
    assert!(
        logged.contains("worker_set_session_secret"),
        "the capture saw the commands"
    );
    assert!(!logged.contains(PLAINTEXT), "a secret value was logged");

    for name in COMMANDS {
        let desc = inventory::iter::<CommandDescriptor>
            .into_iter()
            .find(|desc| (desc.meta)().name == name)
            .unwrap();
        assert!(
            !matches!((desc.change)(), Change::Subject { .. }),
            "{name} would record its params in entity history"
        );
    }
    let debug = format!(
        "{:?}",
        super::WorkerSetSessionSecret {
            session_id: session_id.to_string(),
            name: "TOKEN".into(),
            value: PLAINTEXT.into(),
        }
    );
    assert!(!debug.contains(PLAINTEXT), "{debug}");
}

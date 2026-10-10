use crate::domains::change_history::Change;
use crate::domains::common::{Ctx, dispatch, *};
use crate::storage::{CreateSessionRow, CreateVirtualUserRow, StorageBackend};
use everruns_contracts::error::Result as CoreResult;
use everruns_contracts::runtime::ServiceApiKeyConnection;
use everruns_contracts::session_sandbox::SessionSandboxCredential;
use everruns_contracts::typed_id::{PrincipalId, SessionId, VirtualUserId};
use everruns_core::connection_services::UserConnectionResolver;
use everruns_core::organization::OrgRole;
use everruns_core::{Caller, DEFAULT_ORG_ID};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

const SESSION_COMMANDS: [&str; 4] = [
    "worker_get_connection_token",
    "worker_get_sandbox_connection_token",
    "worker_get_service_api_key_connection",
    "worker_get_connection_user",
];
const VIRTUAL_USER_COMMANDS: [&str; 2] = [
    "worker_get_virtual_user_connection_token",
    "worker_get_connection_token_for_connection",
];

const TOKEN: &str = "ghs_l34ked-if-logged";

/// Answers every lookup and records which lookups ran and how they were bound.
#[derive(Clone, Default)]
struct Recording {
    calls: Arc<Mutex<Vec<String>>>,
    bound: Option<Uuid>,
    owner: Option<Uuid>,
}

impl Recording {
    fn note(&self, call: &str) {
        let bound = self.bound.map(|id| format!("@{id}")).unwrap_or_default();
        self.calls.lock().unwrap().push(format!("{call}{bound}"));
    }
}

#[async_trait::async_trait]
impl UserConnectionResolver for Recording {
    fn for_execution(&self, id: Uuid) -> Option<Arc<dyn UserConnectionResolver>> {
        Some(Arc::new(Self {
            bound: Some(id),
            ..self.clone()
        }))
    }
    async fn get_connection_token(&self, _: SessionId, _: &str) -> CoreResult<Option<String>> {
        self.note("token");
        Ok(Some(TOKEN.to_string()))
    }
    async fn get_sandbox_connection_token(
        &self,
        _: SessionId,
        _: &str,
        _: &SessionSandboxCredential,
    ) -> CoreResult<Option<String>> {
        self.note("sandbox");
        Ok(Some(TOKEN.to_string()))
    }
    async fn get_service_api_key_connection(
        &self,
        _: SessionId,
        _: &str,
    ) -> CoreResult<Option<ServiceApiKeyConnection>> {
        self.note("service");
        Ok(Some(ServiceApiKeyConnection {
            api_key: TOKEN.to_string(),
            metadata: Some(json!({ "region": "eu" })),
        }))
    }
    async fn get_connection_user(&self, _: SessionId, _: &str) -> CoreResult<Option<Uuid>> {
        self.note("user");
        Ok(self.owner)
    }
    async fn get_connection_token_for_user(&self, _: Uuid, _: &str) -> CoreResult<Option<String>> {
        self.note("for_user");
        Ok(Some(TOKEN.to_string()))
    }
    async fn get_connection_token_for_connection(
        &self,
        _: Uuid,
        _: Uuid,
        _: &str,
    ) -> CoreResult<Option<String>> {
        self.note("for_connection");
        Ok(Some(TOKEN.to_string()))
    }
}

async fn session(db: &Arc<StorageBackend>, org_id: i64) -> SessionId {
    db.create_session(CreateSessionRow {
        org_id,
        owner_principal_id: PrincipalId::from_seed(1),
        title: Some("connections".to_string()),
        ..Default::default()
    })
    .await
    .unwrap()
    .id
}

async fn virtual_user(db: &Arc<StorageBackend>, org_id: i64) -> Uuid {
    let id = VirtualUserId::new();
    db.create_virtual_user(CreateVirtualUserRow {
        usage: "service".to_string(),
        org_id,
        id,
        name: "cleanup owner".to_string(),
        description: None,
        avatar_url: None,
        locale: None,
        timezone: None,
    })
    .await
    .unwrap();
    id.uuid()
}

fn worker_ctx(db: Arc<StorageBackend>, org_id: i64, resolver: &Recording) -> Ctx {
    Ctx::minimal_for_test(Caller::internal(org_id), db, None)
        .with_connection_resolver(Some(Arc::new(resolver.clone())))
}

async fn run(ctx: &Ctx, name: &str, params: Value) -> Result<Value, CommandError> {
    dispatch(name, params, ctx)
        .await
        .map(|json| serde_json::from_str(&json).unwrap())
}

fn session_params(session_id: SessionId, input: Option<Uuid>) -> Value {
    json!({
        "session_id": session_id.to_string(),
        "provider": "daytona",
        "input_message_id": input,
        "credential": SessionSandboxCredential::default(),
    })
}

fn virtual_user_params(owner: Uuid) -> Value {
    json!({ "virtual_user_id": owner, "connection_id": Uuid::new_v4(), "provider": "daytona" })
}

#[tokio::test]
async fn lookups_run_through_the_servers_resolver() {
    let db = Arc::new(StorageBackend::test_database());
    let session_id = session(&db, DEFAULT_ORG_ID).await;
    let owner = virtual_user(&db, DEFAULT_ORG_ID).await;
    let resolver = Recording {
        owner: Some(owner),
        ..Default::default()
    };
    let ctx = worker_ctx(db, DEFAULT_ORG_ID, &resolver);
    let invocation = Uuid::new_v4();

    let token = run(
        &ctx,
        "worker_get_connection_token",
        session_params(session_id, Some(invocation)),
    )
    .await
    .unwrap();
    assert_eq!(token, json!(TOKEN));
    let sandbox = run(
        &ctx,
        "worker_get_sandbox_connection_token",
        session_params(session_id, None),
    )
    .await
    .unwrap();
    assert_eq!(sandbox, json!(TOKEN));
    let service = run(
        &ctx,
        "worker_get_service_api_key_connection",
        session_params(session_id, Some(invocation)),
    )
    .await
    .unwrap();
    assert_eq!(service["api_key"], TOKEN);
    assert_eq!(service["metadata"]["region"], "eu");
    let user = run(
        &ctx,
        "worker_get_connection_user",
        session_params(session_id, None),
    )
    .await
    .unwrap();
    assert_eq!(user, json!(owner));
    for name in VIRTUAL_USER_COMMANDS {
        assert_eq!(
            run(&ctx, name, virtual_user_params(owner)).await.unwrap(),
            json!(TOKEN)
        );
    }

    assert_eq!(
        *resolver.calls.lock().unwrap(),
        [
            format!("token@{invocation}"),
            "sandbox".to_string(),
            format!("service@{invocation}"),
            "user".to_string(),
            "for_user".to_string(),
            "for_connection".to_string(),
        ],
        "each lookup bound to the invocation the worker named, and only then"
    );
}

/// MCP grants are attachment-scoped; the plain lookup refuses their providers
/// before it reads anything.
#[tokio::test]
async fn the_plain_lookup_refuses_mcp_grants() {
    let db = Arc::new(StorageBackend::test_database());
    let session_id = session(&db, DEFAULT_ORG_ID).await;
    let resolver = Recording::default();
    let ctx = worker_ctx(db, DEFAULT_ORG_ID, &resolver);
    let error = run(
        &ctx,
        "worker_get_connection_token",
        json!({ "session_id": session_id.to_string(), "provider": "mcp_oauth_123" }),
    )
    .await
    .expect_err("refused");
    assert!(
        matches!(error.kind, CommandErrorKind::Forbidden(_)),
        "{error:?}"
    );
    assert!(resolver.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn without_a_resolver_lookups_are_unavailable() {
    let db = Arc::new(StorageBackend::test_database());
    let session_id = session(&db, DEFAULT_ORG_ID).await;
    let ctx = Ctx::minimal_for_test(Caller::internal(DEFAULT_ORG_ID), db, None);
    let error = run(
        &ctx,
        "worker_get_connection_token",
        session_params(session_id, None),
    )
    .await
    .expect_err("unavailable");
    assert!(
        matches!(error.kind, CommandErrorKind::Unavailable(_)),
        "{error:?}"
    );
}

/// THREAT[TM-TENANT-001]: a worker acting for one org cannot read another
/// org's connection tokens by naming its session or virtual user. The RPCs
/// these replace took the session or virtual user alone.
#[tokio::test]
async fn another_orgs_session_or_virtual_user_is_not_found() {
    let db = Arc::new(StorageBackend::test_database());
    let foreign_session = session(&db, DEFAULT_ORG_ID).await;
    let foreign_owner = virtual_user(&db, DEFAULT_ORG_ID).await;
    let resolver = Recording::default();
    let ctx = worker_ctx(db, DEFAULT_ORG_ID + 1, &resolver);

    for name in SESSION_COMMANDS {
        let error = run(&ctx, name, session_params(foreign_session, None))
            .await
            .expect_err(name);
        assert!(
            matches!(error.kind, CommandErrorKind::NotFound(_)),
            "{name}: {error:?}"
        );
    }
    for name in VIRTUAL_USER_COMMANDS {
        let error = run(&ctx, name, virtual_user_params(foreign_owner))
            .await
            .expect_err(name);
        assert!(
            matches!(error.kind, CommandErrorKind::NotFound(_)),
            "{name}: {error:?}"
        );
    }
    assert!(
        resolver.calls.lock().unwrap().is_empty(),
        "no grant was read for a foreign org"
    );
}

/// THREAT[TM-AUTHZ-002]: connection tokens are not a person's API. An owner,
/// who passes every session and virtual-user policy, is still refused.
#[tokio::test]
async fn a_person_cannot_run_internal_commands() {
    let db = Arc::new(StorageBackend::test_database());
    let session_id = session(&db, DEFAULT_ORG_ID).await;
    let owner_id = virtual_user(&db, DEFAULT_ORG_ID).await;
    let owner = Caller {
        is_internal: false,
        user_id: Some(Uuid::nil()),
        role: OrgRole::Owner,
        ..Caller::internal(DEFAULT_ORG_ID)
    };
    let resolver = Recording::default();
    let ctx = Ctx::minimal_for_test(owner, db, None)
        .with_connection_resolver(Some(Arc::new(resolver.clone())));

    for name in SESSION_COMMANDS {
        let error = run(&ctx, name, session_params(session_id, None))
            .await
            .expect_err(name);
        assert!(
            matches!(error.kind, CommandErrorKind::Forbidden(_)),
            "{name}: {error:?}"
        );
    }
    for name in VIRTUAL_USER_COMMANDS {
        let error = run(&ctx, name, virtual_user_params(owner_id))
            .await
            .expect_err(name);
        assert!(
            matches!(error.kind, CommandErrorKind::Forbidden(_)),
            "{name}: {error:?}"
        );
    }
}

/// Internal commands stay off every public surface: discovery, the scripted
/// toolset (MCP, Platform, `/v1/commands`), and the command tree. They are
/// reads, so entity history (which captures a `Subject` change's params)
/// never sees a token.
#[test]
fn internal_commands_are_on_no_public_surface_and_record_nothing() {
    let flags = all_feature_flags_for_test();
    let discovered: Vec<&str> = catalog_entries_with_schemas(false, &flags)
        .into_iter()
        .map(|entry| entry.name)
        .chain(catalog_entries().into_iter().map(|meta| meta.name))
        .collect();
    let contracts = crate::services::command_catalog::cli_tree::contracts();
    assert!(discovered.contains(&"list_user_connections"));

    for name in SESSION_COMMANDS.into_iter().chain(VIRTUAL_USER_COMMANDS) {
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
        assert!((desc.read_only)(), "{name} is a read");
        assert!(matches!((desc.change)(), Change::None), "{name}");
    }
}

/// THREAT[TM-AUTHZ-023]: a token answer is never logged. Every lookup here,
/// a failure included, runs under a TRACE subscriber capturing spans and
/// events, and none of them prints the token.
#[tokio::test]
async fn tokens_stay_out_of_logs() {
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
    let owner = virtual_user(&db, DEFAULT_ORG_ID).await;
    let resolver = Recording::default();
    let ctx = worker_ctx(db, DEFAULT_ORG_ID, &resolver);
    let exercise = async {
        for name in SESSION_COMMANDS {
            run(&ctx, name, session_params(session_id, None))
                .await
                .unwrap();
        }
        for name in VIRTUAL_USER_COMMANDS {
            run(&ctx, name, virtual_user_params(owner)).await.unwrap();
        }
        run(
            &ctx,
            "worker_get_connection_token",
            json!({ "session_id": "nope" }),
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
        logged.contains("worker_get_connection_token"),
        "the capture saw the commands"
    );
    assert!(!logged.contains(TOKEN), "a token was logged");
}

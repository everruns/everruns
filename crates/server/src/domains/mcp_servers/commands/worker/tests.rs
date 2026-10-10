use crate::domains::change_history::Change;
use crate::domains::common::{Ctx, dispatch, *};
use crate::storage::{CreateHarnessRow, CreateMcpServerRow, CreateSessionRow, StorageBackend};
use everruns_contracts::error::Result as CoreResult;
use everruns_contracts::typed_id::{PrincipalId, SessionId};
use everruns_core::connection_services::UserConnectionResolver;
use everruns_core::organization::OrgRole;
use everruns_core::{Caller, DEFAULT_ORG_ID, McpServerActsAs};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

const COMMANDS: [&str; 2] = [
    "worker_get_mcp_connection_token",
    "worker_invalidate_mcp_connection",
];

/// Records each grant read or invalidation it is asked for.
#[derive(Clone, Default)]
struct Recording(Arc<Mutex<Vec<(String, McpServerActsAs)>>>);

#[async_trait::async_trait]
impl UserConnectionResolver for Recording {
    async fn get_connection_token(&self, _: SessionId, _: &str) -> CoreResult<Option<String>> {
        Ok(None)
    }
    async fn get_mcp_connection_token(
        &self,
        _: SessionId,
        _: &str,
        acts_as: McpServerActsAs,
    ) -> CoreResult<Option<String>> {
        self.0.lock().unwrap().push(("token".into(), acts_as));
        Ok(Some("mcp-grant".into()))
    }
    async fn invalidate_mcp_connection(
        &self,
        _: SessionId,
        _: &str,
        acts_as: McpServerActsAs,
        _: &str,
    ) -> CoreResult<()> {
        self.0.lock().unwrap().push(("invalidate".into(), acts_as));
        Ok(())
    }
}

/// A session with an org OAuth MCP server named `docs` and one unattended
/// invocation on it.
struct Fixture {
    db: Arc<StorageBackend>,
    session_id: SessionId,
    provider: String,
    invocation: Uuid,
}

async fn fixture() -> Fixture {
    let db = Arc::new(StorageBackend::test_database());
    // Session scope resolution starts from the org's built-in base harness.
    db.create_harness(
        DEFAULT_ORG_ID,
        CreateHarnessRow {
            name: "base".to_string(),
            display_name: Some("Base".to_string()),
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
    let session_id = db
        .create_session(CreateSessionRow {
            org_id: DEFAULT_ORG_ID,
            owner_principal_id: PrincipalId::from_seed(1),
            title: Some("mcp".to_string()),
            ..Default::default()
        })
        .await
        .unwrap()
        .id;
    let server = db
        .create_mcp_server(
            DEFAULT_ORG_ID,
            CreateMcpServerRow {
                name: "docs".to_string(),
                description: None,
                url: "https://docs.example/mcp".to_string(),
                transport_type: "http".to_string(),
                api_key_encrypted: None,
                headers: None,
                settings: Some(json!({ "auth_mode": "oauth" })),
            },
        )
        .await
        .unwrap();
    let invocation = Uuid::now_v7();
    db.record_runtime_invocation(DEFAULT_ORG_ID, session_id, invocation, None, None, None)
        .await
        .unwrap();
    Fixture {
        db,
        session_id,
        provider: everruns_core::mcp_oauth_provider_id_for_uuid(server.id.uuid()),
        invocation,
    }
}

impl Fixture {
    fn params(&self, acts_as: &str) -> Value {
        json!({
            "session_id": self.session_id.to_string(),
            "server_prefix": "docs",
            "input_message_id": self.invocation,
            "provider": self.provider,
            "acts_as": acts_as,
            "rejected_credential_fingerprint": "fp",
        })
    }

    fn ctx(&self, caller: Caller, resolver: &Recording) -> Ctx {
        Ctx::minimal_for_test(caller, self.db.clone(), None)
            .with_connection_resolver(Some(Arc::new(resolver.clone())))
    }
}

async fn run(ctx: &Ctx, name: &str, params: Value) -> Result<Value, CommandError> {
    dispatch(name, params, ctx)
        .await
        .map(|json| serde_json::from_str(&json).unwrap())
}

/// An org server resolves with `actsAs: none`; asking for exactly that reads
/// the grant, and invalidation reaches the resolver.
#[tokio::test]
async fn the_configured_attachment_reads_its_grant() {
    let fixture = fixture().await;
    let resolver = Recording::default();
    let ctx = fixture.ctx(Caller::internal(DEFAULT_ORG_ID), &resolver);

    let token = run(
        &ctx,
        "worker_get_mcp_connection_token",
        fixture.params("none"),
    )
    .await
    .expect("token");
    assert_eq!(token, json!("mcp-grant"));
    run(
        &ctx,
        "worker_invalidate_mcp_connection",
        fixture.params("none"),
    )
    .await
    .expect("invalidate");
    assert_eq!(
        *resolver.0.lock().unwrap(),
        [
            ("token".to_string(), McpServerActsAs::None),
            ("invalidate".to_string(), McpServerActsAs::None),
        ]
    );
}

/// THREAT[TM-TOOL-041]: a worker-supplied `acts_as`, provider, prefix or
/// invocation that does not match the configured attachment reads no grant.
#[tokio::test]
async fn a_request_that_does_not_match_the_attachment_reads_nothing() {
    let fixture = fixture().await;
    let resolver = Recording::default();
    let ctx = fixture.ctx(Caller::internal(DEFAULT_ORG_ID), &resolver);

    let mut cases = Vec::new();
    cases.push(("another identity", fixture.params("user")));
    let mut wrong_provider = fixture.params("none");
    wrong_provider["provider"] = json!("mcp_oauth_00000000-0000-0000-0000-000000000000");
    cases.push(("another provider", wrong_provider));
    let mut no_invocation = fixture.params("none");
    no_invocation["input_message_id"] = Value::Null;
    cases.push(("no invocation", no_invocation));
    let mut no_prefix = fixture.params("none");
    no_prefix["server_prefix"] = Value::Null;
    cases.push(("no attachment", no_prefix));
    let mut unknown_invocation = fixture.params("none");
    unknown_invocation["input_message_id"] = json!(Uuid::now_v7());
    cases.push(("unknown invocation", unknown_invocation));
    let mut unknown_prefix = fixture.params("none");
    unknown_prefix["server_prefix"] = json!("elsewhere");
    cases.push(("unknown attachment", unknown_prefix));

    for name in COMMANDS {
        for (case, params) in &cases {
            let error = run(&ctx, name, params.clone()).await.expect_err(case);
            assert!(
                matches!(error.kind, CommandErrorKind::Forbidden(_)),
                "{name} {case}: {error:?}"
            );
        }
        let error = run(&ctx, name, fixture.params("system"))
            .await
            .expect_err("invalid acts_as");
        assert!(
            matches!(error.kind, CommandErrorKind::BadRequest(_)),
            "{error:?}"
        );
    }
    assert!(
        resolver.0.lock().unwrap().is_empty(),
        "no grant was touched"
    );
}

/// THREAT[TM-TENANT-001]: a worker acting for another org cannot reach this
/// session's MCP grants. The RPCs found the org from the session itself.
#[tokio::test]
async fn another_orgs_session_is_not_found() {
    let fixture = fixture().await;
    let resolver = Recording::default();
    let ctx = fixture.ctx(Caller::internal(DEFAULT_ORG_ID + 1), &resolver);
    for name in COMMANDS {
        let error = run(&ctx, name, fixture.params("none"))
            .await
            .expect_err(name);
        assert!(
            matches!(error.kind, CommandErrorKind::NotFound(_)),
            "{name}: {error:?}"
        );
    }
    assert!(resolver.0.lock().unwrap().is_empty());
}

/// THREAT[TM-AUTHZ-002]: MCP grants are not a person's API.
#[tokio::test]
async fn a_person_cannot_run_internal_commands() {
    let fixture = fixture().await;
    let resolver = Recording::default();
    let owner = Caller {
        is_internal: false,
        user_id: Some(Uuid::nil()),
        role: OrgRole::Owner,
        ..Caller::internal(DEFAULT_ORG_ID)
    };
    let ctx = fixture.ctx(owner, &resolver);
    for name in COMMANDS {
        let error = run(&ctx, name, fixture.params("none"))
            .await
            .expect_err(name);
        assert!(
            matches!(error.kind, CommandErrorKind::Forbidden(_)),
            "{name}: {error:?}"
        );
    }
}

/// Internal commands stay off every public surface, and neither records its
/// params (which name a credential fingerprint) in entity history.
#[test]
fn internal_commands_are_on_no_public_surface_and_record_nothing() {
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
        assert!(
            !matches!((desc.change)(), Change::Subject { .. }),
            "{name} would record its params"
        );
    }
}

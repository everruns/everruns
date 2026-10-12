//! Bound secrets are write-only: stored with a binding through the public
//! commands, listed and resolved for the shell's HTTP layer, and refused to
//! every path that would hand the value to the model.

use crate::domains::common::{Ctx, dispatch, *};
use crate::storage::{
    AgentRow, CreateAgentRow, CreateSessionRow, EncryptionService, StorageBackend,
};
use everruns_contracts::typed_id::{AgentId, PrincipalId, SessionId};
use everruns_core::{Caller, DEFAULT_ORG_ID};
use serde_json::{Value, json};
use std::sync::Arc;

const KEY: &str = "jev-live-key-value";

fn encryption() -> Arc<EncryptionService> {
    Arc::new(
        EncryptionService::new("kek-v1:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=", &[]).unwrap(),
    )
}

fn ctx(db: &Arc<StorageBackend>) -> Ctx {
    Ctx::minimal_for_test(
        Caller::internal(DEFAULT_ORG_ID),
        db.clone(),
        Some(encryption()),
    )
}

async fn run(ctx: &Ctx, name: &str, params: Value) -> Result<Value, CommandError> {
    dispatch(name, params, ctx)
        .await
        .map(|json| serde_json::from_str(&json).unwrap())
}

async fn agent(db: &Arc<StorageBackend>) -> AgentRow {
    db.create_agent(
        DEFAULT_ORG_ID,
        CreateAgentRow {
            public_id: AgentId::new().to_string(),
            name: "jev-triage".into(),
            display_name: None,
            description: None,
            intro_markdown: None,
            short_description: None,
            starters: json!([]),
            system_prompt: String::new(),
            default_model_id: None,
            harness_id: everruns_contracts::typed_id::HarnessId::from_uuid(uuid::Uuid::nil()),
            tags: vec![],
            initial_files: json!([]),
            tools: json!([]),
            mcp_servers: json!({}),
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
            communication: Default::default(),
            environments: None,
            is_built_in: false,
        },
    )
    .await
    .unwrap()
}

async fn session(db: &Arc<StorageBackend>, agent: Option<&AgentRow>) -> SessionId {
    db.create_session(CreateSessionRow {
        org_id: DEFAULT_ORG_ID,
        owner_principal_id: PrincipalId::from_seed(1),
        title: Some("bound".to_string()),
        agent_id: agent.map(|a| a.id),
        ..Default::default()
    })
    .await
    .unwrap()
    .id
}

fn jev_binding() -> Value {
    json!({ "url": "https://api.typesafe.ai/", "auth": { "type": "bearer" } })
}

fn sid(session_id: SessionId) -> Value {
    json!({ "session_id": session_id.to_string() })
}

#[tokio::test]
async fn a_bound_session_secret_resolves_for_the_shell_and_never_reads_back() {
    let db = Arc::new(StorageBackend::test_database());
    let session_id = session(&db, None).await;
    let ctx = ctx(&db);

    run(
        &ctx,
        "batch_set_session_secrets",
        json!({
            "session_id": session_id.to_string(),
            "secrets": { "JEV_API_KEY": KEY, "PLAIN": "readable" },
            "bindings": { "JEV_API_KEY": jev_binding() },
        }),
    )
    .await
    .expect("set");

    // The public listing shows the binding, normalized, and no value.
    let listed = run(&ctx, "list_session_secrets", sid(session_id))
        .await
        .unwrap();
    let jev = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "JEV_API_KEY")
        .unwrap();
    assert_eq!(jev["binding"]["url"], "https://api.typesafe.ai");
    assert!(!listed.to_string().contains(KEY));

    // THREAT[TM-BASH-030]: the runtime cannot read or swap the bound value.
    let get = json!({ "session_id": session_id.to_string(), "name": "JEV_API_KEY" });
    let error = run(&ctx, "worker_get_session_secret", get.clone())
        .await
        .expect_err("bound get");
    assert!(error.to_string().contains("$JEV_API_KEY"), "{error}");
    let swap =
        json!({ "session_id": session_id.to_string(), "name": "JEV_API_KEY", "value": "attacker" });
    run(&ctx, "worker_set_session_secret", swap)
        .await
        .expect_err("bound set");
    // An ordinary secret still reads back.
    let plain = json!({ "session_id": session_id.to_string(), "name": "PLAIN" });
    assert_eq!(
        run(&ctx, "worker_get_session_secret", plain).await.unwrap(),
        json!("readable")
    );

    let infos = run(&ctx, "worker_list_bound_secrets", sid(session_id))
        .await
        .unwrap();
    assert_eq!(infos.as_array().unwrap().len(), 1);
    assert_eq!(infos[0]["scope"], "session");
    assert!(!infos.to_string().contains(KEY));

    let resolved = run(&ctx, "worker_resolve_bound_secrets", sid(session_id))
        .await
        .unwrap();
    assert_eq!(resolved[0]["name"], "JEV_API_KEY");
    assert_eq!(resolved[0]["value"], KEY);

    // Setting the name again without a binding makes it ordinary again: an
    // explicit act by someone who holds the value.
    run(
        &ctx,
        "batch_set_session_secrets",
        json!({ "session_id": session_id.to_string(), "secrets": { "JEV_API_KEY": "new" } }),
    )
    .await
    .unwrap();
    assert_eq!(
        run(&ctx, "worker_resolve_bound_secrets", sid(session_id))
            .await
            .unwrap(),
        json!([])
    );
}

/// Declare an HTTP credential on the agent (no value), as Platform Chat or
/// the Credentials tab's "Add binding" would, then store its value the way
/// the panel's value form does.
async fn agent_credential(ctx: &Ctx, agent: &AgentRow, env_name: &str, value: &str) -> Value {
    let declared = run(
        ctx,
        "create_agent_credential_binding",
        json!({
            "agent_id": agent.id.to_string(),
            "env_name": env_name,
            "url": "https://api.typesafe.ai/",
            "label": "Jev API key",
        }),
    )
    .await
    .expect("declare http credential");
    assert_eq!(declared["kind"], "http");
    assert_eq!(declared["configured"], false);
    let id: uuid::Uuid = declared["id"].as_str().unwrap().parse().unwrap();
    let encrypted = encryption().encrypt_string(value).unwrap();
    ctx.db
        .set_agent_mcp_secret_binding_value(DEFAULT_ORG_ID, agent.id, id, encrypted)
        .await
        .unwrap()
        .unwrap();
    declared
}

#[tokio::test]
async fn agent_http_credentials_reach_every_session_of_the_agent_and_sessions_shadow_them() {
    let db = Arc::new(StorageBackend::test_database());
    let agent = agent(&db).await;
    let first = session(&db, Some(&agent)).await;
    let second = session(&db, Some(&agent)).await;
    let unrelated = session(&db, None).await;
    let ctx = ctx(&db);

    // Declared without a value: nothing for the shell yet.
    let declared = run(
        &ctx,
        "create_agent_credential_binding",
        json!({
            "agent_id": agent.id.to_string(),
            "env_name": "JEV_API_KEY",
            "url": "https://api.typesafe.ai/",
            "label": "Jev API key",
        }),
    )
    .await
    .unwrap();
    assert_eq!(declared["binding"]["url"], "https://api.typesafe.ai");
    assert_eq!(declared["binding"]["auth"]["type"], "bearer");
    assert_eq!(
        run(&ctx, "worker_resolve_bound_secrets", sid(first))
            .await
            .unwrap(),
        json!([])
    );

    agent_credential(&ctx, &agent, "JEV_API_KEY", KEY).await;
    for session_id in [first, second] {
        let resolved = run(&ctx, "worker_resolve_bound_secrets", sid(session_id))
            .await
            .unwrap();
        assert_eq!(resolved[0]["name"], "JEV_API_KEY");
        assert_eq!(resolved[0]["value"], KEY);
        assert_eq!(resolved[0]["scope"], "agent");
    }
    assert_eq!(
        run(&ctx, "worker_resolve_bound_secrets", sid(unrelated))
            .await
            .unwrap(),
        json!([])
    );

    // THREAT[TM-BASH-030]: the runtime cannot overwrite the agent's key.
    let swap =
        json!({ "session_id": first.to_string(), "name": "JEV_API_KEY", "value": "attacker" });
    run(&ctx, "worker_set_session_secret", swap)
        .await
        .expect_err("bound set");

    // Re-declaring with the same binding keeps the value; pointing it at
    // another host drops it, so the person has to paste it again.
    let same = run(
        &ctx,
        "create_agent_credential_binding",
        json!({
            "agent_id": agent.id.to_string(),
            "env_name": "JEV_API_KEY",
            "url": "https://api.typesafe.ai",
            "label": "Jev API key (renamed)",
        }),
    )
    .await
    .unwrap();
    assert_eq!(same["configured"], true);
    let moved = run(
        &ctx,
        "create_agent_credential_binding",
        json!({
            "agent_id": agent.id.to_string(),
            "env_name": "JEV_API_KEY",
            "url": "https://attacker.example",
            "label": "Jev API key",
        }),
    )
    .await
    .unwrap();
    assert_eq!(moved["configured"], false);
    assert_eq!(
        run(&ctx, "worker_resolve_bound_secrets", sid(second))
            .await
            .unwrap(),
        json!([])
    );

    // A session secret of the same name wins in that session only.
    agent_credential(&ctx, &agent, "JEV_API_KEY", KEY).await;
    run(
        &ctx,
        "batch_set_session_secrets",
        json!({
            "session_id": first.to_string(),
            "secrets": { "JEV_API_KEY": "session-key" },
            "bindings": { "JEV_API_KEY": jev_binding() },
        }),
    )
    .await
    .unwrap();
    let resolved = run(&ctx, "worker_resolve_bound_secrets", sid(first))
        .await
        .unwrap();
    assert_eq!(resolved.as_array().unwrap().len(), 1);
    assert_eq!(resolved[0]["value"], "session-key");
    let resolved = run(&ctx, "worker_resolve_bound_secrets", sid(second))
        .await
        .unwrap();
    assert_eq!(resolved[0]["value"], KEY);

    // HTTP credentials never reach MCP tool argument injection.
    let mcp = crate::domains::agents::credentials::resolve_runtime_secret_bindings(
        &db,
        Some(&encryption()),
        DEFAULT_ORG_ID,
        Some(agent.id),
        "jev",
        "https://api.typesafe.ai",
    )
    .await
    .unwrap();
    assert!(mcp.is_empty());
}

#[tokio::test]
async fn bindings_must_be_https_header_only_env_names() {
    let db = Arc::new(StorageBackend::test_database());
    let agent = agent(&db).await;
    let ctx = ctx(&db);

    for request in [
        json!({ "env_name": "JEV", "url": "http://api.typesafe.ai" }),
        json!({ "env_name": "JEV", "url": "https://u:p@api.typesafe.ai" }),
        json!({ "env_name": "JEV", "url": "https://api.typesafe.ai?k=1" }),
        json!({ "env_name": "JEV", "url": "https://api.typesafe.ai", "auth": "header", "header_name": "X Key" }),
        json!({ "env_name": "JEV", "url": "https://api.typesafe.ai", "auth": "header" }),
        json!({ "env_name": "JEV", "url": "https://api.typesafe.ai", "auth": "basic" }),
        json!({ "env_name": "JEV" }),
        json!({ "env_name": "not-an-env", "url": "https://api.typesafe.ai" }),
        json!({ "env_name": "JEV", "url": "https://api.typesafe.ai", "tool_name": "x" }),
        json!({ "url": "https://api.typesafe.ai" }),
    ] {
        let mut request = request;
        request["agent_id"] = json!(agent.id.to_string());
        request["label"] = json!("Jev");
        let error = run(&ctx, "create_agent_credential_binding", request.clone())
            .await
            .expect_err("invalid binding");
        assert!(
            matches!(error.kind, CommandErrorKind::BadRequest(_)),
            "{request}: {error:?}"
        );
    }
    let header = run(
        &ctx,
        "create_agent_credential_binding",
        json!({
            "agent_id": agent.id.to_string(),
            "label": "Jev",
            "env_name": "JEV",
            "url": "https://api.typesafe.ai/v1",
            "auth": "header",
            "header_name": "X-Api-Key",
        }),
    )
    .await
    .unwrap();
    assert_eq!(header["binding"]["auth"]["name"], "X-Api-Key");

    let session_id = session(&db, None).await;
    let orphan = run(
        &ctx,
        "batch_set_session_secrets",
        json!({
            "session_id": session_id.to_string(),
            "secrets": {},
            "bindings": { "JEV": jev_binding() },
        }),
    )
    .await;
    // Empty secrets are a no-op before bindings are looked at.
    assert!(orphan.is_ok());
    run(
        &ctx,
        "batch_set_session_secrets",
        json!({
            "session_id": session_id.to_string(),
            "secrets": { "OTHER": "x" },
            "bindings": { "JEV": jev_binding() },
        }),
    )
    .await
    .expect_err("binding without a value");
}

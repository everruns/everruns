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

#[tokio::test]
async fn agent_secrets_reach_every_session_of_the_agent_and_sessions_shadow_them() {
    let db = Arc::new(StorageBackend::test_database());
    let agent = agent(&db).await;
    let first = session(&db, Some(&agent)).await;
    let second = session(&db, Some(&agent)).await;
    let unrelated = session(&db, None).await;
    let ctx = ctx(&db);

    let stored = run(
        &ctx,
        "set_agent_secret",
        json!({
            "agent_id": agent.public_id,
            "name": "JEV_API_KEY",
            "value": KEY,
            "binding": jev_binding(),
        }),
    )
    .await
    .expect("set agent secret");
    assert!(!stored.to_string().contains(KEY));

    for session_id in [first, second] {
        let resolved = run(&ctx, "worker_resolve_bound_secrets", sid(session_id))
            .await
            .unwrap();
        assert_eq!(resolved[0]["value"], KEY);
        assert_eq!(resolved[0]["scope"], "agent");
    }
    assert_eq!(
        run(&ctx, "worker_resolve_bound_secrets", sid(unrelated))
            .await
            .unwrap(),
        json!([])
    );

    // A session secret of the same name wins in that session only.
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

    let listed = run(
        &ctx,
        "list_agent_secrets",
        json!({ "agent_id": agent.public_id }),
    )
    .await
    .unwrap();
    assert_eq!(listed[0]["name"], "JEV_API_KEY");
    assert!(!listed.to_string().contains(KEY));

    run(
        &ctx,
        "delete_agent_secret",
        json!({ "agent_id": agent.public_id, "name": "JEV_API_KEY" }),
    )
    .await
    .unwrap();
    assert_eq!(
        run(&ctx, "worker_resolve_bound_secrets", sid(second))
            .await
            .unwrap(),
        json!([])
    );
}

#[tokio::test]
async fn bindings_must_be_https_header_only_env_names() {
    let db = Arc::new(StorageBackend::test_database());
    let agent = agent(&db).await;
    let ctx = ctx(&db);

    for (name, binding) in [
        (
            "JEV",
            json!({ "url": "http://api.typesafe.ai", "auth": { "type": "bearer" } }),
        ),
        (
            "JEV",
            json!({ "url": "https://u:p@api.typesafe.ai", "auth": { "type": "bearer" } }),
        ),
        (
            "JEV",
            json!({ "url": "https://api.typesafe.ai?k=1", "auth": { "type": "bearer" } }),
        ),
        (
            "JEV",
            json!({ "url": "https://api.typesafe.ai", "auth": { "type": "header", "name": "X Key" } }),
        ),
        ("not-an-env", jev_binding()),
    ] {
        let error = run(
            &ctx,
            "set_agent_secret",
            json!({ "agent_id": agent.public_id, "name": name, "value": KEY, "binding": binding }),
        )
        .await
        .expect_err("invalid binding");
        assert!(
            matches!(error.kind, CommandErrorKind::BadRequest(_)),
            "{error:?}"
        );
    }

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

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
            localizations: Default::default(),
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
            max_output_tokens: None,
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

async fn invocation(
    db: &StorageBackend,
    session: SessionId,
    responder: Option<AgentId>,
) -> everruns_contracts::runtime::credential_services::CredentialInvocation {
    let input = uuid::Uuid::now_v7();
    db.record_runtime_invocation(
        DEFAULT_ORG_ID,
        session,
        input,
        None,
        None,
        responder.map(|v| v.uuid()),
    )
    .await
    .unwrap();
    everruns_contracts::runtime::credential_services::CredentialInvocation {
        session_id: session,
        input_message_id: input,
    }
}
async fn resolve(
    ctx: &Ctx,
    inv: everruns_contracts::runtime::credential_services::CredentialInvocation,
) -> Result<Value, CommandError> {
    crate::domains::credentials::service::execute(
        ctx,
        inv,
        "resolve",
        json!({"usage":"service","names":["JEV_API_KEY"]}),
    )
    .await
}
#[tokio::test]
async fn legacy_session_entry_adapts_to_one_binding_store_and_old_workers_fail_closed() {
    let db = Arc::new(StorageBackend::test_database());
    let session_id = session(&db, None).await;
    let ctx = ctx(&db);
    run(&ctx,"batch_set_session_secrets",json!({"session_id":session_id.to_string(),"secrets":{"JEV_API_KEY":KEY,"PLAIN":"readable"},"bindings":{"JEV_API_KEY":jev_binding()}})).await.unwrap();
    let old: i64 =
        sqlx::query_scalar("SELECT count(*) FROM session_secrets WHERE binding IS NOT NULL")
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert_eq!(old, 0);
    let listed = run(&ctx, "list_session_secrets", sid(session_id))
        .await
        .unwrap();
    assert!(!listed.to_string().contains(KEY));
    assert!(listed.as_array().unwrap().iter().any(|r|r["name"]=="JEV_API_KEY"&&r["binding"]["url"]=="https://api.typesafe.ai"));
    for command in [
        "worker_get_session_secret",
        "worker_set_session_secret",
        "worker_delete_session_secret",
    ] {
        assert!(
            run(
                &ctx,
                command,
                json!({"session_id":session_id.to_string(),"name":"JEV_API_KEY","value":"attacker"})
            )
            .await
            .is_err()
        );
    }
    assert!(
        run(&ctx, "worker_resolve_bound_secrets", sid(session_id))
            .await
            .is_err()
    );
    let inv = invocation(&db, session_id, None).await;
    let resolved = resolve(&ctx, inv).await.unwrap();
    assert_eq!(resolved[0]["info"]["scope"], "session");
    assert_eq!(resolved[0]["value"], KEY);
    assert_eq!(
        run(
            &ctx,
            "worker_get_session_secret",
            json!({"session_id":session_id.to_string(),"name":"PLAIN"})
        )
        .await
        .unwrap(),
        json!("readable")
    );
    run(
        &ctx,
        "batch_set_session_secrets",
        json!({"session_id":session_id.to_string(),"secrets":{"JEV_API_KEY":"newly-provided"}}),
    )
    .await
    .unwrap();
    assert!(resolve(&ctx, inv).await.is_err());
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
        .set_credential_binding_value(DEFAULT_ORG_ID, agent.id, id, encrypted)
        .await
        .unwrap()
        .unwrap();
    declared
}

#[tokio::test]
async fn agent_credentials_follow_the_actual_responder_and_never_use_scope_precedence() {
    let db = Arc::new(StorageBackend::test_database());
    let agent = agent(&db).await;
    let first = session(&db, Some(&agent)).await;
    let second = session(&db, Some(&agent)).await;
    let ctx = ctx(&db);
    agent_credential(&ctx, &agent, "JEV_API_KEY", KEY).await;
    for session in [first, second] {
        let inv = invocation(&db, session, Some(agent.id)).await;
        let resolved = resolve(&ctx, inv).await.unwrap();
        assert_eq!(resolved[0]["value"], KEY);
        assert_eq!(resolved[0]["info"]["scope"], "agent");
    }
    let guest = invocation(&db, first, None).await;
    assert!(resolve(&ctx, guest).await.is_err());
    let inv = invocation(&db, first, Some(agent.id)).await;
    run(&ctx,"batch_set_session_secrets",json!({"session_id":first.to_string(),"secrets":{"JEV_API_KEY":"session-key"},"bindings":{"JEV_API_KEY":jev_binding()}})).await.unwrap();
    assert!(
        resolve(&ctx, inv).await.is_err(),
        "ambiguous names must never silently shadow another owner"
    );
    let moved=run(&ctx,"create_agent_credential_binding",json!({"agent_id":agent.id.to_string(),"env_name":"JEV_API_KEY","url":"https://other.example","label":"Moved key"})).await.unwrap();
    assert_eq!(moved["configured"], false);
    let mcp = crate::domains::agents::credentials::resolve_runtime_secret_bindings(
        &db,
        Some(&encryption()),
        DEFAULT_ORG_ID,
        Some(inv),
        everruns_core::McpServerActsAs::Service,
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

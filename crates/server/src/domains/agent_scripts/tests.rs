// Unit tests for agent-script validation and commands, on the embedded test
// database like the agent-trigger tests.

use super::validation::*;
use super::*;
use crate::domains::agent_scripts::types::{CreateAgentScriptRequest, UpdateAgentScriptRequest};
use crate::domains::common::{Command, Ctx};
use crate::storage::StorageBackend;
use axum::http::StatusCode;
use everruns_contracts::typed_id::AgentId;
use everruns_core::{Caller, DEFAULT_ORG_ID};
use serde_json::{Value, json};
use std::sync::Arc;

async fn seed() -> (Arc<StorageBackend>, String) {
    let db = Arc::new(StorageBackend::test_database());
    let agent = AgentId::from_uuid(
        db.create_test_agent(DEFAULT_ORG_ID, uuid::Uuid::now_v7())
            .await,
    );
    (db, agent.to_string())
}

fn internal_ctx(db: Arc<StorageBackend>) -> Ctx {
    Ctx::minimal_for_test(Caller::internal(DEFAULT_ORG_ID), db, None)
}

fn req(name: &str) -> CreateAgentScriptRequest {
    CreateAgentScriptRequest {
        name: name.to_string(),
        description: "Says hello".to_string(),
        input_schema: Some(json!({"type": "object", "properties": {"who": {"type": "string"}}})),
        body: "echo hello".to_string(),
    }
}

async fn create(
    ctx: &Ctx,
    agent_id: &str,
    req: CreateAgentScriptRequest,
) -> Result<crate::records::AgentScript, crate::domains::common::CommandError> {
    CreateAgentScript {
        agent_id: agent_id.to_string(),
        req,
    }
    .run(ctx)
    .await
}

#[test]
fn name_validation_matches_the_contract() {
    for ok in ["a", "daily-digest", "a_b-c9", &"a".repeat(64)] {
        assert!(validate_name(ok).is_ok(), "{ok} should be valid");
    }
    for bad in [
        "",
        "Upper",
        "1abc",
        "-abc",
        "_abc",
        "has space",
        "dot.name",
        "ünï",
        &"a".repeat(65),
    ] {
        assert!(validate_name(bad).is_err(), "{bad:?} should be invalid");
    }
}

#[test]
fn description_and_body_limits() {
    assert!(validate_description("x").is_ok());
    assert!(validate_description(&"x".repeat(300)).is_ok());
    assert!(validate_description("").is_err());
    assert!(validate_description(&"x".repeat(301)).is_err());
    // Characters, not bytes.
    assert!(validate_description(&"é".repeat(300)).is_ok());

    assert!(validate_body("x").is_ok());
    assert!(validate_body(&"x".repeat(MAX_BODY_BYTES)).is_ok());
    assert!(validate_body("").is_err());
    assert!(validate_body(&"x".repeat(MAX_BODY_BYTES + 1)).is_err());
}

#[test]
fn input_schema_must_be_an_object_schema() {
    assert!(validate_input_schema(&json!({"type": "object"})).is_ok());
    assert!(validate_input_schema(&json!({"type": "string"})).is_err());
    assert!(validate_input_schema(&json!({"properties": {}})).is_err());
    assert!(validate_input_schema(&json!(["object"])).is_err());
    assert!(validate_input_schema(&json!("object")).is_err());
    assert!(validate_input_schema(&Value::Null).is_err());
}

#[tokio::test]
async fn create_get_list_update_delete_round_trip() {
    let (db, agent_id) = seed().await;
    let ctx = internal_ctx(db);

    let created = create(&ctx, &agent_id, req("hello")).await.unwrap();
    assert_eq!(created.name, "hello");
    assert_eq!(created.agent_id.to_string(), agent_id);
    assert!(created.id.to_string().starts_with("scr_"));

    let fetched = GetAgentScript {
        agent_id: agent_id.clone(),
        script_id: created.id.to_string(),
    }
    .run(&ctx)
    .await
    .unwrap();
    assert_eq!(fetched.body, "echo hello");

    let listed = ListAgentScripts {
        agent_id: agent_id.clone(),
        include_archived: false,
    }
    .run(&ctx)
    .await
    .unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].body, "echo hello", "list returns full records");

    let updated = UpdateAgentScriptCmd {
        agent_id: agent_id.clone(),
        script_id: created.id.to_string(),
        req: UpdateAgentScriptRequest {
            body: Some("echo bye".to_string()),
            ..Default::default()
        },
    }
    .run(&ctx)
    .await
    .unwrap();
    assert_eq!(updated.body, "echo bye");
    assert_eq!(updated.name, "hello", "name is immutable");
    assert_eq!(
        updated.description, "Says hello",
        "untouched fields persist"
    );
    assert!(updated.input_schema.is_some());

    DeleteAgentScript {
        agent_id: agent_id.clone(),
        script_id: created.id.to_string(),
    }
    .run(&ctx)
    .await
    .unwrap();
    let active = ListAgentScripts {
        agent_id: agent_id.clone(),
        include_archived: false,
    }
    .run(&ctx)
    .await
    .unwrap();
    assert!(active.is_empty());
    let all = ListAgentScripts {
        agent_id: agent_id.clone(),
        include_archived: true,
    }
    .run(&ctx)
    .await
    .unwrap();
    assert_eq!(all.len(), 1);

    // Archived scripts are read-only and cannot be deleted twice.
    let error = UpdateAgentScriptCmd {
        agent_id: agent_id.clone(),
        script_id: created.id.to_string(),
        req: UpdateAgentScriptRequest {
            body: Some("echo again".to_string()),
            ..Default::default()
        },
    }
    .run(&ctx)
    .await
    .expect_err("archived script cannot change");
    assert_eq!(error.status(), StatusCode::NOT_FOUND);
    let error = DeleteAgentScript {
        agent_id,
        script_id: created.id.to_string(),
    }
    .run(&ctx)
    .await
    .expect_err("second delete finds nothing active");
    assert_eq!(error.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn create_rejects_invalid_input() {
    let (db, agent_id) = seed().await;
    let ctx = internal_ctx(db);

    let mut bad_name = req("Bad Name");
    bad_name.name = "Bad Name".to_string();
    let mut empty_description = req("a");
    empty_description.description = String::new();
    let mut long_body = req("b");
    long_body.body = "x".repeat(MAX_BODY_BYTES + 1);
    let mut empty_body = req("c");
    empty_body.body = String::new();
    let mut bad_schema = req("d");
    bad_schema.input_schema = Some(json!({"type": "array"}));

    for case in [
        bad_name,
        empty_description,
        long_body,
        empty_body,
        bad_schema,
    ] {
        let error = create(&ctx, &agent_id, case).await.expect_err("invalid");
        assert_eq!(error.status(), StatusCode::BAD_REQUEST);
    }
    let listed = ListAgentScripts {
        agent_id,
        include_archived: true,
    }
    .run(&ctx)
    .await
    .unwrap();
    assert!(listed.is_empty(), "nothing was stored");
}

#[tokio::test]
async fn update_validates_each_provided_field() {
    let (db, agent_id) = seed().await;
    let ctx = internal_ctx(db);
    let created = create(&ctx, &agent_id, req("hello")).await.unwrap();

    for patch in [
        UpdateAgentScriptRequest {
            description: Some(String::new()),
            ..Default::default()
        },
        UpdateAgentScriptRequest {
            body: Some("x".repeat(MAX_BODY_BYTES + 1)),
            ..Default::default()
        },
        UpdateAgentScriptRequest {
            input_schema: Some(json!({"type": "string"})),
            ..Default::default()
        },
    ] {
        let error = UpdateAgentScriptCmd {
            agent_id: agent_id.clone(),
            script_id: created.id.to_string(),
            req: patch,
        }
        .run(&ctx)
        .await
        .expect_err("invalid update");
        assert_eq!(error.status(), StatusCode::BAD_REQUEST);
    }
}

#[tokio::test]
async fn duplicate_active_name_conflicts_until_the_first_is_archived() {
    let (db, agent_id) = seed().await;
    let ctx = internal_ctx(db);

    let first = create(&ctx, &agent_id, req("hello")).await.unwrap();
    let error = create(&ctx, &agent_id, req("hello"))
        .await
        .expect_err("duplicate name");
    assert_eq!(error.status(), StatusCode::CONFLICT);

    DeleteAgentScript {
        agent_id: agent_id.clone(),
        script_id: first.id.to_string(),
    }
    .run(&ctx)
    .await
    .unwrap();
    create(&ctx, &agent_id, req("hello"))
        .await
        .expect("an archived script frees its name");
}

#[tokio::test]
async fn names_are_scoped_to_the_agent() {
    let (db, agent_a) = seed().await;
    let agent_b = AgentId::from_uuid(
        db.create_test_agent(DEFAULT_ORG_ID, uuid::Uuid::now_v7())
            .await,
    )
    .to_string();
    let ctx = internal_ctx(db);

    create(&ctx, &agent_a, req("hello")).await.unwrap();
    let on_b = create(&ctx, &agent_b, req("hello")).await.unwrap();

    // A script is not reachable through another agent's path.
    let error = GetAgentScript {
        agent_id: agent_a,
        script_id: on_b.id.to_string(),
    }
    .run(&ctx)
    .await
    .expect_err("wrong agent");
    assert_eq!(error.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn an_agent_holds_at_most_one_hundred_active_scripts() {
    let (db, agent_id) = seed().await;
    let ctx = internal_ctx(db);

    let mut first = None;
    for i in 0..MAX_ACTIVE_SCRIPTS_PER_AGENT {
        let script = create(&ctx, &agent_id, req(&format!("script-{i}")))
            .await
            .unwrap();
        first.get_or_insert(script);
    }
    let error = create(&ctx, &agent_id, req("one-too-many"))
        .await
        .expect_err("cap reached");
    assert_eq!(error.status(), StatusCode::BAD_REQUEST);

    // Archiving one makes room again.
    DeleteAgentScript {
        agent_id: agent_id.clone(),
        script_id: first.unwrap().id.to_string(),
    }
    .run(&ctx)
    .await
    .unwrap();
    create(&ctx, &agent_id, req("one-too-many")).await.unwrap();
}

#[tokio::test]
async fn unknown_agent_is_not_found_and_other_orgs_see_nothing() {
    let (db, agent_id) = seed().await;
    let ctx = internal_ctx(db.clone());
    let created = create(&ctx, &agent_id, req("hello")).await.unwrap();

    let error = create(&ctx, &AgentId::new().to_string(), req("hello"))
        .await
        .expect_err("unknown agent");
    assert_eq!(error.status(), StatusCode::NOT_FOUND);

    let other_org = Ctx::minimal_for_test(Caller::internal(DEFAULT_ORG_ID + 1000), db, None);
    let error = GetAgentScript {
        agent_id,
        script_id: created.id.to_string(),
    }
    .run(&other_org)
    .await
    .expect_err("org scoped");
    assert_eq!(error.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn internal_callers_pass_policy_and_policies_match_triggers() {
    let (db, agent_id) = seed().await;

    // The worker's gRPC ExecuteCommand (user_id None) resolves to
    // `Caller::internal(org_id)`, which `Command::run` lets through.
    let internal = internal_ctx(db.clone());
    create(&internal, &agent_id, req("from-worker"))
        .await
        .unwrap();
    let listed = ListAgentScripts {
        agent_id: agent_id.clone(),
        include_archived: false,
    }
    .run(&internal)
    .await
    .unwrap();
    assert_eq!(listed.len(), 1);

    // Not widened: scripts use exactly the policies triggers use.
    use crate::domains::agent_triggers::{CreateAgentTrigger, ListAgentTriggers};
    let id = |policy: Option<&'static crate::kernel_imports::Policy>| policy.map(|p| p.id);
    assert_eq!(
        id(CreateAgentScript::policy()),
        id(CreateAgentTrigger::policy())
    );
    assert_eq!(
        id(ListAgentScripts::policy()),
        id(ListAgentTriggers::policy())
    );
    assert_eq!(
        id(UpdateAgentScriptCmd::policy()),
        id(CreateAgentTrigger::policy())
    );
    assert_eq!(
        id(DeleteAgentScript::policy()),
        id(CreateAgentTrigger::policy())
    );
    assert_eq!(
        id(GetAgentScript::policy()),
        id(ListAgentTriggers::policy())
    );
}

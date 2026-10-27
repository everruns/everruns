use crate::domains::common::{CommandError, CommandErrorKind, Ctx, dispatch};
use crate::storage::StorageBackend;
use everruns_core::organization::OrgRole;
use everruns_core::{Caller, DEFAULT_ORG_ID};
use serde_json::{Value, json};
use std::sync::Arc;

const DEFAULT_ORG: &str = "org_00000000000000000000000000000001";

fn caller(role: OrgRole) -> Caller {
    Caller {
        org_id: DEFAULT_ORG_ID,
        org_public_id: DEFAULT_ORG.to_string(),
        user_id: Some(uuid::Uuid::now_v7()),
        role,
        is_platform_user: false,
        is_internal: false,
        read_only: false,
    }
}

fn ctx(db: &Arc<StorageBackend>, role: OrgRole) -> Ctx {
    Ctx::minimal_for_test(caller(role), db.clone(), None)
}

/// The `conversation` harness the examples name; a fresh test database has
/// no harnesses.
async fn seed_conversation_harness(db: &StorageBackend) {
    db.create_harness(
        DEFAULT_ORG_ID,
        crate::storage::CreateHarnessRow {
            localizations: Default::default(),
            name: "conversation".to_string(),
            display_name: Some("Conversation".to_string()),
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
    .expect("seed conversation harness");
}

async fn run(ctx: &Ctx, name: &str, params: Value) -> Result<Value, CommandError> {
    dispatch(name, params, ctx)
        .await
        .map(|json| serde_json::from_str(&json).unwrap())
}

#[tokio::test]
async fn lists_offered_examples_with_template_setup() {
    let db = Arc::new(StorageBackend::test_database());
    let examples = run(&ctx(&db, OrgRole::Member), "list_agent_examples", json!({}))
        .await
        .expect("members list examples");
    let examples = examples.as_array().expect("array output");
    let jokes = examples
        .iter()
        .find(|e| e["name"] == "dad-jokes-agent")
        .expect("dad-jokes-agent offered");
    assert_eq!(jokes["harness_name"], "conversation");
    assert!(jokes.get("setup").is_none(), "{jokes}");
    if let Some(reviewer) = examples.iter().find(|e| e["name"] == "pr-reviewer") {
        assert_eq!(reviewer["setup"]["connect_github"], true);
    }
}

#[tokio::test]
async fn adopting_creates_a_tagged_agent_and_suffixes_a_taken_name() {
    let db = Arc::new(StorageBackend::test_database());
    seed_conversation_harness(&db).await;
    let admin = ctx(&db, OrgRole::Admin);

    let first = run(
        &admin,
        "adopt_agent_example",
        json!({ "name": "dad-jokes-agent" }),
    )
    .await
    .expect("adopt example");
    assert_eq!(first["name"], "dad-jokes-agent");
    assert_eq!(first["display_name"], "Dad Jokes Agent");
    assert!(
        first["tags"]
            .as_array()
            .unwrap()
            .contains(&json!("example:dad-jokes-agent")),
        "{first}"
    );

    let second = run(
        &admin,
        "adopt_agent_example",
        json!({ "name": "dad-jokes-agent" }),
    )
    .await
    .expect("adopt the same example again");
    let second_name = second["name"].as_str().unwrap();
    assert!(
        second_name.starts_with("dad-jokes-agent-") && second_name != "dad-jokes-agent",
        "{second_name}"
    );
    assert_ne!(first["id"], second["id"]);
}

#[tokio::test]
async fn an_unknown_example_is_not_found_and_creates_nothing() {
    let db = Arc::new(StorageBackend::test_database());
    let admin = ctx(&db, OrgRole::Admin);
    let before = db.count_agents_for_org(DEFAULT_ORG_ID).await.unwrap();

    let error = run(
        &admin,
        "adopt_agent_example",
        json!({ "name": "no-such-example" }),
    )
    .await
    .expect_err("unknown example");
    assert!(
        matches!(error.kind, CommandErrorKind::NotFound(_)),
        "{error:?}"
    );
    assert_eq!(
        db.count_agents_for_org(DEFAULT_ORG_ID).await.unwrap(),
        before
    );
}

#[tokio::test]
async fn a_read_only_caller_cannot_adopt() {
    let db = Arc::new(StorageBackend::test_database());
    let mut read_only = caller(OrgRole::Admin);
    read_only.read_only = true;
    let ctx = Ctx::minimal_for_test(read_only, db.clone(), None);

    let error = run(
        &ctx,
        "adopt_agent_example",
        json!({ "name": "dad-jokes-agent" }),
    )
    .await
    .expect_err("read-only token cannot adopt");
    assert!(
        matches!(error.kind, CommandErrorKind::Forbidden(_)),
        "{error:?}"
    );
}

/// TM-AGENT-005 holds through adoption: a member cannot adopt an example
/// whose capabilities are high-risk.
#[tokio::test]
async fn a_member_cannot_adopt_an_example_with_high_risk_capabilities() {
    let db = Arc::new(StorageBackend::test_database());
    seed_conversation_harness(&db).await;
    let member = ctx(&db, OrgRole::Member);
    let examples = run(&member, "list_agent_examples", json!({}))
        .await
        .unwrap();
    let risky = examples.as_array().unwrap().iter().find(|example| {
        let ids: Vec<&str> = example["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|cap| cap.as_str().or_else(|| cap["ref"].as_str()))
            .collect();
        !member.capability_service.high_risk_ids(&ids).is_empty()
    });
    let Some(risky) = risky else {
        return; // No offered example carries a high-risk capability.
    };

    let error = run(
        &member,
        "adopt_agent_example",
        json!({ "name": risky["name"] }),
    )
    .await
    .expect_err("member cannot adopt a high-risk example");
    assert!(
        matches!(error.kind, CommandErrorKind::Forbidden(_)),
        "{error:?}"
    );
}

/// The gate adoption relies on is CreateAgent's: members are refused
/// high-risk capabilities (including the legacy `virtual_bash` alias) and keep
/// low-risk ones. This replaced the HTTP-only `require_admin_for_high_risk`.
#[tokio::test]
async fn members_are_refused_high_risk_capabilities_on_create() {
    let db = Arc::new(StorageBackend::test_database());
    seed_conversation_harness(&db).await;
    let member = ctx(&db, OrgRole::Member);
    let create = |name: &str, cap: &str| {
        json!({
            "name": name,
            "system_prompt": "Help.",
            "harness_name": "conversation",
            "capabilities": [{ "ref": cap }],
        })
    };
    for cap in ["bashkit_shell", "virtual_bash", "web_fetch"] {
        let error = run(&member, "create_agent", create("risky", cap))
            .await
            .expect_err("member must not assign a high-risk capability");
        assert!(
            matches!(error.kind, CommandErrorKind::Forbidden(_)),
            "{cap}: {error:?}"
        );
    }
    run(&member, "create_agent", create("clock", "current_time"))
        .await
        .expect("low-risk capabilities stay assignable by members");
}

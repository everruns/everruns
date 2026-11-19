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

/// The built-in parents the examples inherit from by name; a fresh test
/// database has no harnesses.
async fn seed_parent_harnesses(db: &StorageBackend) {
    for name in ["sandbox-worker", "bashkit-worker"] {
        db.create_harness(
            DEFAULT_ORG_ID,
            crate::storage::CreateHarnessRow {
                localizations: Default::default(),
                name: name.to_string(),
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
        .expect("seed parent harness");
    }
}

async fn run(ctx: &Ctx, name: &str, params: Value) -> Result<Value, CommandError> {
    dispatch(name, params, ctx)
        .await
        .map(|json| serde_json::from_str(&json).unwrap())
}

#[tokio::test]
async fn lists_the_coding_example() {
    let db = Arc::new(StorageBackend::test_database());
    let examples = run(
        &ctx(&db, OrgRole::Member),
        "list_harness_examples",
        json!({}),
    )
    .await
    .expect("members list harness examples");
    let coding = examples
        .as_array()
        .expect("array output")
        .iter()
        .find(|e| e["name"] == "coding")
        .expect("coding offered");
    assert_eq!(coding["parent_name"], "sandbox-worker");
    assert!(!coding["capabilities"].as_array().unwrap().is_empty());
}

#[test]
fn an_example_with_an_unregistered_capability_is_not_offered() {
    let all = super::offered_harness_examples(false, &|_| true);
    assert!(all.iter().any(|ex| ex.name == "coding"));
    let none = super::offered_harness_examples(false, &|_| false);
    assert!(
        none.iter().all(|ex| ex.capabilities.is_empty()),
        "examples that need capabilities must be hidden when none are registered"
    );
}

#[tokio::test]
async fn adopting_creates_a_harness_under_the_parent_and_suffixes_a_taken_name() {
    let db = Arc::new(StorageBackend::test_database());
    seed_parent_harnesses(&db).await;
    let admin = ctx(&db, OrgRole::Admin);
    let parent = db
        .get_harness_by_name(DEFAULT_ORG_ID, "sandbox-worker")
        .await
        .unwrap()
        .unwrap();

    let first = run(&admin, "adopt_harness_example", json!({ "name": "coding" }))
        .await
        .expect("adopt coding");
    assert_eq!(first["name"], "coding");
    assert_eq!(first["display_name"], "Coding");
    let stored = db
        .get_harness_by_name(DEFAULT_ORG_ID, "coding")
        .await
        .unwrap()
        .expect("adopted harness stored");
    assert_eq!(stored.parent_harness_id, Some(parent.id));
    assert!(!stored.is_built_in, "an adopted example is org-owned");

    let second = run(&admin, "adopt_harness_example", json!({ "name": "coding" }))
        .await
        .expect("adopt coding again");
    let second_name = second["name"].as_str().unwrap();
    assert!(
        second_name.starts_with("coding-") && second_name != "coding",
        "{second_name}"
    );
    assert_ne!(first["id"], second["id"]);
}

#[tokio::test]
async fn an_unknown_example_is_not_found_and_creates_nothing() {
    let db = Arc::new(StorageBackend::test_database());
    seed_parent_harnesses(&db).await;
    let admin = ctx(&db, OrgRole::Admin);
    let before = db.count_harnesses_for_org(DEFAULT_ORG_ID).await.unwrap();

    for name in ["no-such-example", "  "] {
        let error = run(&admin, "adopt_harness_example", json!({ "name": name }))
            .await
            .expect_err("unknown or empty example");
        assert!(
            matches!(
                error.kind,
                CommandErrorKind::NotFound(_) | CommandErrorKind::BadRequest(_)
            ),
            "{error:?}"
        );
    }
    assert_eq!(
        db.count_harnesses_for_org(DEFAULT_ORG_ID).await.unwrap(),
        before
    );
}

#[tokio::test]
async fn a_read_only_caller_cannot_adopt() {
    let db = Arc::new(StorageBackend::test_database());
    seed_parent_harnesses(&db).await;

    let mut read_only = caller(OrgRole::Admin);
    read_only.read_only = true;
    let read_only = Ctx::minimal_for_test(read_only, db.clone(), None);
    let error = run(
        &read_only,
        "adopt_harness_example",
        json!({ "name": "coding" }),
    )
    .await
    .expect_err("read-only token cannot adopt");
    assert!(
        matches!(error.kind, CommandErrorKind::Forbidden(_)),
        "{error:?}"
    );
}

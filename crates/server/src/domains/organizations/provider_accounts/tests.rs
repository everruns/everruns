use crate::domains::common::{CommandError, CommandErrorKind, Ctx, dispatch};
use crate::storage::{CreateOrganizationConnectionRow, StorageBackend};
use everruns_core::organization::OrgRole;
use everruns_core::{Caller, DEFAULT_ORG_ID};
use serde_json::{Value, json};
use std::sync::Arc;

const DEFAULT_ORG: &str = "org_00000000000000000000000000000001";
const OTHER_ORG: &str = "org_00000000000000000000000000000099";
const SECRET: &[u8] = b"sk-live-never-shown";

fn ctx(db: &Arc<StorageBackend>, role: OrgRole) -> Ctx {
    let caller = Caller {
        org_id: DEFAULT_ORG_ID,
        org_public_id: DEFAULT_ORG.to_string(),
        user_id: Some(uuid::Uuid::now_v7()),
        role,
        is_platform_user: false,
        is_internal: false,
        read_only: false,
    };
    Ctx::minimal_for_test(caller, db.clone(), None)
}

async fn run(ctx: &Ctx, params: Value) -> Result<Value, CommandError> {
    dispatch("list_organization_connections", params, ctx)
        .await
        .map(|json| serde_json::from_str(&json).unwrap())
}

async fn seed_account(db: &StorageBackend) {
    db.create_organization_connection(CreateOrganizationConnectionRow {
        org_id: DEFAULT_ORG_ID,
        name: "Production Daytona".to_string(),
        provider: "daytona".to_string(),
        access_token_encrypted: Some(SECRET.to_vec()),
        provider_username: Some("platform-team".to_string()),
        provider_metadata: Some(json!({ "api_url": "https://daytona.internal" })),
        ..Default::default()
    })
    .await
    .expect("seed organization connection");
}

#[tokio::test]
async fn an_admin_lists_account_metadata_without_secrets() {
    let db = Arc::new(StorageBackend::test_database());
    seed_account(&db).await;

    let accounts = run(&ctx(&db, OrgRole::Admin), json!({ "org": DEFAULT_ORG }))
        .await
        .expect("admin lists provider accounts");
    let accounts = accounts.as_array().expect("array output");
    assert_eq!(accounts.len(), 1);
    let account = &accounts[0];
    assert_eq!(account["name"], "Production Daytona");
    assert_eq!(account["provider"], "daytona");
    assert_eq!(account["provider_username"], "platform-team");
    let keys: Vec<&str> = account
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        [
            "connected_at",
            "id",
            "name",
            "provider",
            "provider_username",
            "updated_at"
        ],
        "only metadata: {account}"
    );
    let text = account.to_string();
    assert!(!text.contains("sk-live"), "{text}");
    assert!(!text.contains("daytona.internal"), "{text}");
}

#[tokio::test]
async fn a_member_is_refused() {
    let db = Arc::new(StorageBackend::test_database());
    seed_account(&db).await;

    let error = run(&ctx(&db, OrgRole::Member), json!({ "org": DEFAULT_ORG }))
        .await
        .expect_err("members cannot list provider accounts");
    assert!(
        matches!(error.kind, CommandErrorKind::Forbidden(_)),
        "{error:?}"
    );
}

#[tokio::test]
async fn another_or_malformed_org_is_not_found() {
    let db = Arc::new(StorageBackend::test_database());
    let admin = ctx(&db, OrgRole::Admin);
    for org in [OTHER_ORG, "not-an-org"] {
        let error = run(&admin, json!({ "org": org }))
            .await
            .expect_err("only the active org");
        assert!(
            matches!(error.kind, CommandErrorKind::NotFound(_)),
            "{org}: {error:?}"
        );
    }
}

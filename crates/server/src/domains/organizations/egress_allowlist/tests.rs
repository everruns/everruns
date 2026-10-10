use crate::domains::common::{CommandError, CommandErrorKind, Ctx, dispatch};
use crate::domains::organizations::record::generate_org_public_id;
use crate::storage::{CreateOrganizationRow, StorageBackend};
use everruns_contracts::runtime::OrgEgressAllowlist;
use everruns_core::organization::{OrgRole, org_public_id_from_internal};
use everruns_core::{Caller, DEFAULT_ORG_ID};
use serde_json::{Value, json};
use std::sync::Arc;

const DEFAULT_ORG: &str = "org_00000000000000000000000000000001";

fn person(org_id: i64, org_public_id: &str, role: OrgRole, platform: bool) -> Caller {
    Caller {
        org_id,
        org_public_id: org_public_id.to_string(),
        user_id: Some(uuid::Uuid::now_v7()),
        role,
        is_platform_user: platform,
        is_internal: false,
    }
}

fn ctx(db: &Arc<StorageBackend>, caller: Caller) -> Ctx {
    Ctx::minimal_for_test(caller, db.clone(), None)
}

async fn run(ctx: &Ctx, name: &str, params: Value) -> Result<Value, CommandError> {
    dispatch(name, params, ctx)
        .await
        .map(|json| serde_json::from_str(&json).unwrap())
}

fn admin(org_public_id: &str) -> Caller {
    person(DEFAULT_ORG_ID, org_public_id, OrgRole::Admin, false)
}

fn platform_user() -> Caller {
    person(DEFAULT_ORG_ID, DEFAULT_ORG, OrgRole::Member, true)
}

async fn grant(db: &Arc<StorageBackend>, org: &str, granted: bool) -> Value {
    run(
        &ctx(db, platform_user()),
        "set_org_egress_allowlist_grant",
        json!({ "org": org, "granted": granted }),
    )
    .await
    .expect("platform user grants")
}

fn assert_kind(result: Result<Value, CommandError>, expected: fn(&CommandErrorKind) -> bool) {
    let error = result.expect_err("command must fail");
    assert!(expected(&error.kind), "{error:?}");
}

#[tokio::test]
async fn org_admin_cannot_edit_without_a_grant() {
    let db = Arc::new(StorageBackend::test_database());
    let admin = ctx(&db, admin(DEFAULT_ORG));

    let state = run(
        &admin,
        "get_org_egress_allowlist",
        json!({ "org": DEFAULT_ORG }),
    )
    .await
    .unwrap();
    assert_eq!(state["granted"], false);
    assert_eq!(state["patterns"], json!([]));
    assert_eq!(state["mode"], "open");

    assert_kind(
        run(
            &admin,
            "set_org_egress_allowlist",
            json!({ "org": DEFAULT_ORG, "patterns": ["api.acme-corp.com"] }),
        )
        .await,
        |kind| matches!(kind, CommandErrorKind::Forbidden(_)),
    );
    assert!(
        db.org_egress_allowlist(DEFAULT_ORG_ID)
            .await
            .unwrap()
            .patterns
            .is_empty()
    );
}

#[tokio::test]
async fn only_platform_users_grant() {
    let db = Arc::new(StorageBackend::test_database());
    for caller in [
        person(DEFAULT_ORG_ID, DEFAULT_ORG, OrgRole::Owner, false),
        admin(DEFAULT_ORG),
    ] {
        assert_kind(
            run(
                &ctx(&db, caller),
                "set_org_egress_allowlist_grant",
                json!({ "org": DEFAULT_ORG, "granted": true }),
            )
            .await,
            |kind| matches!(kind, CommandErrorKind::Forbidden(_)),
        );
    }
    assert!(
        !db.org_egress_allowlist(DEFAULT_ORG_ID)
            .await
            .unwrap()
            .granted
    );
}

#[tokio::test]
async fn granted_admin_edits_and_a_revoke_keeps_but_stops_the_list() {
    let db = Arc::new(StorageBackend::test_database());
    // The platform user is not a member of this org and grants it anyway.
    let other = db
        .create_organization(CreateOrganizationRow {
            public_id: generate_org_public_id(),
            name: "Acme".to_string(),
            created_by: None,
        })
        .await
        .unwrap();
    let granted = grant(&db, &other.public_id, true).await;
    assert_eq!(granted["granted"], true);

    let admin = ctx(
        &db,
        person(other.org_id, &other.public_id, OrgRole::Admin, false),
    );
    let saved = run(
        &admin,
        "set_org_egress_allowlist",
        json!({
            "org": other.public_id,
            "patterns": [" API.acme-corp.com ", "", "*.acme-corp.io", "api.acme-corp.com"],
        }),
    )
    .await
    .unwrap();
    assert_eq!(
        saved["patterns"],
        json!(["api.acme-corp.com", "*.acme-corp.io"])
    );

    // Invalid patterns are rejected whole, leaving the stored list alone.
    for bad in [
        json!(["*"]),
        json!(["*.com"]),
        json!(["127.0.0.1"]),
        json!(["localhost"]),
    ] {
        assert_kind(
            run(
                &admin,
                "set_org_egress_allowlist",
                json!({ "org": other.public_id, "patterns": bad }),
            )
            .await,
            |kind| matches!(kind, CommandErrorKind::BadRequest(_)),
        );
    }
    let too_many: Vec<String> = (0..51).map(|i| format!("h{i}.acme-corp.com")).collect();
    assert_kind(
        run(
            &admin,
            "set_org_egress_allowlist",
            json!({ "org": other.public_id, "patterns": too_many }),
        )
        .await,
        |kind| matches!(kind, CommandErrorKind::BadRequest(_)),
    );

    let worker = ctx(&db, Caller::internal(other.org_id));
    let enforced = run(&worker, "worker_get_org_egress_allowlist", json!({}))
        .await
        .unwrap();
    assert_eq!(
        enforced["patterns"],
        json!(["api.acme-corp.com", "*.acme-corp.io"])
    );

    let resolver = crate::platform::DbOrgEgressAllowlist::new(db.clone());
    let org_id = org_public_id_from_internal(other.org_id).parse().unwrap();
    let extension = resolver.extension(&org_id).await.unwrap().expect("granted");
    assert!(extension.is_url_allowed("https://x.acme-corp.io/mcp"));

    let revoked = grant(&db, &other.public_id, false).await;
    assert_eq!(revoked["granted"], false);
    assert_eq!(
        revoked["patterns"],
        json!(["api.acme-corp.com", "*.acme-corp.io"]),
        "revoking keeps the list"
    );
    let enforced = run(&worker, "worker_get_org_egress_allowlist", json!({}))
        .await
        .unwrap();
    assert_eq!(enforced["patterns"], json!([]), "but stops enforcing it");
    assert!(resolver.extension(&org_id).await.unwrap().is_none());
    assert_kind(
        run(
            &admin,
            "set_org_egress_allowlist",
            json!({ "org": other.public_id, "patterns": ["api.acme-corp.com"] }),
        )
        .await,
        |kind| matches!(kind, CommandErrorKind::Forbidden(_)),
    );
}

#[tokio::test]
async fn members_cannot_edit_and_admins_cannot_reach_other_orgs() {
    let db = Arc::new(StorageBackend::test_database());
    grant(&db, DEFAULT_ORG, true).await;
    let member = ctx(
        &db,
        person(DEFAULT_ORG_ID, DEFAULT_ORG, OrgRole::Member, false),
    );
    assert_kind(
        run(
            &member,
            "set_org_egress_allowlist",
            json!({ "org": DEFAULT_ORG, "patterns": ["api.acme-corp.com"] }),
        )
        .await,
        |kind| matches!(kind, CommandErrorKind::Forbidden(_)),
    );

    let other = db
        .create_organization(CreateOrganizationRow {
            public_id: generate_org_public_id(),
            name: "Other".to_string(),
            created_by: None,
        })
        .await
        .unwrap();
    let admin = ctx(&db, admin(DEFAULT_ORG));
    for (name, params) in [
        (
            "get_org_egress_allowlist",
            json!({ "org": other.public_id }),
        ),
        (
            "set_org_egress_allowlist",
            json!({ "org": other.public_id, "patterns": [] }),
        ),
    ] {
        assert_kind(run(&admin, name, params).await, |kind| {
            matches!(kind, CommandErrorKind::NotFound(_))
        });
    }
}

/// THREAT[TM-AUTHZ-002]: the worker's read is not a person's API.
#[tokio::test]
async fn a_person_cannot_run_the_worker_command() {
    let db = Arc::new(StorageBackend::test_database());
    let owner = ctx(
        &db,
        person(DEFAULT_ORG_ID, DEFAULT_ORG, OrgRole::Owner, true),
    );
    assert_kind(
        run(&owner, "worker_get_org_egress_allowlist", json!({})).await,
        |kind| matches!(kind, CommandErrorKind::Forbidden(_)),
    );
}

//! Sessions are scoped to their project (knowledge/security/multitenancy.md).

use super::tests::{create_request, external_test_ctx, seed_harness, test_ctx};
use super::*;
use crate::storage::StorageBackend;
use everruns_core::{Caller, DEFAULT_ORG_ID};
use std::sync::Arc;
use uuid::Uuid;

/// Sessions sit in a project: an agentless session lands in the project
/// the caller is working in, a caller in another project reads it as
/// missing, and internal callers stay org-wide.
#[tokio::test]
async fn sessions_are_scoped_to_their_project() {
    let db = Arc::new(StorageBackend::in_memory());
    let internal_ctx = test_ctx(db.clone(), 10);
    let harness_id = seed_harness(&internal_ctx).await;
    let project_c = db
        .create_project(crate::storage::models::CreateProjectRow {
            public_id: "proj_000000000000000000000000000000cc".to_string(),
            org_id: DEFAULT_ORG_ID,
            name: "Project C".to_string(),
            description: None,
            is_default: false,
        })
        .await
        .expect("create project")
        .project_id;
    let owner = db
        .create_user(crate::storage::models::CreateUserRow {
            email: format!("owner-{}@example.com", Uuid::now_v7()),
            name: "Owner".to_string(),
            avatar_url: None,
            roles: vec!["user".to_string()],
            password_hash: None,
            email_verified: true,
            auth_provider: None,
            auth_provider_id: None,
            external_id: None,
        })
        .await
        .expect("create owner user");
    // A real (non-internal) caller working in project C.
    let mut ctx = external_test_ctx(db.clone(), owner.id);
    ctx.caller.project_id = project_c;

    let session = CreateSession(create_request(harness_id))
        .execute(&ctx)
        .await
        .expect("create agentless session");
    let row = db
        .get_session(DEFAULT_ORG_ID, session.id)
        .await
        .expect("load session")
        .expect("session exists");
    assert_eq!(row.project_id, project_c);

    let service = crate::domains::sessions::SessionService::new(db.clone());
    assert!(
        service
            .get(&ctx.caller, session.id.uuid(), None)
            .await
            .expect("get")
            .is_some()
    );
    let mut elsewhere = ctx.caller.clone();
    elsewhere.project_id = everruns_core::DEFAULT_PROJECT_ID;
    assert!(
        service
            .get(&elsewhere, session.id.uuid(), None)
            .await
            .expect("get")
            .is_none(),
        "another project reads the session as missing"
    );
    assert!(
        service
            .get(&Caller::internal(DEFAULT_ORG_ID), session.id.uuid(), None)
            .await
            .expect("get")
            .is_some(),
        "internal callers are org-wide"
    );
}

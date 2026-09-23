//! Project scope of internal vs. user callers.

use super::*;
use crate::storage::models::CreateOrganizationRow;

/// Internal callers carry a placeholder project, so they must read
/// org-wide and write to their own org's default project — never to
/// project 1, which belongs to the default org.
#[tokio::test]
async fn internal_callers_are_org_wide_and_write_to_their_org_default() {
    let db = Arc::new(StorageBackend::in_memory());
    db.create_organization_with_id(
        2,
        CreateOrganizationRow {
            public_id: "org_2".to_string(),
            name: "Org 2".to_string(),
            created_by: None,
        },
    )
    .await
    .expect("create org");
    let org_two_default = db
        .ensure_default_project(2)
        .await
        .expect("default project")
        .project_id;
    assert_ne!(org_two_default, everruns_core::DEFAULT_PROJECT_ID);

    let internal = Ctx::minimal_for_test(Caller::internal(2), db.clone(), None);
    assert_eq!(internal.project_scope(), None);
    assert_eq!(
        internal.creation_project_id().await.expect("project"),
        org_two_default
    );

    let mut user_caller = Caller::internal(2);
    user_caller.is_internal = false;
    user_caller.project_id = org_two_default;
    let user = Ctx::minimal_for_test(user_caller, db, None);
    assert_eq!(user.project_scope(), Some(org_two_default));
    assert_eq!(
        user.creation_project_id().await.expect("project"),
        org_two_default
    );
}

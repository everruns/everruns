//! Active-project resolution: selection honoured only while the flag is on.

use super::*;
use axum::http::HeaderValue;

/// With the flag on, a valid selection wins; with it off, every caller is
/// in the org's default project whatever the request selects.
#[tokio::test]
async fn active_project_selection_is_ignored_while_projects_flag_is_off() {
    let db = std::sync::Arc::new(StorageBackend::in_memory());
    let project = db
        .create_project(crate::storage::models::CreateProjectRow {
            public_id: "proj_000000000000000000000000000000cc".to_string(),
            org_id: DEFAULT_ORG_ID,
            name: "Project C".to_string(),
            description: None,
            is_default: false,
        })
        .await
        .expect("create project");
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        PROJECT_HEADER_NAME,
        HeaderValue::from_str(&project.public_id).expect("header"),
    );

    assert_eq!(
        resolve_active_project_id(Some(&db), &headers, DEFAULT_ORG_ID, true).await,
        project.project_id
    );
    assert_eq!(
        resolve_active_project_id(Some(&db), &headers, DEFAULT_ORG_ID, false).await,
        DEFAULT_PROJECT_ID
    );
}

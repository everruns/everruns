//! Foreign harness bindings must preserve tenant isolation.
use super::tests_support::*;
use super::*;
use crate::domains::common::Command;
use crate::domains::harnesses::types::CreateHarnessRequest;
use crate::kernel_imports::{Caller, DEFAULT_ORG_ID};
use crate::storage::StorageBackend;

#[tokio::test]
async fn create_rejects_harness_from_another_org() {
    let db = Arc::new(StorageBackend::test_database());
    let session_service = SessionService::new(db.clone());
    let caller = Caller::internal(DEFAULT_ORG_ID);
    let other_org_id = create_second_org(&db).await;
    let other_ctx = test_ctx(Caller::internal(other_org_id), db.clone()).await;

    let other_harness = crate::domains::harnesses::CreateHarness(CreateHarnessRequest {
        localizations: Default::default(),
        name: "other-harness".to_string(),
        display_name: Some("Other Harness".to_string()),
        description: None,
        intro_markdown: None,
        short_description: None,
        starters: Vec::new(),
        system_prompt: Some("Other".to_string()),
        parent_harness_id: None,
        default_model_id: None,
        tags: vec![],
        capabilities: vec![],
        initial_files: vec![],
        mcp_servers: Default::default(),
        network_access: None,
        embedder_metadata: Default::default(),
    })
    .execute(&other_ctx)
    .await
    .unwrap();

    let err = session_service
        .create(
            &caller,
            other_harness.id.uuid(),
            None,
            None,
            SessionSource::Api,
            build_create_request(other_harness.id, None, None),
        )
        .await
        .unwrap_err();

    let not_found = err.downcast_ref::<ResourceNotFoundError>().unwrap();
    assert_eq!(not_found.resource(), "Harness");
}

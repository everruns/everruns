//! Tests: Sessions started without a user and `session_user` sandbox accounts.

use super::*;
use crate::domains::agents::types::CreateAgentRequest;
use crate::domains::common::Command;
use crate::kernel_imports::{Caller, DEFAULT_ORG_ID};
use crate::storage::StorageBackend;

use super::tests_support::*;

/// A trigger-started Session has no user, so a `session_user` managed
/// template rebinds to the organization's sole account for the provider and
/// pins that account in the snapshot. Without exactly one account it fails
/// with an error that says why.
#[tokio::test]
async fn userless_session_binds_session_user_template_to_sole_org_account() {
    use everruns_contracts::session_sandbox::SessionSandboxCredentialSource;

    let db = Arc::new(StorageBackend::test_database());
    let session_service = SessionService::new(db.clone());
    let caller = Caller::internal(DEFAULT_ORG_ID);
    assert!(caller.user_id.is_none());
    let ctx = test_ctx(caller.clone(), db.clone()).await;

    let sandbox_policy = serde_json::from_value(serde_json::json!({
        "default": "main",
        "templates": {"main": {"target": {"kind": "managed", "provider": "daytona"}}}
    }))
    .unwrap();
    let agent = crate::domains::agents::CreateAgent(CreateAgentRequest {
        service_virtual_user_id: None,
        id: None,
        name: "pr-reviewer".to_string(),
        display_name: None,
        description: None,
        intro_markdown: None,
        short_description: None,
        starters: Vec::new(),
        system_prompt: "Review pull requests.".to_string(),
        default_model_id: None,
        harness_id: None,
        harness_name: Some("sandbox-worker".to_string()),
        tags: vec![],
        capabilities: vec![],
        sandbox_policy: Some(sandbox_policy),
        initial_files: vec![],
        tools: vec![],
        mcp_servers: Default::default(),
        network_access: None,
        max_iterations: None,
        parallel_tool_calls: None,
        max_output_tokens: None,
        communication: Default::default(),
    })
    .execute(&ctx)
    .await
    .unwrap();
    let harness_id = db
        .get_harness_by_name(DEFAULT_ORG_ID, "sandbox-worker")
        .await
        .unwrap()
        .unwrap()
        .id;
    let create = || {
        session_service.create(
            &caller,
            harness_id.uuid(),
            Some(agent.internal_id),
            Some(agent.public_id),
            SessionSource::Api,
            build_create_request(harness_id, Some(agent.public_id), None),
        )
    };
    let account = |name: &str| crate::storage::CreateOrganizationConnectionRow {
        org_id: DEFAULT_ORG_ID,
        name: name.to_string(),
        provider: "daytona".to_string(),
        access_token_encrypted: vec![1],
        provider_username: None,
        provider_metadata: None,
    };

    let none = create().await.unwrap_err().to_string();
    assert!(none.contains("no daytona account"), "{none}");

    let sole = db
        .create_organization_connection(account("Sandboxes"))
        .await
        .unwrap();
    let session = create().await.unwrap();
    let pinned = db
        .get_primary_sandbox(session.id)
        .await
        .unwrap()
        .expect("managed sandbox pinned");
    let credential = &pinned.spec.target.credential;
    assert_eq!(
        credential.source,
        SessionSandboxCredentialSource::Organization
    );
    assert_eq!(credential.connection_id, Some(sole.id));
    assert_eq!(
        credential.virtual_user_id,
        Some(sole.virtual_user_id.uuid())
    );

    db.create_organization_connection(account("Second"))
        .await
        .unwrap();
    let many = create().await.unwrap_err().to_string();
    assert!(many.contains("has 2 daytona accounts"), "{many}");
}

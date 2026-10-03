//! Runtime credentials never derive from management-owner lineage.

use super::*;

#[tokio::test]
async fn test_session_connection_resolution_never_borrows_owner_or_other_user() {
    let backend = create_test_backend().await;

    let owner = create_test_user(&backend, "owner").await;
    let other = create_test_user(&backend, "other").await;

    backend
        .add_organization_member(TEST_ORG_ID, owner.id, "member")
        .await
        .expect("Failed to add owner to org");
    backend
        .add_organization_member(TEST_ORG_ID, other.id, "member")
        .await
        .expect("Failed to add other user to org");

    let owner_runtime = backend
        .default_virtual_user(TEST_ORG_ID, owner.id)
        .await
        .unwrap();
    let other_runtime = backend
        .default_virtual_user(TEST_ORG_ID, other.id)
        .await
        .unwrap();

    let owner_principal_id = create_test_user_principal(&backend, TEST_ORG_ID, owner.id).await;
    let session = backend
        .create_session(CreateSessionRow {
            playground_user_id: None,

            trigger_id: None,
            source: everruns_platform::SessionSource::Api,
            workspace_id: None,
            org_id: TEST_ORG_ID,
            app_id: None,
            endpoint_id: None,
            harness_id: None,
            agent_id: None,
            agent_version_id: None,
            agent_config_hash: None,
            virtual_user_id: None,
            owner_principal_id,
            resolved_owner_user_id: Some(owner.id),
            title: Some(format!("connection-owner-scope-{}", Uuid::now_v7())),
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
            mcp_servers: serde_json::json!({}),
            system_prompt: None,
            initial_files: serde_json::Value::Array(vec![]),
            hints: None,
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
            blueprint_id: None,
            blueprint_config: None,
            parent_session_id: None,
            budget_root_session_id: None,
        })
        .await
        .expect("Failed to create session");

    backend
        .upsert_virtual_user_connection(CreateVirtualUserConnectionRow {
            virtual_user_id: other_runtime.id,
            provider: "gitlab".to_string(),
            connection_type: "oauth".to_string(),
            provider_user_id: Some("other-gitlab".to_string()),
            provider_username: Some("other".to_string()),
            access_token_encrypted: Some(b"other-token".to_vec()),
            refresh_token_encrypted: None,
            scopes: Some("api".to_string()),
            expires_at: None,
            installation_id: None,
            provider_metadata: Some(serde_json::json!({ "user": "other" })),
        })
        .await
        .expect("Failed to insert other user's OAuth connection");
    backend
        .upsert_virtual_user_connection(CreateVirtualUserConnectionRow {
            virtual_user_id: other_runtime.id,
            provider: "github".to_string(),
            connection_type: "oauth".to_string(),
            provider_user_id: Some("other-github".to_string()),
            provider_username: Some("other".to_string()),
            access_token_encrypted: None,
            refresh_token_encrypted: None,
            scopes: Some("contents:read".to_string()),
            expires_at: None,
            installation_id: Some(222),
            provider_metadata: None,
        })
        .await
        .expect("Failed to insert other user's GitHub installation");

    assert_eq!(
        backend
            .get_connection_token_for_session(session.id, "gitlab")
            .await
            .expect("Failed to resolve connection token"),
        None
    );
    assert_eq!(
        backend
            .get_connection_metadata_for_session(session.id, "gitlab")
            .await
            .expect("Failed to resolve connection metadata"),
        None
    );
    assert_eq!(
        backend
            .get_connection_user_for_session(session.id, "gitlab")
            .await
            .expect("Failed to resolve connection owner"),
        None
    );
    assert_eq!(
        backend
            .get_installation_id_for_session(session.id, "github")
            .await
            .expect("Failed to resolve GitHub installation"),
        None
    );

    backend
        .upsert_virtual_user_connection(CreateVirtualUserConnectionRow {
            virtual_user_id: owner_runtime.id,
            provider: "gitlab".to_string(),
            connection_type: "oauth".to_string(),
            provider_user_id: Some("owner-gitlab".to_string()),
            provider_username: Some("owner".to_string()),
            access_token_encrypted: None,
            refresh_token_encrypted: None,
            scopes: Some("api".to_string()),
            expires_at: None,
            installation_id: None,
            provider_metadata: None,
        })
        .await
        .expect("Failed to insert owner OAuth connection");
    backend
        .upsert_virtual_user_connection(CreateVirtualUserConnectionRow {
            virtual_user_id: owner_runtime.id,
            provider: "github".to_string(),
            connection_type: "oauth".to_string(),
            provider_user_id: Some("owner-github".to_string()),
            provider_username: Some("owner".to_string()),
            access_token_encrypted: None,
            refresh_token_encrypted: None,
            scopes: Some("contents:read".to_string()),
            expires_at: None,
            installation_id: Some(111),
            provider_metadata: None,
        })
        .await
        .expect("Failed to insert owner GitHub installation");

    assert_eq!(
        backend
            .get_connection_token_for_session(session.id, "gitlab")
            .await
            .expect("Failed to resolve owner connection token"),
        None
    );
    assert_eq!(
        backend
            .get_connection_metadata_for_session(session.id, "gitlab")
            .await
            .expect("Failed to resolve owner connection metadata"),
        None
    );
    assert_eq!(
        backend
            .get_connection_user_for_session(session.id, "gitlab")
            .await
            .expect("Failed to resolve owner connection user"),
        None
    );
    assert_eq!(
        backend
            .get_installation_id_for_session(session.id, "github")
            .await
            .expect("Failed to resolve owner GitHub installation"),
        None
    );
}

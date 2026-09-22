//! The worker-facing platform-command RPC: owner resolution and the capability gate.
//!
//! Its own file rather than another block in `tests.rs`, which is on the size
//! ratchet's debt list and may not grow.

use super::tests::test_worker_service;
use super::*;

#[tokio::test]
async fn platform_command_surface_uses_session_owner_and_org() {
    use crate::storage::models::{CreateSessionRow, CreateUserRow};

    let service = test_worker_service().await;
    let user = service
        .db
        .create_user(CreateUserRow {
            email: "platform-surface-member@example.com".to_string(),
            name: "Platform Surface Member".to_string(),
            avatar_url: None,
            external_id: None,
            roles: vec![],
            password_hash: None,
            email_verified: true,
            auth_provider: Some("test".to_string()),
            auth_provider_id: None,
        })
        .await
        .expect("create user");
    service
        .db
        .ensure_membership(user.id, everruns_core::DEFAULT_ORG_ID, "member")
        .await
        .expect("ensure membership");
    let session = service
        .db
        .create_session(CreateSessionRow {
            source: everruns_platform::SessionSource::Api,
            workspace_id: None,
            org_id: everruns_core::DEFAULT_ORG_ID,
            app_id: None,
            endpoint_id: None,
            harness_id: None,
            agent_id: None,
            agent_identity_id: None,
            agent_version_id: None,
            agent_config_hash: None,
            owner_principal_id: everruns_provider::typed_id::PrincipalId::from_seed(2),
            resolved_owner_user_id: Some(user.id),
            title: Some("platform surface".to_string()),
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([{ "ref": "platform" }]),
            tools: serde_json::json!([]),
            mcp_servers: serde_json::json!({}),
            system_prompt: None,
            initial_files: serde_json::json!([]),
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
        .expect("create session");

    let response = service
        .invoke_platform_command_surface(Request::new(InvokePlatformCommandSurfaceRequest {
            session_id: Some(proto::Uuid {
                value: session.id.uuid().to_string(),
            }),
            org_id: session.org_id,
            operation: PlatformCommandSurfaceOperation::Discover as i32,
            arguments_json: serde_json::to_vec(&serde_json::json!({
                "query": "models"
            }))
            .unwrap(),
        }))
        .await
        .expect("discover succeeds")
        .into_inner();
    let proto::invoke_platform_command_surface_response::Result::Output(output) =
        response.result.expect("discover result")
    else {
        panic!("expected discover output");
    };
    assert!(output.contains("list_models"));

    let session_without_platform = service
        .db
        .create_session(CreateSessionRow {
            source: everruns_platform::SessionSource::Api,
            workspace_id: None,
            org_id: everruns_core::DEFAULT_ORG_ID,
            app_id: None,
            endpoint_id: None,
            harness_id: None,
            agent_id: None,
            agent_identity_id: None,
            agent_version_id: None,
            agent_config_hash: None,
            owner_principal_id: everruns_provider::typed_id::PrincipalId::from_seed(2),
            resolved_owner_user_id: Some(user.id),
            title: Some("no platform surface".to_string()),
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
            mcp_servers: serde_json::json!({}),
            system_prompt: None,
            initial_files: serde_json::json!([]),
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
        .expect("create session without platform");
    let missing_capability = service
        .invoke_platform_command_surface(Request::new(InvokePlatformCommandSurfaceRequest {
            session_id: Some(proto::Uuid {
                value: session_without_platform.id.uuid().to_string(),
            }),
            org_id: session_without_platform.org_id,
            operation: PlatformCommandSurfaceOperation::Discover as i32,
            arguments_json: br#"{"query":"models"}"#.to_vec(),
        }))
        .await
        .expect_err("session without platform capability must be denied");
    assert_eq!(missing_capability.code(), tonic::Code::PermissionDenied);

    // `platform` reached only by dependency expansion still opens the gate.
    // A declarative capability carries its definition in its config and needs
    // no registry entry, so `platform` appears in the resolved set without
    // appearing in any layer's declared list. A gate that tested the declared
    // lists would deny this session while its shell carries the `everruns`
    // builtin -- the worker installs the catalog from the same resolved set.
    let session_via_dependency = service
        .db
        .create_session(CreateSessionRow {
            source: everruns_platform::SessionSource::Api,
            workspace_id: None,
            org_id: everruns_core::DEFAULT_ORG_ID,
            app_id: None,
            endpoint_id: None,
            harness_id: None,
            agent_id: None,
            agent_identity_id: None,
            agent_version_id: None,
            agent_config_hash: None,
            owner_principal_id: everruns_provider::typed_id::PrincipalId::from_seed(3),
            resolved_owner_user_id: Some(user.id),
            title: Some("platform via dependency".to_string()),
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([{
                "ref": "declarative:platform_pack",
                "config": {
                    "name": "platform_pack",
                    "description": "Bundle that pulls in the platform surface.",
                    "dependencies": ["platform"],
                },
            }]),
            tools: serde_json::json!([]),
            mcp_servers: serde_json::json!({}),
            system_prompt: None,
            initial_files: serde_json::json!([]),
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
        .expect("create session with platform via dependency");
    let via_dependency = service
        .invoke_platform_command_surface(Request::new(InvokePlatformCommandSurfaceRequest {
            session_id: Some(proto::Uuid {
                value: session_via_dependency.id.uuid().to_string(),
            }),
            org_id: session_via_dependency.org_id,
            operation: PlatformCommandSurfaceOperation::Discover as i32,
            arguments_json: br#"{"query":"models"}"#.to_vec(),
        }))
        .await;
    assert!(
        !matches!(
            via_dependency.as_ref().err().map(|status| status.code()),
            Some(tonic::Code::PermissionDenied)
        ),
        "dependency-expanded platform must not be denied: {:?}",
        via_dependency.err()
    );

    for (query, expected) in [
        // A tree command advertises its spelling and defers its flags to
        // `--help`, so discovery names the command, not its parameters.
        ("create agent", ["create_agent", "agents create --help"]),
        (
            "create agent trigger",
            ["create_agent_trigger", "cron_expression"],
        ),
    ] {
        let response = service
            .invoke_platform_command_surface(Request::new(InvokePlatformCommandSurfaceRequest {
                session_id: Some(proto::Uuid {
                    value: session.id.uuid().to_string(),
                }),
                org_id: session.org_id,
                operation: PlatformCommandSurfaceOperation::Discover as i32,
                arguments_json: serde_json::to_vec(&serde_json::json!({ "query": query })).unwrap(),
            }))
            .await
            .expect("command discovery succeeds")
            .into_inner();
        let proto::invoke_platform_command_surface_response::Result::Output(output) =
            response.result.expect("discover result")
        else {
            panic!("expected discover output");
        };
        for needle in expected {
            assert!(
                output.contains(needle),
                "{query} discovery omitted {needle}"
            );
        }
    }

    let denied = service
        .invoke_platform_command_surface(Request::new(InvokePlatformCommandSurfaceRequest {
            session_id: Some(proto::Uuid {
                value: session.id.uuid().to_string(),
            }),
            org_id: session.org_id,
            operation: PlatformCommandSurfaceOperation::Execute as i32,
            arguments_json: serde_json::to_vec(&serde_json::json!({
                "commands": "create_harness --name forbidden"
            }))
            .unwrap(),
        }))
        .await
        .expect("authorization denial is a tool result")
        .into_inner();
    let proto::invoke_platform_command_surface_response::Result::Error(error) =
        denied.result.expect("execute result")
    else {
        panic!("member mutation must be denied");
    };
    assert!(
        error.contains("forbidden") || error.contains("Access denied"),
        "unexpected authorization error: {error}"
    );

    let foreign = service
        .invoke_platform_command_surface(Request::new(InvokePlatformCommandSurfaceRequest {
            session_id: Some(proto::Uuid {
                value: session.id.uuid().to_string(),
            }),
            org_id: session.org_id + 1,
            operation: PlatformCommandSurfaceOperation::Discover as i32,
            arguments_json: br#"{"query":"models"}"#.to_vec(),
        }))
        .await
        .expect_err("cross-org session lookup must fail");
    assert_eq!(foreign.code(), tonic::Code::NotFound);
}

//! Managed sandbox credential authorization regressions.

use super::*;

#[tokio::test]
async fn sandbox_credentials_require_a_pinned_session_binding() {
    use everruns_contracts::session_sandbox::{
        SessionSandboxContext, SessionSandboxCredential, SessionSandboxCredentialSource,
    };
    let fixture = mcp_setup(ATTENDED, true, true).await;
    let mut context = everruns_core::tool_context::ToolContext::new(fixture.session_id);
    context.connection_resolver = Some(Arc::new(resolver_for(&fixture)));
    // Both grants exist, but neither a caller-controlled owner nor the session
    // owner's grant is authority without a server-pinned Sandbox Template.
    for (source, owner) in [
        (SessionSandboxCredentialSource::SessionUser, fixture.user_id),
        (
            SessionSandboxCredentialSource::Agent,
            fixture.identity_id.uuid(),
        ),
    ] {
        let token = context
            .sandbox_connection_token(
                &fixture.provider,
                &SessionSandboxCredential {
                    source,
                    virtual_user_id: Some(owner),
                    connection_id: None,
                },
            )
            .await
            .unwrap();
        assert!(
            token.is_none(),
            "untrusted sandbox binding decrypted a private grant"
        );
    }
}

#[tokio::test]
async fn sandbox_credentials_reject_private_and_cross_org_owner_substitution() {
    use everruns_contracts::session_sandbox::{
        SessionSandboxContext, SessionSandboxCredential, SessionSandboxCredentialSource,
    };
    for (source, owner_is_agent, expected_token) in [
        (
            SessionSandboxCredentialSource::SessionUser,
            false,
            "user-token",
        ),
        (
            SessionSandboxCredentialSource::Agent,
            true,
            "identity-token",
        ),
    ] {
        let fixture = mcp_setup(ATTENDED, true, true).await;
        let owner = if owner_is_agent {
            fixture.identity_id.uuid()
        } else {
            fixture.user_id
        };
        let credential = SessionSandboxCredential {
            source,
            virtual_user_id: Some(owner),
            connection_id: None,
        };
        pin_sandbox_binding(&fixture, credential.clone()).await;
        let mut context = everruns_core::tool_context::ToolContext::new(fixture.session_id);
        context.connection_resolver = Some(Arc::new(resolver_for(&fixture)));
        assert_eq!(
            context
                .sandbox_connection_token(&fixture.provider, &credential)
                .await
                .unwrap(),
            Some(expected_token.to_string()),
        );
        let db = Arc::new(fixture.db.clone());
        let event_service = crate::services::EventService::with_listeners(
            db.clone(),
            crate::EventDelivery::in_memory(),
            vec![],
        );
        let worker_service = crate::worker_link::grpc_service::WorkerServiceImpl::new(
            event_service,
            db,
            Some(Arc::new(fixture.encryption.clone())),
            None,
            crate::oss_host_composition_for_grade(everruns_core::DeploymentGrade::Dev),
        );
        let rpc_request = |binding: &SessionSandboxCredential| {
            tonic::Request::new(
                everruns_internal_protocol::proto::GetSandboxConnectionTokenRequest {
                    session_id: Some(everruns_internal_protocol::proto::Uuid {
                        value: fixture.session_id.uuid().to_string(),
                    }),
                    provider: fixture.provider.clone(),
                    credential_json: serde_json::to_string(binding).unwrap(),
                },
            )
        };
        assert_eq!(
            worker_service
                .handle_get_sandbox_connection_token(rpc_request(&credential))
                .await
                .unwrap()
                .into_inner()
                .token,
            Some(expected_token.to_string()),
        );
        for org_id in [DEFAULT_ORG_ID, 2] {
            let victim = VirtualUserId::from_uuid(Uuid::now_v7());
            fixture
                .db
                .create_virtual_user(crate::storage::CreateVirtualUserRow {
                    org_id,
                    id: victim,
                    usage: "end_user".into(),
                    name: "Victim".into(),
                    description: None,
                    avatar_url: None,
                    locale: None,
                    timezone: None,
                })
                .await
                .unwrap();
            fixture
                .db
                .upsert_virtual_user_connection(CreateVirtualUserConnectionRow {
                    virtual_user_id: victim,
                    provider: fixture.provider.clone(),
                    connection_type: "api_key".into(),
                    provider_user_id: None,
                    provider_username: None,
                    access_token_encrypted: Some(
                        fixture.encryption.encrypt_string("victim-key").unwrap(),
                    ),
                    refresh_token_encrypted: None,
                    scopes: None,
                    expires_at: None,
                    installation_id: None,
                    provider_metadata: None,
                })
                .await
                .unwrap();
            let forged = SessionSandboxCredential {
                virtual_user_id: Some(victim.uuid()),
                ..credential.clone()
            };
            assert!(
                worker_service
                    .handle_get_sandbox_connection_token(rpc_request(&forged))
                    .await
                    .unwrap()
                    .into_inner()
                    .token
                    .is_none()
            );
            assert!(
                context
                    .sandbox_connection_token(&fixture.provider, &forged)
                    .await
                    .unwrap()
                    .is_none()
            );
        }
        assert!(
            context
                .sandbox_connection_token("other-provider", &credential)
                .await
                .unwrap()
                .is_none()
        );
    }
}

async fn pin_sandbox_binding(
    fixture: &McpFixture,
    credential: everruns_contracts::session_sandbox::SessionSandboxCredential,
) {
    let template: crate::domains::sandbox_templates::record::SandboxTemplateSpec =
        serde_json::from_value(
            serde_json::json!({"target": {"kind": "managed", "provider": "daytona"}}),
        )
        .unwrap();
    let mut spec = crate::domains::sandbox_templates::resolution::resolve_spec(&template).unwrap();
    spec.target.provider = Some(fixture.provider.clone());
    spec.target.credential = credential;
    fixture
        .db
        .pin_primary_sandbox(fixture.session_id, "managed", &spec)
        .await
        .unwrap();
}

#[tokio::test]
async fn sandbox_organization_credentials_keep_the_exact_account_and_tenant() {
    use everruns_contracts::session_sandbox::{
        SessionSandboxCredential, SessionSandboxCredentialSource,
    };
    for org_id in [DEFAULT_ORG_ID, 2] {
        let fixture = mcp_setup(ATTENDED, true, true).await;
        let connection = fixture
            .db
            .create_organization_connection(crate::storage::CreateOrganizationConnectionRow {
                org_id,
                name: "Sandbox account".into(),
                provider: fixture.provider.clone(),
                access_token_encrypted: fixture.encryption.encrypt_string("org-key").unwrap(),
                provider_username: None,
                provider_metadata: None,
            })
            .await
            .unwrap();
        let credential = SessionSandboxCredential {
            source: SessionSandboxCredentialSource::Organization,
            virtual_user_id: Some(connection.virtual_user_id.uuid()),
            connection_id: Some(connection.id),
        };
        pin_sandbox_binding(&fixture, credential.clone()).await;
        let resolver = resolver_for(&fixture);
        assert_eq!(
            resolver
                .get_sandbox_connection_token(fixture.session_id, &fixture.provider, &credential)
                .await
                .unwrap(),
            (org_id == DEFAULT_ORG_ID).then(|| "org-key".to_string()),
        );
        let substituted = SessionSandboxCredential {
            connection_id: Some(Uuid::now_v7()),
            ..credential
        };
        assert!(
            resolver
                .get_sandbox_connection_token(fixture.session_id, &fixture.provider, &substituted,)
                .await
                .unwrap()
                .is_none()
        );
    }
}

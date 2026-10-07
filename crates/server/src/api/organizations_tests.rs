use super::*;
use crate::auth::backend::AuthBackend;
use crate::auth::config::{AuthConfig, AuthMode, JwtConfig};
use crate::auth::middleware::{AuthError, AuthMethod};
use crate::auth::routes::AuthConfigResponse;
use crate::records::OrgMembership;
use axum::body::Body;
use axum::http::Request;
use http_body_util::BodyExt;
use tower::ServiceExt;
use uuid::Uuid;

// ---- Org create policy extension point (EVE-607) ----

/// Minimal auth backend that authenticates every request as one fixed user.
#[derive(Clone)]
struct MockAuthBackend {
    user_id: Uuid,
}

#[async_trait]
impl AuthBackend for MockAuthBackend {
    async fn validate_token(&self, _token: &str) -> Result<AuthUser, AuthError> {
        Ok(AuthUser {
            id: self.user_id,
            email: "test@example.com".to_string(),
            name: "Test User".to_string(),
            roles: vec!["user".to_string()],
            is_platform_user: true,
            auth_method: AuthMethod::Jwt,
            organizations: vec![OrgMembership {
                org_id: DEFAULT_ORG_ID,
                public_id: "org_00000000000000000000000000000001".to_string(),
                name: "Default Organization".to_string(),
                role: OrgRole::Owner,
            }],
        })
    }

    async fn validate_personal_access_token(&self, _token: &str) -> Result<AuthUser, AuthError> {
        Err(AuthError::unauthorized("not supported"))
    }

    fn auth_routes(&self) -> Option<Router> {
        None
    }

    fn auth_config_response(&self) -> AuthConfigResponse {
        AuthConfigResponse {
            mode: "full".to_string(),
            login_origin: None,
            password_auth_enabled: false,
            signup_enabled: false,
            oauth_providers: vec![],
            signup_email_confirm: false,
            captcha: None,
        }
    }
}

/// Policy that always rejects with `403` and a UI-facing message.
struct RejectAllPolicy {
    message: &'static str,
}

#[async_trait]
impl OrgCreatePolicy for RejectAllPolicy {
    async fn check(&self, _ctx: OrgCreateContext<'_>) -> Result<(), OrgCreateRejection> {
        Err(OrgCreateRejection::forbidden(self.message))
    }
}

/// Build a create-org router over an in-memory DB, optionally with a policy.
async fn create_org_app(
    policy: Option<Arc<dyn OrgCreatePolicy>>,
) -> (Router, Arc<StorageBackend>, Uuid) {
    let db = Arc::new(StorageBackend::test_database());
    let user_id = db.create_test_user(Uuid::now_v7()).await;
    let config = AuthConfig {
        mode: AuthMode::Full,
        jwt: JwtConfig {
            secret: "test-secret-for-unit-tests-only".to_string(),
            ..Default::default()
        },
        ..Default::default()
    };
    let auth = AuthState::new(config, Arc::new(MockAuthBackend { user_id }));
    let mut state = AppState::new(db.clone(), auth);
    state.org_create_policy = policy;
    (routes(state), db, user_id)
}

fn create_org_request(name: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/v1/orgs")
        .header("Authorization", "Bearer test-token")
        .header("content-type", "application/json")
        .body(Body::from(format!(r#"{{"name":"{name}"}}"#)))
        .unwrap()
}

fn update_default_org_request(name: &str) -> Request<Body> {
    update_default_org_json_request(format!(r#"{{"name":"{name}"}}"#))
}

fn update_default_org_json_request(body: impl Into<Body>) -> Request<Body> {
    update_org_json_request("org_00000000000000000000000000000001", body)
}

fn update_org_json_request(org_public_id: &str, body: impl Into<Body>) -> Request<Body> {
    Request::builder()
        .method("PATCH")
        .uri(format!("/v1/orgs/{org_public_id}"))
        .header("Authorization", "Bearer test-token")
        .header("content-type", "application/json")
        .body(body.into())
        .unwrap()
}

#[tokio::test]
async fn default_organization_accepts_unchanged_name() {
    let (app, db, user_id) = create_org_app(None).await;
    db.add_organization_member(DEFAULT_ORG_ID, user_id, "owner")
        .await
        .unwrap();

    let response = app
        .oneshot(update_default_org_request("Default Organization"))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn default_organization_still_rejects_renames() {
    let (app, db, user_id) = create_org_app(None).await;
    db.add_organization_member(DEFAULT_ORG_ID, user_id, "owner")
        .await
        .unwrap();

    let response = app
        .oneshot(update_default_org_request("Renamed"))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn organization_settings_require_database_admin_role() {
    let (app, db, user_id) = create_org_app(None).await;
    db.add_organization_member(DEFAULT_ORG_ID, user_id, "member")
        .await
        .unwrap();

    let response = app
        .oneshot(update_default_org_json_request(
            r#"{"default_model_id":"model_01933b5a00007000800000000000030b"}"#,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        json["detail"],
        "Only organization admins can update organization settings"
    );
}

#[tokio::test]
async fn organization_name_update_requires_database_admin_role() {
    use crate::storage::models::CreateOrganizationRow;

    let (app, db, user_id) = create_org_app(None).await;
    let public_id = generate_org_public_id();
    let org = db
        .create_organization(CreateOrganizationRow {
            public_id: public_id.clone(),
            name: "Original Name".to_string(),
            created_by: None,
        })
        .await
        .unwrap();
    db.add_organization_member(org.org_id, user_id, "member")
        .await
        .unwrap();

    let response = app
        .oneshot(update_org_json_request(
            &public_id,
            r#"{"name":"Member Renamed"}"#,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let persisted = db.get_organization(org.org_id).await.unwrap().unwrap();
    assert_eq!(persisted.name, "Original Name");
}

#[tokio::test]
async fn organization_rejects_stale_default_model() {
    let (app, db, user_id) = create_org_app(None).await;
    db.add_organization_member(DEFAULT_ORG_ID, user_id, "owner")
        .await
        .unwrap();

    let response = app
        .oneshot(update_default_org_json_request(
            r#"{"default_model_id":"model_01933b5a00007000800000000000030b"}"#,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["detail"], "Model not found");
}

#[tokio::test]
async fn organization_rejects_personal_model_and_service_defaults() {
    use crate::storage::models::{CreateModelRow, CreateProviderRow};
    let (app, db, user_id) = create_org_app(None).await;
    db.add_organization_member(DEFAULT_ORG_ID, user_id, "owner")
        .await
        .unwrap();
    let provider = db
        .create_provider(
            DEFAULT_ORG_ID,
            CreateProviderRow {
                name: "Personal ChatGPT".into(),
                provider_type: "chatgpt".into(),
                base_url: None,
                api_key_encrypted: None,
                settings: Some(
                    serde_json::json!({"chatgpt":{"owner_user_id":user_id.to_string()}}),
                ),
            },
        )
        .await
        .unwrap();
    let model = db
        .create_model(
            DEFAULT_ORG_ID,
            CreateModelRow {
                provider_id: provider.id,
                model_id: "gpt-test".into(),
                display_name: "Test".into(),
                capabilities: vec![],
                is_favorite: false,
                enabled: true,
                source: "discovered".into(),
                provider_metadata: None,
            },
        )
        .await
        .unwrap();
    for body in [
        serde_json::json!({"default_model_id":model.id}),
        serde_json::json!({"default_provider_per_service":{"realtime":provider.id}}),
    ] {
        let response = app
            .clone()
            .oneshot(update_default_org_json_request(body.to_string()))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let error: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            error["detail"],
            "Personal providers cannot be organization defaults"
        );
    }
}

#[tokio::test]
async fn organization_chooses_who_answers_system_decisions() {
    let (app, db, user_id) = create_org_app(None).await;
    db.add_organization_member(DEFAULT_ORG_ID, user_id, "owner")
        .await
        .unwrap();
    let send = |body: serde_json::Value| {
        let app = app.clone();
        async move {
            let response = app
                .oneshot(update_default_org_json_request(body.to_string()))
                .await
                .unwrap();
            let status = response.status();
            let bytes = response.into_body().collect().await.unwrap().to_bytes();
            let body = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
            (status, body)
        }
    };
    // Defaults to the deployment; an unrelated update leaves it alone.
    let (status, org) = send(serde_json::json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(org["system_decisions"], "deployment");
    let (status, org) = send(serde_json::json!({"system_decisions": "organization"})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(org["system_decisions"], "organization");
    let (_, org) = send(serde_json::json!({})).await;
    assert_eq!(org["system_decisions"], "organization");
    let (status, _) = send(serde_json::json!({"system_decisions": "tenant"})).await;
    assert!(status.is_client_error());
    let (_, org) = send(serde_json::json!({"system_decisions": "deployment"})).await;
    assert_eq!(org["system_decisions"], "deployment");
}

#[tokio::test]
async fn create_organization_rejects_empty_name() {
    let (app, db, user_id) = create_org_app(None).await;

    let response = app.oneshot(create_org_request("")).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["detail"], "Organization name cannot be empty");
    assert_eq!(
        db.count_user_created_organizations(user_id).await.unwrap(),
        0
    );
}

#[tokio::test]
async fn create_organization_rejects_whitespace_only_name() {
    let (app, db, user_id) = create_org_app(None).await;

    let response = app.oneshot(create_org_request("   ")).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["detail"], "Organization name cannot be empty");
    assert_eq!(
        db.count_user_created_organizations(user_id).await.unwrap(),
        0
    );
}

#[tokio::test]
async fn update_organization_rejects_whitespace_only_name() {
    use crate::storage::models::CreateOrganizationRow;

    let (app, db, user_id) = create_org_app(None).await;
    let public_id = generate_org_public_id();
    let org = db
        .create_organization(CreateOrganizationRow {
            public_id: public_id.clone(),
            name: "Original Name".to_string(),
            created_by: None,
        })
        .await
        .unwrap();
    db.add_organization_member(org.org_id, user_id, "owner")
        .await
        .unwrap();

    let response = app
        .oneshot(update_org_json_request(&public_id, r#"{"name":"   "}"#))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let persisted = db.get_organization(org.org_id).await.unwrap().unwrap();
    assert_eq!(persisted.name, "Original Name");
}

#[tokio::test]
async fn create_organization_succeeds_without_policy() {
    // Default OSS behavior: no policy registered, creation proceeds.
    let (app, db, user_id) = create_org_app(None).await;

    let response = app.oneshot(create_org_request("Acme Corp")).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    // The org row and owner membership were persisted.
    let count = db.count_user_created_organizations(user_id).await.unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn org_create_policy_rejects_before_db_write() {
    let policy: Arc<dyn OrgCreatePolicy> = Arc::new(RejectAllPolicy {
        message: "Please verify your email address before continuing.",
    });
    let (app, db, user_id) = create_org_app(Some(policy)).await;

    let response = app.oneshot(create_org_request("Acme Corp")).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        json["detail"],
        "Please verify your email address before continuing."
    );

    // Fail-closed: no org or membership row was written.
    let count = db.count_user_created_organizations(user_id).await.unwrap();
    assert_eq!(count, 0);
    assert!(
        db.list_user_organizations(user_id)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn org_create_policy_allows_creation() {
    // A policy that returns Ok must not change default behavior.
    struct AllowPolicy;
    #[async_trait]
    impl OrgCreatePolicy for AllowPolicy {
        async fn check(&self, _ctx: OrgCreateContext<'_>) -> Result<(), OrgCreateRejection> {
            Ok(())
        }
    }
    let policy: Arc<dyn OrgCreatePolicy> = Arc::new(AllowPolicy);
    let (app, db, user_id) = create_org_app(Some(policy)).await;

    let response = app.oneshot(create_org_request("Acme Corp")).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let count = db.count_user_created_organizations(user_id).await.unwrap();
    assert_eq!(count, 1);
}

// ------------------------------------------------------------------------
// Post-create org initializers (EVE-811)
// ------------------------------------------------------------------------

use crate::org_init::{OrgInitContext, OrgInitializer};

/// Build a create-org router with the given post-create initializers.
async fn create_org_app_with_initializers(
    initializers: Vec<Arc<dyn OrgInitializer>>,
) -> (Router, Arc<StorageBackend>, Uuid) {
    let db = Arc::new(StorageBackend::test_database());
    let user_id = db.create_test_user(Uuid::now_v7()).await;
    let config = AuthConfig {
        mode: AuthMode::Full,
        jwt: JwtConfig {
            secret: "test-secret-for-unit-tests-only".to_string(),
            ..Default::default()
        },
        ..Default::default()
    };
    let auth = AuthState::new(config, Arc::new(MockAuthBackend { user_id }));
    let mut state = AppState::new(db.clone(), auth);
    state.org_initializers = initializers;
    (routes(state), db, user_id)
}

#[tokio::test]
async fn org_initializer_runs_after_org_created() {
    use std::sync::Mutex;

    /// (org_id, created_by) recorded by the initializer.
    type SeenOrg = Arc<Mutex<Option<(i64, Option<Uuid>)>>>;

    /// Records the org id and creating user it was invoked with.
    struct RecordingInitializer {
        seen: SeenOrg,
    }
    #[async_trait]
    impl OrgInitializer for RecordingInitializer {
        async fn on_org_created(&self, ctx: OrgInitContext<'_>) -> anyhow::Result<()> {
            // The org exists by the time the initializer runs, and its
            // built-in harnesses are already provisioned.
            let harnesses = ctx.db.list_harnesses(ctx.org_id, None, false).await?;
            assert!(
                !harnesses.is_empty(),
                "harnesses should be provisioned before initializers run"
            );
            *self.seen.lock().unwrap() = Some((ctx.org_id, ctx.created_by));
            Ok(())
        }
    }

    let seen = Arc::new(Mutex::new(None));
    let init: Arc<dyn OrgInitializer> = Arc::new(RecordingInitializer { seen: seen.clone() });
    let (app, db, user_id) = create_org_app_with_initializers(vec![init]).await;

    let response = app.oneshot(create_org_request("Acme Corp")).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    let recorded = seen.lock().unwrap().expect("initializer must have run");
    assert_eq!(
        recorded.1,
        Some(user_id),
        "created_by is the requesting user"
    );
    // The org persisted and the recorded id matches it.
    let orgs = db.list_user_organizations(user_id).await.unwrap();
    assert_eq!(orgs.len(), 1);
    assert_eq!(recorded.0, orgs[0].org_id);
}

#[tokio::test]
async fn required_org_initializer_failure_aborts_and_rolls_back() {
    struct FailingRequired;
    #[async_trait]
    impl OrgInitializer for FailingRequired {
        async fn on_org_created(&self, _ctx: OrgInitContext<'_>) -> anyhow::Result<()> {
            anyhow::bail!("provisioning failed")
        }
        fn name(&self) -> &str {
            "failing-required"
        }
    }

    let init: Arc<dyn OrgInitializer> = Arc::new(FailingRequired);
    let (app, db, user_id) = create_org_app_with_initializers(vec![init]).await;

    let response = app.oneshot(create_org_request("Acme Corp")).await.unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

    // The org was rolled back: no user-owned org survives.
    let count = db.count_user_created_organizations(user_id).await.unwrap();
    assert_eq!(
        count, 0,
        "failed required initializer must roll back the org"
    );
    assert!(
        db.list_user_organizations(user_id)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn optional_org_initializer_failure_is_non_fatal() {
    struct FailingOptional;
    #[async_trait]
    impl OrgInitializer for FailingOptional {
        async fn on_org_created(&self, _ctx: OrgInitContext<'_>) -> anyhow::Result<()> {
            anyhow::bail!("best-effort provisioning failed")
        }
        fn required(&self) -> bool {
            false
        }
        fn name(&self) -> &str {
            "failing-optional"
        }
    }

    let init: Arc<dyn OrgInitializer> = Arc::new(FailingOptional);
    let (app, db, user_id) = create_org_app_with_initializers(vec![init]).await;

    let response = app.oneshot(create_org_request("Acme Corp")).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    // Org still created despite the optional initializer failing.
    let count = db.count_user_created_organizations(user_id).await.unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn mark_org_onboarding_complete_is_idempotent() {
    use crate::storage::models::CreateOrganizationRow;

    let db = StorageBackend::test_database();
    let creator = db.create_test_user(Uuid::now_v7()).await;
    let org = db
        .create_organization(CreateOrganizationRow {
            public_id: generate_org_public_id(),
            name: "New Org".to_string(),
            created_by: Some(creator),
        })
        .await
        .unwrap();
    // A freshly created (user-owned) org starts un-onboarded.
    assert!(org.onboarding_completed_at.is_none());

    db.mark_org_onboarding_complete(org.org_id).await.unwrap();
    let after = db.get_organization(org.org_id).await.unwrap().unwrap();
    let first = after.onboarding_completed_at.expect("timestamp set");

    // A second call must be a no-op — the completion time never moves.
    db.mark_org_onboarding_complete(org.org_id).await.unwrap();
    let after2 = db.get_organization(org.org_id).await.unwrap().unwrap();
    assert_eq!(after2.onboarding_completed_at, Some(first));
}

#[tokio::test]
async fn seeded_org_is_created_already_onboarded() {
    use crate::storage::models::CreateOrganizationRow;

    let db = StorageBackend::test_database();

    // The pre-seeded default org is already onboarded.
    let default_org = db.get_organization(DEFAULT_ORG_ID).await.unwrap().unwrap();
    assert!(default_org.onboarding_completed_at.is_some());

    // Orgs created via the seeding path (`create_organization_with_id`) are
    // likewise created already-complete.
    let seeded = db
        .create_organization_with_id(
            4242,
            CreateOrganizationRow {
                public_id: generate_org_public_id(),
                name: "Seeded".to_string(),
                created_by: None,
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert!(seeded.onboarding_completed_at.is_some());
}

#[tokio::test]
async fn complete_org_onboarding_marks_and_is_idempotent() {
    let (app, db, _user_id) = create_org_app(None).await;

    // Create an org — the caller becomes owner and onboarding starts NULL.
    let resp = app
        .clone()
        .oneshot(create_org_request("Acme Corp"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let org_public_id = json["id"].as_str().unwrap().to_string();
    assert!(json["onboarding_completed_at"].is_null());

    let complete_req = |id: &str| {
        Request::builder()
            .method("POST")
            .uri(format!("/v1/orgs/{id}/onboarding/complete"))
            .header("Authorization", "Bearer test-token")
            .body(Body::empty())
            .unwrap()
    };

    let resp = app
        .clone()
        .oneshot(complete_req(&org_public_id))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(!json["onboarding_completed_at"].is_null());

    let first = db
        .get_organization_by_public_id(&org_public_id)
        .await
        .unwrap()
        .unwrap()
        .onboarding_completed_at
        .expect("marked complete");

    // Idempotent: a repeat call still returns 200 and never moves the time.
    let resp = app
        .clone()
        .oneshot(complete_req(&org_public_id))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let again = db
        .get_organization_by_public_id(&org_public_id)
        .await
        .unwrap()
        .unwrap()
        .onboarding_completed_at
        .expect("still complete");
    assert_eq!(first, again);
}

// Trivial derive-only serde round-trips removed; covered by the derive + handler tests.
// (OrganizationResponse field echo and empty-name deserialization were
// struct-literal/derive checks with no custom logic; the handler tests
// above cover the real validation and response paths.)

#[test]
fn test_update_request_partial() {
    let json = r#"{}"#;
    let req: UpdateOrganizationRequest = serde_json::from_str(json).unwrap();
    assert!(req.name.is_none());
    assert!(req.default_model_id.is_none());
    assert!(req.default_harness_id.is_none());
    assert!(req.base_harness_id.is_none());

    let json = r#"{
            "name": "New Name",
            "default_harness_id": "harness_01933b5a000070008000000000000602",
            "base_harness_id": "harness_01933b5a000070008000000000000601"
        }"#;
    let req: UpdateOrganizationRequest = serde_json::from_str(json).unwrap();
    assert_eq!(req.name.unwrap(), "New Name");
    assert!(req.default_harness_id.is_some());
    assert!(req.base_harness_id.is_some());

    let req: UpdateOrganizationRequest =
        serde_json::from_str(r#"{"default_model_id":null}"#).unwrap();
    assert_eq!(req.default_model_id, Some(None));
}

#[tokio::test]
async fn admins_set_and_clear_the_agentid_owner_cap() {
    use crate::storage::models::CreateOrganizationRow;

    let (app, db, user_id) = create_org_app(None).await;
    let public_id = generate_org_public_id();
    let org = db
        .create_organization(CreateOrganizationRow {
            public_id: public_id.clone(),
            name: "AgentID Cap".to_string(),
            created_by: None,
        })
        .await
        .unwrap();
    db.add_organization_member(org.org_id, user_id, "owner")
        .await
        .unwrap();

    let patch = |body: &'static str| {
        app.clone()
            .oneshot(update_org_json_request(&public_id, body))
    };
    let response = patch(r#"{"agentid_agents_per_owner":3}"#).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["agentid_agents_per_owner"], 3);
    assert_eq!(db.agentid_agents_per_owner(org.org_id).await.unwrap(), 3);

    let response = patch(r#"{"agentid_agents_per_owner":-1}"#).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let response = patch(r#"{"agentid_agents_per_owner":null}"#).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        db.agentid_agents_per_owner(org.org_id).await.unwrap(),
        crate::storage::agentid::DEFAULT_AGENTS_PER_OWNER
    );
}

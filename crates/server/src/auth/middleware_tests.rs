use super::*;
use axum::http::HeaderValue;

#[test]
fn test_auth_user_anonymous() {
    let user = AuthUser::anonymous();
    assert_eq!(user.id, ANONYMOUS_USER_ID);
    assert!(!user.id.is_nil(), "anonymous user should not use nil UUID");
    assert!(user.is_admin());
    assert!(user.has_role("admin"));
    assert!(user.is_platform_user);
    assert_eq!(user.auth_method, AuthMethod::None);
    // Anonymous user should belong to default org with Owner role
    assert_eq!(user.organizations.len(), 1);
    assert_eq!(user.organizations[0].org_id, DEFAULT_ORG_ID);
    assert_eq!(user.organizations[0].public_id, DEFAULT_ORG_PUBLIC_ID);
    assert_eq!(user.organizations[0].role, OrgRole::Owner);
}

#[test]
fn test_auth_user_has_role() {
    let user = AuthUser {
        id: Uuid::nil(), // Use nil UUID for testing
        email: "test@example.com".to_string(),
        name: "Test".to_string(),
        roles: vec!["user".to_string(), "editor".to_string()],
        is_platform_user: false,
        auth_method: AuthMethod::Jwt,
        organizations: vec![],
    };

    assert!(user.has_role("user"));
    assert!(user.has_role("editor"));
    assert!(!user.has_role("admin"));
    assert!(!user.is_admin());
}

#[test]
fn test_auth_user_admin() {
    let admin = AuthUser {
        id: Uuid::nil(), // Use nil UUID for testing
        email: "admin@example.com".to_string(),
        name: "Admin".to_string(),
        roles: vec!["admin".to_string()],
        is_platform_user: true,
        auth_method: AuthMethod::Jwt,
        organizations: vec![],
    };

    assert!(admin.is_admin());
    assert!(admin.has_role("admin"));
    assert!(admin.has_role("user")); // Admin has all roles
}

#[test]
fn test_auth_user_org_membership() {
    let user = AuthUser {
        id: Uuid::nil(),
        email: "test@example.com".to_string(),
        name: "Test".to_string(),
        roles: vec!["user".to_string()],
        is_platform_user: false,
        auth_method: AuthMethod::Jwt,
        organizations: vec![
            OrgMembership {
                org_id: 1,
                public_id: "org_000000000000000000000000000000a1".to_string(),
                name: "Org 1".to_string(),
                role: OrgRole::Owner,
            },
            OrgMembership {
                org_id: 2,
                public_id: "org_000000000000000000000000000000a2".to_string(),
                name: "Org 2".to_string(),
                role: OrgRole::Member,
            },
        ],
    };

    assert!(user.is_member_of(1));
    assert!(user.is_member_of(2));
    assert!(!user.is_member_of(3));

    assert!(user.is_member_of_public("org_000000000000000000000000000000a1"));
    assert!(user.is_member_of_public("org_000000000000000000000000000000a2"));
    assert!(!user.is_member_of_public("org_000000000000000000000000000000a3"));

    let org = user.get_org("org_000000000000000000000000000000a1");
    assert!(org.is_some());
    assert_eq!(org.unwrap().name, "Org 1");
}

#[test]
fn test_auth_error() {
    let error = AuthError::unauthorized("Test error");
    assert_eq!(error.status, StatusCode::UNAUTHORIZED);
    assert_eq!(error.error, "Test error");

    let forbidden = AuthError::forbidden("Forbidden");
    assert_eq!(forbidden.status, StatusCode::FORBIDDEN);
}

#[test]
fn test_org_role_hierarchy() {
    assert!(OrgRole::Owner.has_permission(OrgRole::Owner));
    assert!(OrgRole::Owner.has_permission(OrgRole::Admin));
    assert!(OrgRole::Owner.has_permission(OrgRole::Member));
    assert!(OrgRole::Admin.has_permission(OrgRole::Admin));
    assert!(OrgRole::Admin.has_permission(OrgRole::Member));
    assert!(!OrgRole::Admin.has_permission(OrgRole::Owner));
    assert!(OrgRole::Member.has_permission(OrgRole::Member));
    assert!(!OrgRole::Member.has_permission(OrgRole::Admin));
    assert!(!OrgRole::Member.has_permission(OrgRole::Owner));
}

// --- extract_auth_user routing tests ---

use crate::auth::{
    backend::AuthBackend,
    config::{AuthConfig, AuthMode},
    routes::AuthConfigResponse,
};
use async_trait::async_trait;
use axum::Router;
use axum::http::{Request, header};
use std::sync::atomic::{AtomicU32, Ordering};

/// Mock backend that records which validation method was called.
struct MockBackend {
    token_calls: AtomicU32,
    pat_calls: AtomicU32,
}

impl MockBackend {
    fn new() -> Self {
        Self {
            token_calls: AtomicU32::new(0),
            pat_calls: AtomicU32::new(0),
        }
    }
}

#[async_trait]
impl AuthBackend for MockBackend {
    async fn validate_token(&self, _token: &str) -> Result<AuthUser, AuthError> {
        self.token_calls.fetch_add(1, Ordering::SeqCst);
        Ok(AuthUser::anonymous())
    }

    async fn validate_personal_access_token(&self, _key: &str) -> Result<AuthUser, AuthError> {
        self.pat_calls.fetch_add(1, Ordering::SeqCst);
        Ok(AuthUser::anonymous())
    }

    fn auth_routes(&self) -> Option<Router> {
        None
    }

    fn auth_config_response(&self) -> AuthConfigResponse {
        AuthConfigResponse {
            mode: "full".into(),
            login_origin: None,
            password_auth_enabled: false,
            oauth_providers: vec![],
            signup_enabled: false,
            signup_email_confirm: false,
            captcha: None,
        }
    }
}

fn make_auth_state(backend: Arc<MockBackend>) -> AuthState {
    AuthState::new(
        AuthConfig {
            mode: AuthMode::Full,
            ..AuthConfig::default()
        },
        backend,
    )
}

#[tokio::test]
async fn test_resolved_org_none_mode_preserves_anonymous_user_identity() {
    let backend = Arc::new(MockBackend::new());
    let state = AuthState::new(
        AuthConfig {
            mode: AuthMode::None,
            ..AuthConfig::default()
        },
        backend,
    );
    let (mut parts, _body) = Request::builder().body(()).unwrap().into_parts();

    let resolved = ResolvedOrg::from_request_parts(&mut parts, &state)
        .await
        .expect("none mode should resolve the default org");

    assert_eq!(resolved.user_id, Some(ANONYMOUS_USER_ID));
}

#[tokio::test]
async fn test_resolved_org_none_mode_preserves_identity_after_org_switch() {
    let backend = Arc::new(MockBackend::new());
    let db = Arc::new(StorageBackend::test_database());
    let switched_org = db
        .create_organization(CreateOrganizationRow {
            public_id: "org_000000000000000000000000000000a2".to_string(),
            name: "Switched Org".to_string(),
            created_by: None,
        })
        .await
        .expect("create switched org");
    let state = AuthState::new(
        AuthConfig {
            mode: AuthMode::None,
            ..AuthConfig::default()
        },
        backend,
    )
    .with_db(db);
    let (mut parts, _body) = Request::builder()
        .header(
            header::COOKIE,
            format!("{}=org_000000000000000000000000000000a2", ORG_COOKIE_NAME),
        )
        .body(())
        .unwrap()
        .into_parts();

    let resolved = ResolvedOrg::from_request_parts(&mut parts, &state)
        .await
        .expect("none mode should resolve the selected org");

    assert_eq!(resolved.org_id, switched_org.org_id);
    assert_eq!(resolved.user_id, Some(ANONYMOUS_USER_ID));
}

async fn call_extract(auth_header: &str) -> (u32, u32) {
    let backend = Arc::new(MockBackend::new());
    let state = make_auth_state(backend.clone());

    let (mut parts, _body) = Request::builder()
        .header(header::AUTHORIZATION, auth_header)
        .body(())
        .unwrap()
        .into_parts();

    let _ = extract_auth_user(&mut parts, &state).await;

    (
        backend.token_calls.load(Ordering::SeqCst),
        backend.pat_calls.load(Ordering::SeqCst),
    )
}

#[tokio::test]
async fn test_bearer_pat_routes_to_validate_personal_access_token() {
    let (token, pat) = call_extract("Bearer evr_pat_testkey123").await;
    assert_eq!(token, 0, "should not call validate_token");
    assert_eq!(pat, 1, "should call validate_personal_access_token");
}

#[tokio::test]
async fn test_bearer_jwt_routes_to_validate_token() {
    let (token, pat) = call_extract("Bearer eyJhbGciOiJIUzI1NiJ9.x.y").await;
    assert_eq!(token, 1, "should call validate_token");
    assert_eq!(pat, 0, "should not call validate_personal_access_token");
}

#[tokio::test]
async fn test_bearer_case_insensitive() {
    let (token, pat) = call_extract("bearer evr_pat_testkey123").await;
    assert_eq!(
        pat, 1,
        "lowercase bearer should route to personal access token validation"
    );
    assert_eq!(token, 0);

    let (token, pat) = call_extract("BEARER evr_pat_testkey123").await;
    assert_eq!(
        pat, 1,
        "uppercase BEARER should route to personal access token validation"
    );
    assert_eq!(token, 0);
}

#[tokio::test]
async fn test_legacy_bare_pat() {
    let (token, pat) = call_extract("evr_pat_testkey123").await;
    assert_eq!(
        pat, 1,
        "bare evr_ should route to validate_personal_access_token"
    );
    assert_eq!(token, 0);
}

#[tokio::test]
async fn test_legacy_apikey_prefix() {
    let (token, pat) = call_extract("ApiKey evr_pat_testkey123").await;
    assert_eq!(
        pat, 1,
        "ApiKey prefix should route to validate_personal_access_token"
    );
    assert_eq!(token, 0);
}

#[tokio::test]
async fn test_legacy_apikey_prefix_case_insensitive() {
    let (token, pat) = call_extract("apikey evr_pat_testkey123").await;
    assert_eq!(
        pat, 1,
        "lowercase apikey should route to validate_personal_access_token"
    );
    assert_eq!(token, 0);
}

// --- ResolvedOrg JWT + DB tests ---

use crate::storage::{StorageBackend, models::CreateOrganizationRow};

/// Mock backend that returns a JWT user with only the specified orgs.
struct JwtMockBackend {
    user: AuthUser,
}

impl JwtMockBackend {
    fn with_user(user: AuthUser) -> Self {
        Self { user }
    }
}

#[async_trait]
impl AuthBackend for JwtMockBackend {
    async fn validate_token(&self, _token: &str) -> Result<AuthUser, AuthError> {
        Ok(self.user.clone())
    }

    async fn validate_personal_access_token(&self, _key: &str) -> Result<AuthUser, AuthError> {
        Err(AuthError::unauthorized("not supported"))
    }

    fn auth_routes(&self) -> Option<Router> {
        None
    }

    fn auth_config_response(&self) -> AuthConfigResponse {
        AuthConfigResponse {
            mode: "full".into(),
            login_origin: None,
            password_auth_enabled: false,
            oauth_providers: vec![],
            signup_enabled: false,
            signup_email_confirm: false,
            captcha: None,
        }
    }
}

/// Build an AuthState with a JWT mock backend and an in-memory DB.
fn jwt_auth_state_with_db(user: AuthUser) -> (AuthState, Arc<StorageBackend>) {
    let db = Arc::new(StorageBackend::test_database());
    let backend: Arc<dyn AuthBackend> = Arc::new(JwtMockBackend::with_user(user));
    let state = AuthState {
        config: AuthConfig {
            mode: AuthMode::Full,
            ..AuthConfig::default()
        },
        backend,
        permission_resolver: Arc::new(DefaultPermissionResolver),
        db: Some(db.clone()),
        feature_flag_policy: crate::records::FeatureFlagPolicy::current(),
    };
    (state, db)
}

async fn multi_org_jwt_state() -> (AuthState, Uuid) {
    let user_id = Uuid::new_v4();
    let jwt_user = AuthUser {
        id: user_id,
        email: "test@example.com".to_string(),
        name: "Test User".to_string(),
        roles: vec!["user".to_string()],
        is_platform_user: false,
        auth_method: AuthMethod::Jwt,
        organizations: vec![],
    };
    let (state, db) = jwt_auth_state_with_db(jwt_user);
    db.create_test_user(user_id).await;

    for (public_id, name, role) in [
        ("org_000000000000000000000000000000a1", "Org A", "owner"),
        ("org_000000000000000000000000000000a2", "Org B", "member"),
    ] {
        let org = db
            .create_organization(CreateOrganizationRow {
                public_id: public_id.to_string(),
                name: name.to_string(),
                created_by: Some(user_id),
            })
            .await
            .expect("create organization");
        db.add_organization_member(org.org_id, user_id, role)
            .await
            .expect("add organization member");
    }

    db.create_organization(CreateOrganizationRow {
        public_id: "org_000000000000000000000000000000a3".to_string(),
        name: "Org C".to_string(),
        created_by: None,
    })
    .await
    .expect("create nonmember organization");

    (state, user_id)
}

#[tokio::test]
async fn test_resolved_org_jwt_x_org_id_header_overrides_cookie() {
    let (state, user_id) = multi_org_jwt_state().await;
    let (mut parts, _body) = Request::builder()
        .header("x-org-id", "org_000000000000000000000000000000a2")
        .header(
            header::COOKIE,
            format!(
                "access_token=fake-jwt-token; {}=org_000000000000000000000000000000a1",
                ORG_COOKIE_NAME
            ),
        )
        .body(())
        .unwrap()
        .into_parts();

    let resolved = ResolvedOrg::from_request_parts(&mut parts, &state)
        .await
        .expect("X-Org-Id should select org B");

    assert_eq!(resolved.public_id, "org_000000000000000000000000000000a2");
    assert_eq!(resolved.name, "Org B");
    assert_eq!(resolved.user_id, Some(user_id));
    assert_eq!(resolved.role, OrgRole::Member);
}

#[tokio::test]
async fn test_resolved_org_jwt_uses_cookie_without_x_org_id_header() {
    let (state, user_id) = multi_org_jwt_state().await;
    let (mut parts, _body) = Request::builder()
        .header(
            header::COOKIE,
            format!(
                "access_token=fake-jwt-token; {}=org_000000000000000000000000000000a1",
                ORG_COOKIE_NAME
            ),
        )
        .body(())
        .unwrap()
        .into_parts();

    let resolved = ResolvedOrg::from_request_parts(&mut parts, &state)
        .await
        .expect("everruns_org should select org A");

    assert_eq!(resolved.public_id, "org_000000000000000000000000000000a1");
    assert_eq!(resolved.name, "Org A");
    assert_eq!(resolved.user_id, Some(user_id));
    assert_eq!(resolved.role, OrgRole::Owner);
}

#[tokio::test]
async fn test_resolved_org_jwt_unknown_x_org_id_returns_404() {
    let (state, _user_id) = multi_org_jwt_state().await;
    let (mut parts, _body) = Request::builder()
        .header("x-org-id", "org_00000000000000000000000000000099")
        .header(
            header::COOKIE,
            format!(
                "access_token=fake-jwt-token; {}=org_000000000000000000000000000000a1",
                ORG_COOKIE_NAME
            ),
        )
        .body(())
        .unwrap()
        .into_parts();

    let err = ResolvedOrg::from_request_parts(&mut parts, &state)
        .await
        .unwrap_err();

    assert_eq!(err.status, StatusCode::NOT_FOUND);
    assert_eq!(err.error, "Organization not found");
}

#[tokio::test]
async fn test_resolved_org_jwt_invalid_x_org_id_returns_401() {
    let (state, _user_id) = multi_org_jwt_state().await;
    let (mut parts, _body) = Request::builder()
        .header("x-org-id", "not-a-valid-org-id")
        .header(
            header::COOKIE,
            format!(
                "access_token=fake-jwt-token; {}=org_000000000000000000000000000000a1",
                ORG_COOKIE_NAME
            ),
        )
        .body(())
        .unwrap()
        .into_parts();

    let err = ResolvedOrg::from_request_parts(&mut parts, &state)
        .await
        .unwrap_err();

    assert_eq!(err.status, StatusCode::UNAUTHORIZED);
    assert_eq!(err.error, "Invalid organization ID format");
}

#[tokio::test]
async fn test_resolved_org_jwt_non_utf8_x_org_id_returns_401() {
    let (state, _user_id) = multi_org_jwt_state().await;
    let (mut parts, _body) = Request::builder()
        .header("x-org-id", HeaderValue::from_bytes(b"\xff").unwrap())
        .header(
            header::COOKIE,
            format!(
                "access_token=fake-jwt-token; {}=org_000000000000000000000000000000a1",
                ORG_COOKIE_NAME
            ),
        )
        .body(())
        .unwrap()
        .into_parts();

    let err = ResolvedOrg::from_request_parts(&mut parts, &state)
        .await
        .unwrap_err();

    assert_eq!(err.status, StatusCode::UNAUTHORIZED);
    assert_eq!(err.error, "Invalid organization ID format");
}

#[tokio::test]
async fn test_resolved_org_jwt_nonmember_x_org_id_overrides_cookie() {
    let (state, _user_id) = multi_org_jwt_state().await;
    let (mut parts, _body) = Request::builder()
        .header("x-org-id", "org_000000000000000000000000000000a3")
        .header(
            header::COOKIE,
            format!(
                "access_token=fake-jwt-token; {}=org_000000000000000000000000000000a1",
                ORG_COOKIE_NAME
            ),
        )
        .body(())
        .unwrap()
        .into_parts();

    let err = ResolvedOrg::from_request_parts(&mut parts, &state)
        .await
        .unwrap_err();

    assert_eq!(err.status, StatusCode::NOT_FOUND);
    assert_eq!(err.error, "Organization not found");
}

#[tokio::test]
async fn test_resolved_org_jwt_uses_db_not_jwt_orgs() {
    // User's JWT only knows about org_a. org_b exists in DB with membership
    // but is NOT in the JWT. Cookie is set to org_b.
    // Expected: ResolvedOrg resolves to org_b via DB lookup.

    let user_id = Uuid::new_v4();
    let jwt_user = AuthUser {
        id: user_id,
        email: "test@example.com".to_string(),
        name: "Test User".to_string(),
        roles: vec!["user".to_string()],
        is_platform_user: false,
        auth_method: AuthMethod::Jwt,
        organizations: vec![OrgMembership {
            org_id: 1,
            public_id: "org_000000000000000000000000000000a1".to_string(),
            name: "Org A".to_string(),
            role: OrgRole::Owner,
        }],
    };

    let (state, db) = jwt_auth_state_with_db(jwt_user);
    db.create_test_user(user_id).await;

    // Seed org_b in the DB and add user membership
    let org_b = db
        .create_organization(CreateOrganizationRow {
            public_id: "org_000000000000000000000000000000a2".to_string(),
            name: "Org B".to_string(),
            created_by: Some(user_id),
        })
        .await
        .unwrap();
    db.add_organization_member(org_b.org_id, user_id, "member")
        .await
        .unwrap();

    // Build request with org cookie pointing to org_b and a Bearer token
    let (mut parts, _body) = Request::builder()
        .header(header::AUTHORIZATION, "Bearer fake-jwt-token")
        .header(
            header::COOKIE,
            format!("{}=org_000000000000000000000000000000a2", ORG_COOKIE_NAME),
        )
        .body(())
        .unwrap()
        .into_parts();

    let resolved = ResolvedOrg::from_request_parts(&mut parts, &state)
        .await
        .expect("should resolve org_b from DB");

    assert_eq!(resolved.public_id, "org_000000000000000000000000000000a2");
    assert_eq!(resolved.name, "Org B");
    assert_eq!(resolved.org_id, org_b.org_id);
    assert_eq!(resolved.user_id, Some(user_id));
    assert_eq!(resolved.role, OrgRole::Member);
}

#[tokio::test]
async fn test_resolved_org_jwt_db_no_membership_returns_404() {
    // User's JWT only knows about org_a. org_c exists in DB but user is NOT
    // a member. Cookie is set to org_c.
    // Expected: 404

    let user_id = Uuid::new_v4();
    let jwt_user = AuthUser {
        id: user_id,
        email: "test@example.com".to_string(),
        name: "Test User".to_string(),
        roles: vec!["user".to_string()],
        is_platform_user: false,
        auth_method: AuthMethod::Jwt,
        organizations: vec![OrgMembership {
            org_id: 1,
            public_id: "org_000000000000000000000000000000a1".to_string(),
            name: "Org A".to_string(),
            role: OrgRole::Owner,
        }],
    };

    let (state, db) = jwt_auth_state_with_db(jwt_user);

    // Seed org_c in the DB but do NOT add user membership
    db.create_organization(CreateOrganizationRow {
        public_id: "org_000000000000000000000000000000a3".to_string(),
        name: "Org C".to_string(),
        created_by: None,
    })
    .await
    .unwrap();

    let (mut parts, _body) = Request::builder()
        .header(header::AUTHORIZATION, "Bearer fake-jwt-token")
        .header(
            header::COOKIE,
            format!("{}=org_000000000000000000000000000000a3", ORG_COOKIE_NAME),
        )
        .body(())
        .unwrap()
        .into_parts();

    let err = ResolvedOrg::from_request_parts(&mut parts, &state)
        .await
        .unwrap_err();

    assert_eq!(err.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_resolved_org_jwt_falls_back_to_jwt_when_db_unavailable() {
    // AuthState has no DB (db: None). Cookie points to org_a which IS in
    // the JWT. Expected: falls back to JWT-based validation and succeeds.

    let user_id = Uuid::new_v4();
    let jwt_user = AuthUser {
        id: user_id,
        email: "test@example.com".to_string(),
        name: "Test User".to_string(),
        roles: vec!["user".to_string()],
        is_platform_user: false,
        auth_method: AuthMethod::Jwt,
        organizations: vec![OrgMembership {
            org_id: 1,
            public_id: "org_000000000000000000000000000000a1".to_string(),
            name: "Org A".to_string(),
            role: OrgRole::Owner,
        }],
    };

    let backend: Arc<dyn AuthBackend> = Arc::new(JwtMockBackend::with_user(jwt_user));
    let state = AuthState {
        config: AuthConfig {
            mode: AuthMode::Full,
            ..AuthConfig::default()
        },
        backend,
        permission_resolver: Arc::new(DefaultPermissionResolver),
        db: None, // No DB — forces JWT fallback
        feature_flag_policy: crate::records::FeatureFlagPolicy::current(),
    };

    let (mut parts, _body) = Request::builder()
        .header(header::AUTHORIZATION, "Bearer fake-jwt-token")
        .header(
            header::COOKIE,
            format!("{}=org_000000000000000000000000000000a1", ORG_COOKIE_NAME),
        )
        .body(())
        .unwrap()
        .into_parts();

    let resolved = ResolvedOrg::from_request_parts(&mut parts, &state)
        .await
        .expect("should fall back to JWT org list");

    assert_eq!(resolved.public_id, "org_000000000000000000000000000000a1");
    assert_eq!(resolved.name, "Org A");
    assert_eq!(resolved.user_id, Some(user_id));
    assert_eq!(resolved.role, OrgRole::Owner);
}

// --- ResolvedOrg API key auth tests ---

/// Mock backend that returns an API-key-authenticated user.
struct PersonalAccessTokenMockBackend {
    user: AuthUser,
}

impl PersonalAccessTokenMockBackend {
    fn with_user(user: AuthUser) -> Self {
        Self { user }
    }
}

#[async_trait]
impl AuthBackend for PersonalAccessTokenMockBackend {
    async fn validate_token(&self, _token: &str) -> Result<AuthUser, AuthError> {
        Err(AuthError::unauthorized("not supported"))
    }

    async fn validate_personal_access_token(&self, _key: &str) -> Result<AuthUser, AuthError> {
        Ok(self.user.clone())
    }

    fn auth_routes(&self) -> Option<Router> {
        None
    }

    fn auth_config_response(&self) -> AuthConfigResponse {
        AuthConfigResponse {
            mode: "full".into(),
            login_origin: None,
            password_auth_enabled: false,
            oauth_providers: vec![],
            signup_enabled: false,
            signup_email_confirm: false,
            captcha: None,
        }
    }
}

fn pat_auth_state(user: AuthUser) -> AuthState {
    let backend: Arc<dyn AuthBackend> = Arc::new(PersonalAccessTokenMockBackend::with_user(user));
    AuthState {
        config: AuthConfig {
            mode: AuthMode::Full,
            ..AuthConfig::default()
        },
        backend,
        permission_resolver: Arc::new(DefaultPermissionResolver),
        db: None,
        feature_flag_policy: crate::records::FeatureFlagPolicy::current(),
    }
}

fn multi_org_pat_user() -> (Uuid, AuthUser) {
    let user_id = Uuid::new_v4();
    let user = AuthUser {
        id: user_id,
        email: "apiuser@example.com".to_string(),
        name: "API User".to_string(),
        roles: vec!["user".to_string()],
        is_platform_user: false,
        auth_method: AuthMethod::PersonalAccessToken,
        organizations: vec![
            OrgMembership {
                org_id: 1,
                public_id: "org_000000000000000000000000000000a1".to_string(),
                name: "Org A".to_string(),
                role: OrgRole::Owner,
            },
            OrgMembership {
                org_id: 2,
                public_id: "org_000000000000000000000000000000a2".to_string(),
                name: "Org B".to_string(),
                role: OrgRole::Member,
            },
        ],
    };
    (user_id, user)
}

#[tokio::test]
async fn test_resolved_org_pat_single_org_auto_resolves() {
    let user_id = Uuid::new_v4();
    let user = AuthUser {
        id: user_id,
        email: "apiuser@example.com".to_string(),
        name: "API User".to_string(),
        roles: vec!["user".to_string()],
        is_platform_user: false,
        auth_method: AuthMethod::PersonalAccessToken,
        organizations: vec![OrgMembership {
            org_id: 1,
            public_id: "org_000000000000000000000000000000a1".to_string(),
            name: "Org A".to_string(),
            role: OrgRole::Owner,
        }],
    };

    let state = pat_auth_state(user);

    let (mut parts, _body) = Request::builder()
        .header(header::AUTHORIZATION, "Bearer evr_pat_testkey123")
        .body(())
        .unwrap()
        .into_parts();

    let resolved = ResolvedOrg::from_request_parts(&mut parts, &state)
        .await
        .expect("single-org user should auto-resolve");

    assert_eq!(resolved.public_id, "org_000000000000000000000000000000a1");
    assert_eq!(resolved.name, "Org A");
    assert_eq!(resolved.user_id, Some(user_id));
}

#[tokio::test]
async fn test_resolved_org_pat_multi_org_no_selection_returns_400() {
    let (_user_id, user) = multi_org_pat_user();
    let state = pat_auth_state(user);

    // No X-Org-Id header, no cookie
    let (mut parts, _body) = Request::builder()
        .header(header::AUTHORIZATION, "Bearer evr_pat_testkey123")
        .body(())
        .unwrap()
        .into_parts();

    let err = ResolvedOrg::from_request_parts(&mut parts, &state)
        .await
        .unwrap_err();

    assert_eq!(err.status, StatusCode::BAD_REQUEST);
    assert!(err.error.contains("Multiple organizations"));
}

#[tokio::test]
async fn test_resolved_org_pat_x_org_id_header_selects_org() {
    let (user_id, user) = multi_org_pat_user();
    let state = pat_auth_state(user);

    let (mut parts, _body) = Request::builder()
        .header(header::AUTHORIZATION, "Bearer evr_pat_testkey123")
        .header("x-org-id", "org_000000000000000000000000000000a2")
        .body(())
        .unwrap()
        .into_parts();

    let resolved = ResolvedOrg::from_request_parts(&mut parts, &state)
        .await
        .expect("should resolve org from X-Org-Id header");

    assert_eq!(resolved.public_id, "org_000000000000000000000000000000a2");
    assert_eq!(resolved.name, "Org B");
    assert_eq!(resolved.user_id, Some(user_id));
    assert_eq!(resolved.role, OrgRole::Member);
}

#[tokio::test]
async fn test_resolved_org_pat_non_member_org_returns_404() {
    let (_user_id, user) = multi_org_pat_user();
    let state = pat_auth_state(user);

    // Request org the user is NOT a member of
    let (mut parts, _body) = Request::builder()
        .header(header::AUTHORIZATION, "Bearer evr_pat_testkey123")
        .header("x-org-id", "org_00000000000000000000000000000099")
        .body(())
        .unwrap()
        .into_parts();

    let err = ResolvedOrg::from_request_parts(&mut parts, &state)
        .await
        .unwrap_err();

    assert_eq!(err.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_resolved_org_pat_invalid_format_returns_401() {
    let (_user_id, user) = multi_org_pat_user();
    let state = pat_auth_state(user);

    // Invalid org ID format
    let (mut parts, _body) = Request::builder()
        .header(header::AUTHORIZATION, "Bearer evr_pat_testkey123")
        .header("x-org-id", "not-a-valid-org-id")
        .body(())
        .unwrap()
        .into_parts();

    let err = ResolvedOrg::from_request_parts(&mut parts, &state)
        .await
        .unwrap_err();

    assert_eq!(err.status, StatusCode::UNAUTHORIZED);
    assert!(err.error.contains("Invalid organization ID format"));
}

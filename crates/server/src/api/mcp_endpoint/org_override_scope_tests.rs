use super::*;
use crate::domains::organizations::record::OrgMembership;
use uuid::Uuid;

fn test_auth_user(auth_method: AuthMethod) -> AuthUser {
    AuthUser {
        mcp_client: None,
        id: Uuid::new_v4(),
        email: "user@example.com".to_string(),
        name: "Test User".to_string(),
        roles: vec!["member".to_string()],
        is_platform_user: false,
        auth_method,
        organizations: vec![OrgMembership {
            org_id: 1,
            public_id: "org_default_12345678".to_string(),
            name: "Default".to_string(),
            role: OrgRole::Member,
        }],
        pat_scope: None,
    }
}

fn default_org() -> ResolvedOrg {
    ResolvedOrg {
        org_id: 1,
        public_id: "org_default_12345678".to_string(),
        name: "Default".to_string(),
        user_id: Some(Uuid::new_v4()),
        role: OrgRole::Member,
        is_platform_user: false,
        feature_flags: crate::records::FeatureFlags::default(),
        read_only: false,
    }
}

#[test]
fn api_key_auth_rejects_cross_org_override() {
    let auth_user = test_auth_user(AuthMethod::PersonalAccessToken);
    let result = enforce_org_override_auth_scope("org_other_12345678", &auth_user, &default_org());

    assert!(result.is_err());
}

#[test]
fn api_key_auth_allows_default_org_override() {
    let auth_user = test_auth_user(AuthMethod::PersonalAccessToken);
    let org = default_org();
    let result = enforce_org_override_auth_scope(&org.public_id, &auth_user, &org);

    assert!(result.is_ok());
}

#[test]
fn jwt_auth_allows_cross_org_override() {
    let auth_user = test_auth_user(AuthMethod::Jwt);
    let result = enforce_org_override_auth_scope("org_other_12345678", &auth_user, &default_org());

    assert!(result.is_ok());
}

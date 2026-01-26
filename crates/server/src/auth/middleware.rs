// Authentication middleware and extractors
// Decision: Support both cookie-based (UI) and header-based (API) auth
// Decision: In "none" mode, create an anonymous user context

use axum::{
    Json,
    extract::{FromRef, FromRequestParts},
    http::{StatusCode, header, request::Parts},
    response::{IntoResponse, Response},
};
use axum_extra::extract::CookieJar;
use everruns_core::{DEFAULT_ORG_ID, DEFAULT_ORG_PUBLIC_ID, OrgMembership, validate_org_public_id};
use serde::Serialize;
use std::sync::Arc;
use uuid::Uuid;

use super::{
    api_key::{API_KEY_PREFIX, ValidatedApiKey, hash_api_key, is_valid_api_key_format},
    config::{AuthConfig, AuthMode},
    jwt::JwtService,
};
use crate::storage::StorageBackend;

/// Authentication error
#[derive(Debug, Clone, Serialize)]
pub struct AuthError {
    pub error: String,
    #[serde(skip)]
    pub status: StatusCode,
}

impl AuthError {
    pub fn unauthorized(message: &str) -> Self {
        Self {
            error: message.to_string(),
            status: StatusCode::UNAUTHORIZED,
        }
    }

    pub fn forbidden(message: &str) -> Self {
        Self {
            error: message.to_string(),
            status: StatusCode::FORBIDDEN,
        }
    }
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        (self.status, Json(self)).into_response()
    }
}

/// Authenticated user context extracted from request
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct AuthUser {
    /// User ID
    pub id: Uuid,
    /// User email
    pub email: String,
    /// User name
    pub name: String,
    /// User roles
    pub roles: Vec<String>,
    /// Authentication method used
    pub auth_method: AuthMethod,
    /// Organizations the user belongs to
    pub organizations: Vec<OrgMembership>,
}

impl AuthUser {
    /// Create an anonymous user for no-auth mode
    pub fn anonymous() -> Self {
        Self {
            id: Uuid::nil(),
            email: "anonymous@local".to_string(),
            name: "Anonymous".to_string(),
            roles: vec!["admin".to_string()], // Full access in no-auth mode
            auth_method: AuthMethod::None,
            organizations: vec![OrgMembership {
                org_id: DEFAULT_ORG_ID,
                public_id: DEFAULT_ORG_PUBLIC_ID.to_string(),
                name: "Default Organization".to_string(),
            }],
        }
    }

    /// Check if user is a member of the organization (by internal id)
    #[allow(dead_code)]
    pub fn is_member_of(&self, org_id: i64) -> bool {
        self.organizations.iter().any(|o| o.org_id == org_id)
    }

    /// Check if user is a member of the organization (by public id)
    pub fn is_member_of_public(&self, public_id: &str) -> bool {
        self.organizations.iter().any(|o| o.public_id == public_id)
    }

    /// Get organization by public id
    pub fn get_org(&self, public_id: &str) -> Option<&OrgMembership> {
        self.organizations.iter().find(|o| o.public_id == public_id)
    }

    /// Check if user has a specific role
    #[allow(dead_code)]
    pub fn has_role(&self, role: &str) -> bool {
        self.roles.iter().any(|r| r == role || r == "admin")
    }

    /// Check if user is admin
    #[allow(dead_code)]
    pub fn is_admin(&self) -> bool {
        self.has_role("admin")
    }
}

/// Authentication method used
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthMethod {
    /// No authentication (anonymous)
    None,
    /// JWT access token
    Jwt,
    /// API key
    ApiKey,
}

/// Auth state shared across routes
#[derive(Clone)]
pub struct AuthState {
    pub config: AuthConfig,
    pub jwt_service: Arc<JwtService>,
    pub db: Arc<StorageBackend>,
}

impl AuthState {
    pub fn new(config: AuthConfig, db: Arc<StorageBackend>) -> Self {
        let jwt_service = Arc::new(JwtService::new(config.jwt.clone()));
        Self {
            config,
            jwt_service,
            db,
        }
    }
}

/// Extractor for authenticated user
/// This is required - returns 401 if not authenticated
#[axum::async_trait]
impl<S> FromRequestParts<S> for AuthUser
where
    S: Send + Sync,
    AuthState: FromRef<S>,
{
    type Rejection = AuthError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let auth_state = AuthState::from_ref(state);
        extract_auth_user(parts, &auth_state).await
    }
}

/// Extract authenticated user from request
async fn extract_auth_user(
    parts: &mut Parts,
    auth_state: &AuthState,
) -> Result<AuthUser, AuthError> {
    // In no-auth mode, always return anonymous user
    if auth_state.config.mode == AuthMode::None {
        return Ok(AuthUser::anonymous());
    }

    // Try to extract from Authorization header first
    if let Some(auth_header) = parts.headers.get(header::AUTHORIZATION) {
        let auth_str = auth_header
            .to_str()
            .map_err(|_| AuthError::unauthorized("Invalid authorization header"))?;

        // Check for Bearer token (JWT)
        if let Some(token) = auth_str.strip_prefix("Bearer ") {
            return validate_jwt_token(token, auth_state).await;
        }

        // Check for API key
        if auth_str.starts_with(API_KEY_PREFIX) || auth_str.starts_with("ApiKey ") {
            let api_key = auth_str.strip_prefix("ApiKey ").unwrap_or(auth_str);
            return validate_api_key(api_key, auth_state).await;
        }
    }

    // Try to extract from cookie (for UI)
    let jar = CookieJar::from_headers(&parts.headers);
    if let Some(cookie) = jar.get("access_token") {
        return validate_jwt_token(cookie.value(), auth_state).await;
    }

    // No valid credentials found
    Err(AuthError::unauthorized("Authentication required"))
}

/// Validate JWT token and return user
async fn validate_jwt_token(token: &str, auth_state: &AuthState) -> Result<AuthUser, AuthError> {
    let claims = auth_state
        .jwt_service
        .validate_access_token(token)
        .map_err(|e| {
            tracing::debug!("JWT validation failed: {}", e);
            AuthError::unauthorized("Invalid or expired token")
        })?;

    let user_id = Uuid::parse_str(&claims.sub)
        .map_err(|_| AuthError::unauthorized("Invalid user ID in token"))?;

    // Fetch organization memberships for the user
    let organizations = fetch_user_organizations(&auth_state.db, user_id).await?;

    Ok(AuthUser {
        id: user_id,
        email: claims.email,
        name: claims.name,
        roles: claims.roles,
        auth_method: AuthMethod::Jwt,
        organizations,
    })
}

/// Validate API key and return user
async fn validate_api_key(key: &str, auth_state: &AuthState) -> Result<AuthUser, AuthError> {
    if !is_valid_api_key_format(key) {
        return Err(AuthError::unauthorized("Invalid API key format"));
    }

    let key_hash = hash_api_key(key);

    let api_key_row = auth_state
        .db
        .get_api_key_by_hash(&key_hash)
        .await
        .map_err(|e| {
            tracing::error!("Failed to fetch API key: {}", e);
            AuthError::unauthorized("Failed to validate API key")
        })?
        .ok_or_else(|| AuthError::unauthorized("Invalid API key"))?;

    // Check if expired
    let validated_key = ValidatedApiKey {
        key_id: api_key_row.id,
        user_id: api_key_row.user_id,
        org_id: api_key_row.org_id,
        name: api_key_row.name.clone(),
        scopes: serde_json::from_value(api_key_row.scopes.clone()).unwrap_or_default(),
        expires_at: api_key_row.expires_at,
    };

    if validated_key.is_expired() {
        return Err(AuthError::unauthorized("API key expired"));
    }

    // Update last used timestamp (fire and forget)
    let db = auth_state.db.clone();
    let key_id = api_key_row.id;
    tokio::spawn(async move {
        let _ = db.update_api_key_last_used(key_id).await;
    });

    // Fetch user info
    let user = auth_state
        .db
        .get_user(api_key_row.user_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to fetch user for API key: {}", e);
            AuthError::unauthorized("Failed to validate API key")
        })?
        .ok_or_else(|| AuthError::unauthorized("User not found for API key"))?;

    let roles: Vec<String> = serde_json::from_value(user.roles.clone()).unwrap_or_default();

    // Fetch org for the API key (API key is scoped to single org)
    let org = auth_state
        .db
        .get_organization(api_key_row.org_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to fetch org for API key: {}", e);
            AuthError::unauthorized("Failed to validate API key")
        })?
        .ok_or_else(|| AuthError::unauthorized("Organization not found for API key"))?;

    Ok(AuthUser {
        id: user.id,
        email: user.email,
        name: user.name,
        roles,
        auth_method: AuthMethod::ApiKey,
        organizations: vec![OrgMembership {
            org_id: org.org_id,
            public_id: org.public_id,
            name: org.name,
        }],
    })
}

/// Fetch organization memberships for a user
async fn fetch_user_organizations(
    db: &StorageBackend,
    user_id: Uuid,
) -> Result<Vec<OrgMembership>, AuthError> {
    let org_rows = db.list_user_organizations(user_id).await.map_err(|e| {
        tracing::error!("Failed to fetch user organizations: {}", e);
        AuthError::unauthorized("Failed to fetch organizations")
    })?;

    Ok(org_rows
        .into_iter()
        .map(|row| OrgMembership {
            org_id: row.org_id,
            public_id: row.public_id,
            name: row.name,
        })
        .collect())
}

/// Optional auth extractor - returns None if not authenticated (in auth mode)
/// or anonymous user (in no-auth mode)
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct OptionalAuthUser(pub Option<AuthUser>);

#[axum::async_trait]
impl<S> FromRequestParts<S> for OptionalAuthUser
where
    S: Send + Sync,
    AuthState: FromRef<S>,
{
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let auth_state = AuthState::from_ref(state);

        // In no-auth mode, always return anonymous user
        if auth_state.config.mode == AuthMode::None {
            return Ok(OptionalAuthUser(Some(AuthUser::anonymous())));
        }

        // Try to extract user, but don't fail if not authenticated
        match extract_auth_user(parts, &auth_state).await {
            Ok(user) => Ok(OptionalAuthUser(Some(user))),
            Err(_) => Ok(OptionalAuthUser(None)),
        }
    }
}

/// Require admin role extractor
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct AdminUser(pub AuthUser);

#[axum::async_trait]
impl<S> FromRequestParts<S> for AdminUser
where
    S: Send + Sync,
    AuthState: FromRef<S>,
{
    type Rejection = AuthError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let user = AuthUser::from_request_parts(parts, state).await?;

        if !user.is_admin() {
            return Err(AuthError::forbidden("Admin access required"));
        }

        Ok(AdminUser(user))
    }
}

// ============================================================================
// OrgContext - Organization context extractor
// ============================================================================

/// Organization context extracted from the URL path
///
/// Extracts the org_public_id from the URL path and validates that the
/// authenticated user has access to that organization.
///
/// Usage:
/// ```rust,ignore
/// async fn handler(
///     OrgContext { org_id, public_id, .. }: OrgContext,
///     user: AuthUser,
/// ) -> impl IntoResponse {
///     // org_id is the internal i64 ID for database queries
///     // public_id is the external ID from the URL
/// }
/// ```
#[derive(Debug, Clone)]
pub struct OrgContext {
    /// Internal organization ID (for database queries)
    pub org_id: i64,
    /// External organization public ID (from URL path)
    pub public_id: String,
    /// Organization name
    pub name: String,
}

/// Extract org from URI path directly (doesn't consume Path extractor)
/// The path pattern is: /v1/orgs/{org}/...
fn extract_org_from_uri(uri: &axum::http::Uri) -> Option<String> {
    let path = uri.path();
    let parts: Vec<&str> = path.split('/').collect();
    // Expected: ["", "v1", "orgs", "{org}", ...]
    if parts.len() >= 4 && parts[1] == "v1" && parts[2] == "orgs" {
        Some(parts[3].to_string())
    } else {
        None
    }
}

#[axum::async_trait]
impl<S> FromRequestParts<S> for OrgContext
where
    S: Send + Sync,
    AuthState: FromRef<S>,
{
    type Rejection = AuthError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        // First extract the authenticated user
        let user = AuthUser::from_request_parts(parts, state).await?;

        // Extract org_public_id from URL path directly (doesn't consume Path extractor)
        // This allows handlers to use Path<T> for other path parameters
        let org_public_id = extract_org_from_uri(&parts.uri)
            .ok_or_else(|| AuthError::unauthorized("Missing organization in path"))?;

        // Validate the org_public_id format
        if !validate_org_public_id(&org_public_id) {
            return Err(AuthError::unauthorized("Invalid organization ID format"));
        }

        // Check if user is a member of this organization
        let org = user.get_org(&org_public_id).ok_or_else(|| {
            // Return 404 to prevent enumeration (spec requirement)
            AuthError {
                error: "Organization not found".to_string(),
                status: StatusCode::NOT_FOUND,
            }
        })?;

        Ok(OrgContext {
            org_id: org.org_id,
            public_id: org.public_id.clone(),
            name: org.name.clone(),
        })
    }
}

// ============================================================================
// ResolvedOrg - Organization context from auth (not URL path)
// ============================================================================

/// Header name for org selection in session auth
pub const ORG_HEADER: &str = "X-Org-Id";

/// Organization context resolved from authentication
///
/// Unlike OrgContext which extracts org from URL path, ResolvedOrg derives
/// the organization from the authentication context:
/// - API key auth: org comes from API key (single org in AuthUser.organizations)
/// - Session auth (JWT): org from X-Org-Id header, validated against user membership
///
/// Usage:
/// ```rust,ignore
/// async fn handler(
///     ResolvedOrg { org_id, public_id, .. }: ResolvedOrg,
/// ) -> impl IntoResponse {
///     // org_id is the internal i64 ID for database queries
/// }
/// ```
#[derive(Debug, Clone)]
pub struct ResolvedOrg {
    /// Internal organization ID (for database queries)
    pub org_id: i64,
    /// External organization public ID
    pub public_id: String,
    /// Organization name
    pub name: String,
}

#[axum::async_trait]
impl<S> FromRequestParts<S> for ResolvedOrg
where
    S: Send + Sync,
    AuthState: axum::extract::FromRef<S>,
{
    type Rejection = AuthError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        // First extract the authenticated user
        let user = AuthUser::from_request_parts(parts, state).await?;

        match user.auth_method {
            AuthMethod::ApiKey => {
                // API key auth: user has exactly one org (from API key)
                let org = user
                    .organizations
                    .first()
                    .ok_or_else(|| AuthError::unauthorized("API key has no organization"))?;
                Ok(ResolvedOrg {
                    org_id: org.org_id,
                    public_id: org.public_id.clone(),
                    name: org.name.clone(),
                })
            }
            AuthMethod::Jwt | AuthMethod::None => {
                // Session auth: get org from X-Org-Id header or org_id query param
                // Query param fallback is needed for SSE (EventSource doesn't support headers)
                let org_public_id = parts
                    .headers
                    .get(ORG_HEADER)
                    .and_then(|v| v.to_str().ok())
                    .map(|s| s.to_string())
                    .or_else(|| {
                        // Fallback to query param for SSE (EventSource doesn't support headers)
                        parts.uri.query().and_then(|q| {
                            q.split('&')
                                .filter_map(|part| part.split_once('='))
                                .find(|(k, _)| *k == "org_id")
                                .map(|(_, v)| v.to_string())
                        })
                    })
                    .ok_or_else(|| {
                        AuthError::unauthorized("Missing X-Org-Id header or org_id query param")
                    })?;
                let org_public_id = org_public_id.as_str();

                // Validate format
                if !validate_org_public_id(org_public_id) {
                    return Err(AuthError::unauthorized("Invalid organization ID format"));
                }

                // Check user membership
                let org = user.get_org(org_public_id).ok_or_else(|| {
                    // Return 404 to prevent enumeration
                    AuthError {
                        error: "Organization not found".to_string(),
                        status: StatusCode::NOT_FOUND,
                    }
                })?;

                Ok(ResolvedOrg {
                    org_id: org.org_id,
                    public_id: org.public_id.clone(),
                    name: org.name.clone(),
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auth_user_anonymous() {
        let user = AuthUser::anonymous();
        assert_eq!(user.id, Uuid::nil());
        assert!(user.is_admin());
        assert!(user.has_role("admin"));
        assert_eq!(user.auth_method, AuthMethod::None);
        // Anonymous user should belong to default org
        assert_eq!(user.organizations.len(), 1);
        assert_eq!(user.organizations[0].org_id, DEFAULT_ORG_ID);
        assert_eq!(user.organizations[0].public_id, DEFAULT_ORG_PUBLIC_ID);
    }

    #[test]
    fn test_auth_user_has_role() {
        let user = AuthUser {
            id: Uuid::nil(), // Use nil UUID for testing
            email: "test@example.com".to_string(),
            name: "Test".to_string(),
            roles: vec!["user".to_string(), "editor".to_string()],
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
            auth_method: AuthMethod::Jwt,
            organizations: vec![
                OrgMembership {
                    org_id: 1,
                    public_id: "org_00000000000000000000000000000001".to_string(),
                    name: "Org 1".to_string(),
                },
                OrgMembership {
                    org_id: 2,
                    public_id: "org_00000000000000000000000000000002".to_string(),
                    name: "Org 2".to_string(),
                },
            ],
        };

        assert!(user.is_member_of(1));
        assert!(user.is_member_of(2));
        assert!(!user.is_member_of(3));

        assert!(user.is_member_of_public("org_00000000000000000000000000000001"));
        assert!(user.is_member_of_public("org_00000000000000000000000000000002"));
        assert!(!user.is_member_of_public("org_00000000000000000000000000000003"));

        let org = user.get_org("org_00000000000000000000000000000001");
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
}

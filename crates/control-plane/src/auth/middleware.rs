// Authentication middleware and extractors
// Decision: Support both cookie-based (UI) and header-based (API) auth
// Decision: In "none" mode, create an anonymous user context

use axum::{
    extract::FromRequestParts,
    http::{header, request::Parts, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use axum_extra::extract::CookieJar;
use serde::Serialize;
use std::sync::Arc;
use uuid::Uuid;

use super::{
    api_key::{hash_api_key, is_valid_api_key_format, ValidatedApiKey, API_KEY_PREFIX},
    config::{AuthConfig, AuthMode},
    jwt::JwtService,
};
use crate::storage::Database;

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
        }
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
    pub db: Arc<Database>,
}

impl AuthState {
    pub fn new(config: AuthConfig, db: Arc<Database>) -> Self {
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

/// Helper trait for extracting AuthState from application state
pub trait FromRef<T> {
    fn from_ref(input: &T) -> Self;
}

impl FromRef<AuthState> for AuthState {
    fn from_ref(input: &AuthState) -> Self {
        input.clone()
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

    Ok(AuthUser {
        id: user_id,
        email: claims.email,
        name: claims.name,
        roles: claims.roles,
        auth_method: AuthMethod::Jwt,
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

    Ok(AuthUser {
        id: user.id,
        email: user.email,
        name: user.name,
        roles,
        auth_method: AuthMethod::ApiKey,
    })
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
    }

    #[test]
    fn test_auth_user_has_role() {
        let user = AuthUser {
            id: Uuid::nil(), // Use nil UUID for testing
            email: "test@example.com".to_string(),
            name: "Test".to_string(),
            roles: vec!["user".to_string(), "editor".to_string()],
            auth_method: AuthMethod::Jwt,
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
        };

        assert!(admin.is_admin());
        assert!(admin.has_role("admin"));
        assert!(admin.has_role("user")); // Admin has all roles
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

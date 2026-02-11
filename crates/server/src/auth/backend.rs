// Pluggable authentication backend
// Decision: Trait allows external auth providers (PropelAuth, Auth0, Keycloak)
// without modifying OSS code. OSS ships BuiltinAuthBackend as the default.

use async_trait::async_trait;
use axum::Router;

use super::middleware::{AuthError, AuthUser};
use super::routes::AuthConfigResponse;

/// Authentication backend trait.
///
/// Implementations validate credentials and return an `AuthUser`.
/// The OSS default is `BuiltinAuthBackend` (JWT + password + API key).
/// SaaS wrappers provide their own implementation (e.g., PropelAuth).
#[async_trait]
pub trait AuthBackend: Send + Sync + 'static {
    /// Validate a Bearer token (JWT or opaque) and return the authenticated user.
    async fn validate_token(&self, token: &str) -> Result<AuthUser, AuthError>;

    /// Validate an API key and return the authenticated user.
    async fn validate_api_key(&self, key: &str) -> Result<AuthUser, AuthError>;

    /// Return auth-specific HTTP routes (login, register, OAuth callbacks, API key CRUD).
    /// Returns `None` if auth is handled externally (e.g., PropelAuth hosted UI).
    fn auth_routes(&self) -> Option<Router>;

    /// Return the auth configuration for the `/v1/auth/config` endpoint.
    fn auth_config_response(&self) -> AuthConfigResponse;
}

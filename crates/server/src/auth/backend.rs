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
/// The OSS default is `BuiltinAuthBackend` (JWT + password + personal access token).
/// SaaS wrappers provide their own implementation (e.g., PropelAuth).
#[async_trait]
pub trait AuthBackend: Send + Sync + 'static {
    /// Validate a Bearer token (JWT or opaque) for the general `/api/*` surface
    /// and return the authenticated user.
    ///
    /// Implementations MUST reject MCP-scoped access tokens here (tokens minted
    /// for the `/mcp` resource). MCP tokens are accepted only by
    /// [`validate_mcp_token`](Self::validate_mcp_token). This audience split
    /// prevents an OAuth confused-deputy: a token a user authorized for `/mcp`
    /// must not act as a full user token on the REST API (TM-MCP-006).
    async fn validate_token(&self, token: &str) -> Result<AuthUser, AuthError>;

    /// Validate an MCP-scoped Bearer token for the `/mcp` endpoint.
    ///
    /// `expected_resource` is the canonical MCP resource URL (e.g.
    /// `https://app.example.com/mcp`). Implementations MUST accept only tokens
    /// minted for that resource (distinguishing claim + audience binding) and
    /// MUST reject regular session/access tokens.
    ///
    /// The default implementation fails closed (`401`). Backends that issue MCP
    /// OAuth tokens — the OSS `BuiltinAuthBackend` and SaaS wrappers — override
    /// it; the mint side is [`JwtService::generate_mcp_access_token`].
    async fn validate_mcp_token(
        &self,
        _token: &str,
        _expected_resource: &str,
    ) -> Result<AuthUser, AuthError> {
        Err(AuthError::unauthorized(
            "MCP token validation not supported",
        ))
    }

    /// Validate a personal access token and return the authenticated user.
    async fn validate_personal_access_token(&self, token: &str) -> Result<AuthUser, AuthError>;

    /// Return auth-specific HTTP routes (login, register, OAuth callbacks).
    /// Returns `None` if auth is handled externally (e.g., PropelAuth hosted UI).
    ///
    /// Personal access token CRUD routes are mounted separately by `ServerAppBuilder`
    /// via `personal_access_token_routes()` — they are auth-provider-agnostic.
    fn auth_routes(&self) -> Option<Router>;

    /// Called when a personal access token is deleted, so backends can invalidate caches.
    /// Default: no-op (backends without caching don't need to override).
    fn on_personal_access_token_deleted(&self) {}

    /// Return root-level public routes that must not be nested under the API prefix.
    ///
    /// Used for endpoints like `/.well-known/*` and other browser-facing pages that
    /// share the backend origin but do not live under `/api`.
    fn public_routes(&self) -> Option<Router> {
        None
    }

    /// Return the auth configuration for the `/v1/auth/config` endpoint.
    fn auth_config_response(&self) -> AuthConfigResponse;
}

// Authentication middleware and extractors
// Decision: Support both cookie-based (UI) and header-based (API) auth
// Decision: In "none" mode, create an anonymous user context

use axum::{
    extract::{FromRef, FromRequestParts},
    http::{StatusCode, header, request::Parts},
    response::{IntoResponse, Response},
};
use axum_extra::extract::CookieJar;
use everruns_core::{
    Caller, DEFAULT_ORG_ID, DEFAULT_ORG_PUBLIC_ID, DefaultPermissionResolver, OrgRole,
    PermissionResolver,
};
use everruns_platform::{
    ANONYMOUS_USER_EMAIL, ANONYMOUS_USER_ID, ANONYMOUS_USER_NAME, FeatureFlags, OrgMembership,
    validate_org_public_id,
};
use serde::Serialize;
use std::sync::Arc;
use uuid::Uuid;

use super::{
    backend::AuthBackend,
    config::{AuthConfig, AuthMode},
    personal_access_token::PAT_PREFIX,
};
use crate::storage::StorageBackend;

/// Authentication error
#[derive(Debug, Clone, Serialize)]
pub struct AuthError {
    pub error: String,
    #[serde(skip)]
    pub status: StatusCode,
    #[serde(skip)]
    pub code: Option<&'static str>,
}

impl AuthError {
    pub fn unauthorized(message: &str) -> Self {
        Self {
            error: message.to_string(),
            status: StatusCode::UNAUTHORIZED,
            code: Some("unauthorized"),
        }
    }

    pub fn forbidden(message: &str) -> Self {
        Self {
            error: message.to_string(),
            status: StatusCode::FORBIDDEN,
            code: Some("forbidden"),
        }
    }

    pub fn unprocessable(message: &str) -> Self {
        Self {
            error: message.to_string(),
            status: StatusCode::UNPROCESSABLE_ENTITY,
            code: Some("unprocessable"),
        }
    }

    pub fn not_found(message: &str) -> Self {
        Self {
            error: message.to_string(),
            status: StatusCode::NOT_FOUND,
            code: Some("not_found"),
        }
    }

    /// Generic 400 for malformed/invalid input that is not an auth failure.
    /// Used by account-recovery endpoints for invalid/expired/used tokens, where
    /// a generic message avoids leaking which condition was hit.
    pub fn bad_request(message: &str) -> Self {
        Self {
            error: message.to_string(),
            status: StatusCode::BAD_REQUEST,
            code: Some("bad_request"),
        }
    }

    /// 409 for a request that is well-formed but conflicts with existing state
    /// where the caller has already proven ownership — e.g. an OAuth callback
    /// whose verified email matches an unsafe local account or whose provider
    /// identity is already bound. The caller completed the provider handshake,
    /// so naming the conflict is not account enumeration (they own the mailbox).
    pub fn conflict(message: &str) -> Self {
        Self {
            error: message.to_string(),
            status: StatusCode::CONFLICT,
            code: Some("conflict"),
        }
    }

    /// 429 for per-account / per-address throttles (login stuffing, email
    /// bombing). Message stays generic — the throttle itself must not become
    /// an enumeration oracle.
    pub fn too_many_requests(message: &str) -> Self {
        Self {
            error: message.to_string(),
            status: StatusCode::TOO_MANY_REQUESTS,
            code: Some("rate_limited"),
        }
    }

    /// Internal server error. Use for storage/DB or other server-side failures
    /// so clients can distinguish a real auth failure (401) from a transient
    /// server error. The message must stay generic — never leak internals.
    pub fn internal(message: &str) -> Self {
        Self {
            error: message.to_string(),
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: Some("internal_error"),
        }
    }
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        // Render through the shared RFC 9457 ErrorResponse so the auth surface
        // matches every other API endpoint's wire format.
        let mut body = crate::api::common::ErrorResponse::new(self.error);
        if let Some(code) = self.code {
            body = body.with_code(code);
        }
        body.into_response(self.status).into_response()
    }
}

/// Authenticated user context extracted from request
#[derive(Debug, Clone)]
pub struct AuthUser {
    /// User ID
    pub id: Uuid,
    /// User email
    pub email: String,
    /// User name
    pub name: String,
    /// User roles
    pub roles: Vec<String>,
    /// Whether this user may access global/platform surfaces.
    pub is_platform_user: bool,
    /// Authentication method used
    pub auth_method: AuthMethod,
    /// Organizations the user belongs to
    pub organizations: Vec<OrgMembership>,
}

impl AuthUser {
    /// Create an anonymous user for no-auth mode.
    /// Uses a well-known UUID that corresponds to a real database user,
    /// so all code paths (org membership, API keys, etc.) work uniformly.
    pub fn anonymous() -> Self {
        Self {
            id: ANONYMOUS_USER_ID,
            email: ANONYMOUS_USER_EMAIL.to_string(),
            name: ANONYMOUS_USER_NAME.to_string(),
            roles: vec!["admin".to_string()], // Full access in no-auth mode
            is_platform_user: true,
            auth_method: AuthMethod::None,
            organizations: vec![OrgMembership {
                org_id: DEFAULT_ORG_ID,
                public_id: DEFAULT_ORG_PUBLIC_ID.to_string(),
                name: "Default Organization".to_string(),
                role: OrgRole::Owner,
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
    pub fn has_role(&self, role: &str) -> bool {
        self.roles.iter().any(|r| r == role || r == "admin")
    }

    /// Check if user is admin
    pub fn is_admin(&self) -> bool {
        self.has_role("admin")
    }
}

/// Authentication method used
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthMethod {
    /// No authentication (anonymous)
    None,
    /// JWT access token (browser/session/API)
    Jwt,
    /// MCP OAuth access token (resource-bound to the `/mcp` endpoint).
    /// Behaves like `Jwt` for org resolution but is minted and validated on a
    /// separate path so it cannot act as a full user token on `/api/*`
    /// (TM-MCP-006).
    Mcp,
    /// Personal access token
    PersonalAccessToken,
}

/// Auth state shared across routes
#[derive(Clone)]
pub struct AuthState {
    pub config: AuthConfig,
    pub backend: Arc<dyn AuthBackend>,
    /// Permission resolver for policy evaluation. Defaults to `DefaultPermissionResolver`.
    /// Downstream consumers can inject a custom resolver to enforce billing-tier rules,
    /// database-backed grants, or external RBAC decisions.
    pub permission_resolver: Arc<dyn PermissionResolver>,
    /// Storage backend for org lookups in ResolvedOrg extraction.
    /// Used in AuthMethod::None (anonymous user only carries default org)
    /// and AuthMethod::Jwt (JWT may be stale after server-side org creation).
    pub db: Option<Arc<StorageBackend>>,
    /// Deployment-level feature flags (env + grade). Combined with per-org opt-in
    /// when building [`ResolvedOrg::feature_flags`].
    pub system_feature_flags: FeatureFlags,
}

impl AuthState {
    pub fn new(config: AuthConfig, backend: Arc<dyn AuthBackend>) -> Self {
        Self {
            config,
            backend,
            permission_resolver: Arc::new(DefaultPermissionResolver),
            db: None,
            system_feature_flags: FeatureFlags::current(),
        }
    }

    /// Create with a custom permission resolver.
    pub fn with_resolver(
        config: AuthConfig,
        backend: Arc<dyn AuthBackend>,
        resolver: Arc<dyn PermissionResolver>,
    ) -> Self {
        Self {
            config,
            backend,
            permission_resolver: resolver,
            db: None,
            system_feature_flags: FeatureFlags::current(),
        }
    }

    /// Convenience: create with built-in backend (OSS default)
    pub fn builtin(config: AuthConfig, db: Arc<StorageBackend>) -> Self {
        let host_composition = Arc::new(crate::platform::oss_host_composition());
        let backend = Arc::new(super::builtin::BuiltinAuthBackend::new(
            config.clone(),
            db.clone(),
            host_composition,
        ));
        Self {
            config,
            backend,
            permission_resolver: Arc::new(DefaultPermissionResolver),
            db: Some(db),
            system_feature_flags: FeatureFlags::current(),
        }
    }

    /// Set the storage backend (for org resolution in ResolvedOrg extraction).
    pub fn with_db(mut self, db: Arc<StorageBackend>) -> Self {
        self.db = Some(db);
        self
    }

    pub fn with_system_feature_flags(mut self, flags: FeatureFlags) -> Self {
        self.system_feature_flags = flags;
        self
    }
}

/// Extractor for authenticated user
/// This is required - returns 401 if not authenticated
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

        // Parse scheme + token per RFC 7235 (case-insensitive scheme)
        let token_after_bearer = {
            let mut parts = auth_str.splitn(2, ' ');
            match (parts.next(), parts.next()) {
                (Some(scheme), Some(token)) if scheme.eq_ignore_ascii_case("bearer") => Some(token),
                _ => None,
            }
        };

        // Bearer token: either personal access token (evr_pat_ prefix) or JWT
        if let Some(token) = token_after_bearer {
            if token.starts_with(PAT_PREFIX) {
                return auth_state
                    .backend
                    .validate_personal_access_token(token)
                    .await;
            }
            return auth_state.backend.validate_token(token).await;
        }

        // Legacy: bare token or "ApiKey" scheme prefix (kept for non-Bearer clients)
        let token_after_scheme = {
            let mut parts = auth_str.splitn(2, ' ');
            match (parts.next(), parts.next()) {
                (Some(scheme), Some(token)) if scheme.eq_ignore_ascii_case("apikey") => Some(token),
                _ => None,
            }
        };

        if let Some(token) = token_after_scheme {
            return auth_state
                .backend
                .validate_personal_access_token(token)
                .await;
        }
        if auth_str.starts_with(PAT_PREFIX) {
            return auth_state
                .backend
                .validate_personal_access_token(auth_str)
                .await;
        }
    }

    // Try to extract from cookie (for UI)
    let jar = CookieJar::from_headers(&parts.headers);
    if let Some(cookie) = jar.get("access_token") {
        return auth_state.backend.validate_token(cookie.value()).await;
    }

    // No valid credentials found
    Err(AuthError::unauthorized("Authentication required"))
}

/// Extract the authenticated caller for the MCP (`/mcp`) endpoint.
///
/// THREAT[TM-MCP-006]: this is the separate validation path for the `/mcp`
/// resource. It accepts only:
/// - the anonymous user in `AuthMode::None` (local dev),
/// - personal access tokens (`evr_pat_` / `ApiKey`) — intentionally full-access
///   programmatic credentials,
/// - MCP-scoped OAuth Bearer JWTs, via `validate_mcp_token(token, resource)`.
///
/// It deliberately does NOT accept regular session/access JWTs or the
/// `access_token` cookie: a browser session must not be replayed onto the MCP
/// resource, and (symmetrically) `validate_token` rejects MCP tokens on
/// `/api/*`, so the two surfaces stay audience-isolated.
pub async fn extract_mcp_auth_user(
    parts: &mut Parts,
    auth_state: &AuthState,
    mcp_resource: Option<&str>,
) -> Result<AuthUser, AuthError> {
    if auth_state.config.mode == AuthMode::None {
        return Ok(AuthUser::anonymous());
    }

    if let Some(auth_header) = parts.headers.get(header::AUTHORIZATION) {
        let auth_str = auth_header
            .to_str()
            .map_err(|_| AuthError::unauthorized("Invalid authorization header"))?;

        // Bearer scheme (RFC 7235, case-insensitive).
        let token_after_bearer = {
            let mut it = auth_str.splitn(2, ' ');
            match (it.next(), it.next()) {
                (Some(scheme), Some(token)) if scheme.eq_ignore_ascii_case("bearer") => Some(token),
                _ => None,
            }
        };
        if let Some(token) = token_after_bearer {
            if token.starts_with(PAT_PREFIX) {
                return auth_state
                    .backend
                    .validate_personal_access_token(token)
                    .await;
            }
            // Bearer JWT must be an MCP-scoped token for this resource. Fail
            // fast with a 500 (not a 401) when the server has no MCP resource
            // configured: an empty/absent audience would reject every otherwise
            // valid MCP token as if it were the client's fault, masking a
            // server misconfiguration (TM-MCP-006).
            let resource = mcp_resource
                .filter(|r| !r.is_empty())
                .ok_or_else(|| AuthError::internal("MCP resource not configured"))?;
            return auth_state.backend.validate_mcp_token(token, resource).await;
        }

        // Legacy "ApiKey" scheme / bare PAT (kept for non-Bearer clients).
        let token_after_scheme = {
            let mut it = auth_str.splitn(2, ' ');
            match (it.next(), it.next()) {
                (Some(scheme), Some(token)) if scheme.eq_ignore_ascii_case("apikey") => Some(token),
                _ => None,
            }
        };
        if let Some(token) = token_after_scheme {
            return auth_state
                .backend
                .validate_personal_access_token(token)
                .await;
        }
        if auth_str.starts_with(PAT_PREFIX) {
            return auth_state
                .backend
                .validate_personal_access_token(auth_str)
                .await;
        }
    }

    // Cookie sessions are intentionally NOT accepted on /mcp (see doc above).
    Err(AuthError::unauthorized("Authentication required"))
}

/// Require admin role extractor
#[derive(Debug, Clone)]
pub struct AdminUser(pub AuthUser);

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

/// Require platform-user access for global/system surfaces.
#[derive(Debug, Clone)]
pub struct PlatformUser(pub AuthUser);

impl<S> FromRequestParts<S> for PlatformUser
where
    S: Send + Sync,
    AuthState: FromRef<S>,
{
    type Rejection = AuthError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let user = AuthUser::from_request_parts(parts, state).await?;

        if !user.is_platform_user {
            return Err(AuthError::forbidden("Platform user access required"));
        }

        Ok(PlatformUser(user))
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
    /// User's role in this organization
    pub role: OrgRole,
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
                code: None,
            }
        })?;

        Ok(OrgContext {
            org_id: org.org_id,
            public_id: org.public_id.clone(),
            name: org.name.clone(),
            role: org.role,
        })
    }
}

// ============================================================================
// Role-based extractors
// ============================================================================

/// Require Admin+ role in the organization (from URL path)
#[derive(Debug, Clone)]
pub struct OrgAdmin(pub OrgContext);

impl<S> FromRequestParts<S> for OrgAdmin
where
    S: Send + Sync,
    AuthState: FromRef<S>,
{
    type Rejection = AuthError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let org = OrgContext::from_request_parts(parts, state).await?;
        if !org.role.has_permission(OrgRole::Admin) {
            return Err(AuthError::forbidden("Admin access required"));
        }
        Ok(OrgAdmin(org))
    }
}

/// Require Owner role in the organization (from URL path)
#[derive(Debug, Clone)]
pub struct OrgOwner(pub OrgContext);

impl<S> FromRequestParts<S> for OrgOwner
where
    S: Send + Sync,
    AuthState: FromRef<S>,
{
    type Rejection = AuthError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let org = OrgContext::from_request_parts(parts, state).await?;
        if !org.role.has_permission(OrgRole::Owner) {
            return Err(AuthError::forbidden("Owner access required"));
        }
        Ok(OrgOwner(org))
    }
}

// ============================================================================
// ResolvedOrg - Organization context from auth (not URL path)
// ============================================================================

/// Cookie name for org selection in session auth (re-exported by api/users.rs)
pub const ORG_COOKIE_NAME: &str = "everruns_org";

/// Max-Age for the org-selection cookie. Without an explicit Max-Age the
/// browser drops it on restart and the UI falls back to the default org.
/// Matches the default refresh-token lifetime so the selection outlives any
/// session it can belong to.
pub const ORG_COOKIE_MAX_AGE: time::Duration = time::Duration::days(30);

/// Organization context resolved from authentication
///
/// Unlike OrgContext which extracts org from URL path, ResolvedOrg derives
/// the organization from the authentication context:
/// - API key auth: org from `X-Org-Id` header or `everruns_org` cookie, validated against membership.
///   If user has exactly one org, it is used as a convenience default.
/// - Session auth (JWT/None): org from everruns_org cookie, validated against user membership
///
/// The cookie is set via POST /v1/users/me/switch-org endpoint.
/// Cookies work automatically with SSE (EventSource) unlike headers.
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
    /// Authenticated principal's user ID (including the well-known local user in none mode)
    pub user_id: Option<Uuid>,
    /// User's role in this organization
    pub role: OrgRole,
    /// Whether the authenticated user may access global/platform surfaces.
    pub is_platform_user: bool,
    /// Effective API-visible feature flags for this org (system gate + org opt-in).
    pub feature_flags: FeatureFlags,
}

impl ResolvedOrg {
    pub async fn with_effective_feature_flags(self, auth_state: &AuthState) -> Self {
        let feature_flags = if let Some(db) = &auth_state.db {
            crate::services::org_feature_flags::resolve_org_feature_flags(
                db,
                self.org_id,
                &auth_state.system_feature_flags,
            )
            .await
            .unwrap_or_else(|_| {
                FeatureFlags::for_org(
                    &auth_state.system_feature_flags,
                    &std::collections::HashMap::new(),
                )
            })
        } else {
            FeatureFlags::for_org(
                &auth_state.system_feature_flags,
                &std::collections::HashMap::new(),
            )
        };
        Self {
            feature_flags,
            ..self
        }
    }
}

impl From<&ResolvedOrg> for Caller {
    fn from(org: &ResolvedOrg) -> Self {
        Caller {
            org_id: org.org_id,
            org_public_id: org.public_id.clone(),
            user_id: org.user_id,
            role: org.role,
            is_platform_user: org.is_platform_user,
            is_internal: false,
        }
    }
}

impl<S> FromRequestParts<S> for ResolvedOrg
where
    S: Send + Sync,
    AuthState: axum::extract::FromRef<S>,
{
    type Rejection = AuthError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        // First extract the authenticated user
        let user = AuthUser::from_request_parts(parts, state).await?;
        let auth_state = AuthState::from_ref(state);
        resolve_org_for_user(user, parts, &auth_state).await
    }
}

/// Resolve the organization context for an already-authenticated user.
///
/// Shared by the `ResolvedOrg` extractor and the MCP endpoint's
/// `McpResolvedOrg`, so an MCP-scoped token (validated on the `/mcp` path) gets
/// the same org-resolution semantics as a session JWT without going back
/// through `validate_token`.
pub(crate) async fn resolve_org_for_user(
    user: AuthUser,
    parts: &mut Parts,
    auth_state: &AuthState,
) -> Result<ResolvedOrg, AuthError> {
    {
        match user.auth_method {
            AuthMethod::PersonalAccessToken => {
                // Personal access token auth: org resolved from X-Org-Id header,
                // everruns_org cookie, or single-org convenience fallback.
                let jar = CookieJar::from_headers(&parts.headers);
                let header_org = parts
                    .headers
                    .get("x-org-id")
                    .and_then(|v| v.to_str().ok())
                    .map(String::from);
                let cookie_org = jar.get(ORG_COOKIE_NAME).map(|c| c.value().to_string());
                let explicit_org = header_org.or(cookie_org);

                if let Some(org_public_id) = &explicit_org {
                    if !validate_org_public_id(org_public_id) {
                        return Err(AuthError::unauthorized("Invalid organization ID format"));
                    }
                    // Validate against user's org memberships (already loaded in AuthUser)
                    let org = user.get_org(org_public_id).ok_or_else(|| AuthError {
                        error: "Organization not found".to_string(),
                        status: StatusCode::NOT_FOUND,
                        code: None,
                    })?;
                    return Ok(ResolvedOrg {
                        org_id: org.org_id,
                        public_id: org.public_id.clone(),
                        name: org.name.clone(),
                        user_id: Some(user.id),
                        role: org.role,
                        is_platform_user: user.is_platform_user,
                        feature_flags: FeatureFlags::default(),
                    }
                    .with_effective_feature_flags(auth_state)
                    .await);
                }

                // No explicit org — convenience: if user has exactly one org, use it
                if user.organizations.len() == 1 {
                    let org = &user.organizations[0];
                    return Ok(ResolvedOrg {
                        org_id: org.org_id,
                        public_id: org.public_id.clone(),
                        name: org.name.clone(),
                        user_id: Some(user.id),
                        role: org.role,
                        is_platform_user: user.is_platform_user,
                        feature_flags: FeatureFlags::default(),
                    }
                    .with_effective_feature_flags(auth_state)
                    .await);
                }

                // Multiple orgs, no explicit selection
                if user.organizations.is_empty() {
                    return Err(AuthError::unauthorized("No organization available"));
                }
                Err(AuthError {
                    error: "Multiple organizations available. Specify the target organization via the X-Org-Id header.".to_string(),
                    status: StatusCode::BAD_REQUEST, code: None,
        })
            }
            AuthMethod::None => {
                // No-auth mode: anonymous user only carries the default org,
                // but the user may have switched to a different org via cookie.
                // Read the org cookie and resolve via DB if available.
                let jar = CookieJar::from_headers(&parts.headers);
                let cookie_org_id = jar.get(ORG_COOKIE_NAME).map(|c| c.value().to_string());

                if let (Some(org_public_id), Some(db)) = (&cookie_org_id, &auth_state.db)
                    && validate_org_public_id(org_public_id)
                    && let Ok(Some(org_row)) = db.get_organization_by_public_id(org_public_id).await
                {
                    return Ok(ResolvedOrg {
                        org_id: org_row.org_id,
                        public_id: org_row.public_id,
                        name: org_row.name,
                        user_id: Some(user.id),
                        role: OrgRole::Owner, // Anonymous user is owner of all orgs
                        is_platform_user: user.is_platform_user,
                        feature_flags: FeatureFlags::default(),
                    }
                    .with_effective_feature_flags(auth_state)
                    .await);
                }

                // Fall back to default org
                let org = user
                    .organizations
                    .first()
                    .ok_or_else(|| AuthError::unauthorized("No organization available"))?;
                Ok(ResolvedOrg {
                    org_id: org.org_id,
                    public_id: org.public_id.clone(),
                    name: org.name.clone(),
                    user_id: Some(user.id),
                    role: org.role,
                    is_platform_user: user.is_platform_user,
                    feature_flags: FeatureFlags::default(),
                }
                .with_effective_feature_flags(auth_state)
                .await)
            }
            AuthMethod::Jwt | AuthMethod::Mcp => {
                // Session auth (and MCP OAuth tokens, which resolve org the same
                // way): get org from everruns_org cookie. Cookie is set via the
                // switch-org endpoint and works automatically with SSE unlike
                // headers. MCP clients can't set cookies, so they fall through to
                // the first-org default below (and override per-call via
                // `organization_id`).
                let jar = CookieJar::from_headers(&parts.headers);
                let cookie_org = jar.get(ORG_COOKIE_NAME).map(|c| c.value().to_string());

                // When no org cookie is present (e.g. MCP OAuth Bearer tokens),
                // fall back to the user's first organization.
                if cookie_org.is_none() {
                    let org = user
                        .organizations
                        .first()
                        .ok_or_else(|| AuthError::unauthorized("No organization available"))?;
                    return Ok(ResolvedOrg {
                        org_id: org.org_id,
                        public_id: org.public_id.clone(),
                        name: org.name.clone(),
                        user_id: Some(user.id),
                        role: org.role,
                        is_platform_user: user.is_platform_user,
                        feature_flags: FeatureFlags::default(),
                    }
                    .with_effective_feature_flags(auth_state)
                    .await);
                }

                let org_public_id = cookie_org.unwrap();
                let org_public_id = org_public_id.as_str();

                // Validate format
                if !validate_org_public_id(org_public_id) {
                    return Err(AuthError::unauthorized("Invalid organization ID format"));
                }

                // Check user membership against the database, not the JWT.
                // The JWT may be stale (e.g. after creating a new org server-side,
                // the client JWT won't include it until PropelAuth refreshes the token).
                // This mirrors the fix in the `switch_org` endpoint
                // (`crates/server/src/api/users.rs`).
                if let Some(db) = &auth_state.db
                    && let Ok(user_orgs) = db.list_user_organizations(user.id).await
                {
                    if let Some(org_row) = user_orgs.iter().find(|o| o.public_id == org_public_id) {
                        let role = org_row.role.parse::<OrgRole>().unwrap_or(OrgRole::Member);
                        return Ok(ResolvedOrg {
                            org_id: org_row.org_id,
                            public_id: org_row.public_id.clone(),
                            name: org_row.name.clone(),
                            user_id: Some(user.id),
                            role,
                            is_platform_user: user.is_platform_user,
                            feature_flags: FeatureFlags::default(),
                        }
                        .with_effective_feature_flags(auth_state)
                        .await);
                    }
                    // DB available, user not a member → 404
                    return Err(AuthError {
                        error: "Organization not found".to_string(),
                        status: StatusCode::NOT_FOUND,
                        code: None,
                    });
                }
                // DB query failed — fall through to JWT-based validation

                // Fallback: DB unavailable, validate against JWT org list
                let org = user.get_org(org_public_id).ok_or_else(|| AuthError {
                    error: "Organization not found".to_string(),
                    status: StatusCode::NOT_FOUND,
                    code: None,
                })?;

                Ok(ResolvedOrg {
                    org_id: org.org_id,
                    public_id: org.public_id.clone(),
                    name: org.name.clone(),
                    user_id: Some(user.id),
                    role: org.role,
                    is_platform_user: user.is_platform_user,
                    feature_flags: FeatureFlags::default(),
                }
                .with_effective_feature_flags(auth_state)
                .await)
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
                    public_id: "org_00000000000000000000000000000001".to_string(),
                    name: "Org 1".to_string(),
                    role: OrgRole::Owner,
                },
                OrgMembership {
                    org_id: 2,
                    public_id: "org_00000000000000000000000000000002".to_string(),
                    name: "Org 2".to_string(),
                    role: OrgRole::Member,
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
        let db = Arc::new(StorageBackend::in_memory());
        let switched_org = db
            .create_organization(CreateOrganizationRow {
                public_id: "org_00000000000000000000000000000002".to_string(),
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
                format!("{}=org_00000000000000000000000000000002", ORG_COOKIE_NAME),
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
        let db = Arc::new(StorageBackend::in_memory());
        let backend: Arc<dyn AuthBackend> = Arc::new(JwtMockBackend::with_user(user));
        let state = AuthState {
            config: AuthConfig {
                mode: AuthMode::Full,
                ..AuthConfig::default()
            },
            backend,
            permission_resolver: Arc::new(DefaultPermissionResolver),
            db: Some(db.clone()),
            system_feature_flags: FeatureFlags::current(),
        };
        (state, db)
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
                public_id: "org_00000000000000000000000000000001".to_string(),
                name: "Org A".to_string(),
                role: OrgRole::Owner,
            }],
        };

        let (state, db) = jwt_auth_state_with_db(jwt_user);

        // Seed org_b in the DB and add user membership
        let org_b = db
            .create_organization(CreateOrganizationRow {
                public_id: "org_00000000000000000000000000000002".to_string(),
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
                format!("{}=org_00000000000000000000000000000002", ORG_COOKIE_NAME),
            )
            .body(())
            .unwrap()
            .into_parts();

        let resolved = ResolvedOrg::from_request_parts(&mut parts, &state)
            .await
            .expect("should resolve org_b from DB");

        assert_eq!(resolved.public_id, "org_00000000000000000000000000000002");
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
                public_id: "org_00000000000000000000000000000001".to_string(),
                name: "Org A".to_string(),
                role: OrgRole::Owner,
            }],
        };

        let (state, db) = jwt_auth_state_with_db(jwt_user);

        // Seed org_c in the DB but do NOT add user membership
        db.create_organization(CreateOrganizationRow {
            public_id: "org_00000000000000000000000000000003".to_string(),
            name: "Org C".to_string(),
            created_by: None,
        })
        .await
        .unwrap();

        let (mut parts, _body) = Request::builder()
            .header(header::AUTHORIZATION, "Bearer fake-jwt-token")
            .header(
                header::COOKIE,
                format!("{}=org_00000000000000000000000000000003", ORG_COOKIE_NAME),
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
                public_id: "org_00000000000000000000000000000001".to_string(),
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
            system_feature_flags: FeatureFlags::current(),
        };

        let (mut parts, _body) = Request::builder()
            .header(header::AUTHORIZATION, "Bearer fake-jwt-token")
            .header(
                header::COOKIE,
                format!("{}=org_00000000000000000000000000000001", ORG_COOKIE_NAME),
            )
            .body(())
            .unwrap()
            .into_parts();

        let resolved = ResolvedOrg::from_request_parts(&mut parts, &state)
            .await
            .expect("should fall back to JWT org list");

        assert_eq!(resolved.public_id, "org_00000000000000000000000000000001");
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
        let backend: Arc<dyn AuthBackend> =
            Arc::new(PersonalAccessTokenMockBackend::with_user(user));
        AuthState {
            config: AuthConfig {
                mode: AuthMode::Full,
                ..AuthConfig::default()
            },
            backend,
            permission_resolver: Arc::new(DefaultPermissionResolver),
            db: None,
            system_feature_flags: FeatureFlags::current(),
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
                    public_id: "org_00000000000000000000000000000001".to_string(),
                    name: "Org A".to_string(),
                    role: OrgRole::Owner,
                },
                OrgMembership {
                    org_id: 2,
                    public_id: "org_00000000000000000000000000000002".to_string(),
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
                public_id: "org_00000000000000000000000000000001".to_string(),
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

        assert_eq!(resolved.public_id, "org_00000000000000000000000000000001");
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
            .header("x-org-id", "org_00000000000000000000000000000002")
            .body(())
            .unwrap()
            .into_parts();

        let resolved = ResolvedOrg::from_request_parts(&mut parts, &state)
            .await
            .expect("should resolve org from X-Org-Id header");

        assert_eq!(resolved.public_id, "org_00000000000000000000000000000002");
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
}

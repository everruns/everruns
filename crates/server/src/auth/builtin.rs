// Built-in authentication backend (JWT + password + OAuth + personal access tokens)
// Decision: Default for OSS. All existing behavior preserved.
// Decision: personal access token auth cache is write-only for now. Read-through
// caching of AuthUser introduced a TOCTOU authorization window for token expiry and
// membership/role changes. We keep inserts + invalidation hooks so follow-up
// work can safely reintroduce cache reads with fresh-state guarantees.

use async_trait::async_trait;
use axum::Router;
use everruns_core::{
    DEFAULT_ORG_ID, DEFAULT_ORG_PUBLIC_ID, OrgMembership, OrgRole, PlatformDefinition,
};
use moka::future::Cache;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

use super::backend::AuthBackend;
use super::config::AuthConfig;
use super::jwt::JwtService;
use super::middleware::{AuthError, AuthMethod, AuthUser};
use super::personal_access_token::{
    ValidatedPersonalAccessToken, hash_personal_access_token, is_valid_personal_access_token_format,
};
use super::rate_limit::AuthRateLimiter;
use super::routes::{self, AuthConfigResponse};
use crate::storage::StorageBackend;
use crate::valkey::ValkeyClient;

/// TTL for cached personal access token auth results.
const PAT_CACHE_TTL: Duration = Duration::from_secs(300); // 5 minutes

/// Max entries in the personal access token auth cache.
const PAT_CACHE_MAX_CAPACITY: u64 = 10_000;

/// Built-in authentication backend (JWT + password + OAuth + personal access tokens).
/// This is the default for OSS deployments.
///
/// HARNESS-SEED SAFETY NET (see also `specs/authentication.md`):
/// `register` and `oauth_callback` both add new users to `DEFAULT_ORG_ID`.
/// The background seed task (see `seed::spawn_seed_task_with_platform_definition`)
/// provisions built-in harnesses for that org, but it runs asynchronously
/// with a 500 ms initial delay — so a user who signs up during the startup
/// window (cold boot, slow DB, or a partial seed failure that will
/// self-retry) can otherwise land in an org that has no harnesses and see
/// the chat/session UI 404.
///
/// PR #1462 removed an earlier safety-net call that used
/// `oss_built_in_harnesses()`, because that could override an operator's
/// custom `PlatformDefinition`. The fix is to keep the safety net but
/// drive it from `platform_definition.built_in_harnesses()` instead, so a
/// custom platform definition is never overwritten. `platform_definition`
/// is owned by this backend for that purpose.
#[derive(Clone)]
pub struct BuiltinAuthBackend {
    pub config: AuthConfig,
    pub jwt_service: Arc<JwtService>,
    pub db: Arc<StorageBackend>,
    pub rate_limiter: AuthRateLimiter,
    /// Platform-defined harness set. Used by the signup safety-net in
    /// `register` / `oauth_callback` so a pre-seed signup still lands in an
    /// org with the correct (operator-chosen) harnesses.
    pub platform_definition: Arc<PlatformDefinition>,
    /// In-process cache: token_hash -> AuthUser. Avoids 4 sequential DB queries per token request.
    personal_access_token_cache: Cache<String, AuthUser>,
}

fn build_personal_access_token_cache() -> Cache<String, AuthUser> {
    Cache::builder()
        .max_capacity(PAT_CACHE_MAX_CAPACITY)
        .time_to_live(PAT_CACHE_TTL)
        .build()
}

/// Strip the API prefix from the base URL to recover the public root origin.
/// Used by MCP metadata endpoints and the MCP 401 `WWW-Authenticate` header
/// so clients can locate `/.well-known/oauth-protected-resource/mcp` (RFC 9728
/// §3.1 path-derived for the `/mcp` resource).
pub(crate) fn root_url_from_api_base(api_base_url: &str) -> String {
    let trimmed = api_base_url.trim_end_matches('/');
    if let Ok(api_prefix) = std::env::var("API_PREFIX") {
        let api_prefix = api_prefix.trim_end_matches('/');
        if !api_prefix.is_empty() && trimmed.ends_with(api_prefix) {
            return trimmed[..trimmed.len() - api_prefix.len()].to_string();
        }
    }
    trimmed.strip_suffix("/api").unwrap_or(trimmed).to_string()
}

fn platform_user_from_roles(roles: &[String]) -> bool {
    roles.iter().any(|role| role == "admin")
}

impl BuiltinAuthBackend {
    /// Create with in-memory rate limiting (per-instance).
    pub fn new(
        config: AuthConfig,
        db: Arc<StorageBackend>,
        platform_definition: Arc<PlatformDefinition>,
    ) -> Self {
        let jwt_service = Arc::new(JwtService::new(config.jwt.clone()));
        Self {
            config,
            jwt_service,
            db,
            rate_limiter: AuthRateLimiter::new(),
            platform_definition,
            personal_access_token_cache: build_personal_access_token_cache(),
        }
    }

    /// Create with Valkey-backed distributed rate limiting.
    pub fn with_valkey(
        config: AuthConfig,
        db: Arc<StorageBackend>,
        platform_definition: Arc<PlatformDefinition>,
        valkey: ValkeyClient,
    ) -> Self {
        let jwt_service = Arc::new(JwtService::new(config.jwt.clone()));
        Self {
            config,
            jwt_service,
            db,
            rate_limiter: AuthRateLimiter::with_valkey(valkey),
            platform_definition,
            personal_access_token_cache: build_personal_access_token_cache(),
        }
    }

    /// Invalidate a cached personal access token entry by its hash.
    pub async fn invalidate_personal_access_token_cache(&self, token_hash: &str) {
        self.personal_access_token_cache
            .invalidate(token_hash)
            .await;
    }

    /// Invalidate all cached personal access token entries (e.g. after user/org updates).
    pub fn invalidate_all_personal_access_token_cache(&self) {
        self.personal_access_token_cache.invalidate_all();
    }

    /// Entry count in the cache (for testing/metrics).
    #[cfg(test)]
    pub fn personal_access_token_cache_entry_count(&self) -> u64 {
        self.personal_access_token_cache.entry_count()
    }

    /// Validate personal access token against DB (4 sequential queries).
    async fn validate_personal_access_token_from_db(
        &self,
        token_hash: &str,
    ) -> Result<AuthUser, AuthError> {
        let token_row = self
            .db
            .get_personal_access_token_by_hash(token_hash)
            .await
            .map_err(|e| {
                tracing::error!("Failed to fetch personal access token: {}", e);
                AuthError::unauthorized("Failed to validate personal access token")
            })?
            .ok_or_else(|| AuthError::unauthorized("Invalid personal access token"))?;

        // Check if expired
        let validated_token = ValidatedPersonalAccessToken {
            token_id: token_row.id,
            user_id: token_row.user_id,
            name: token_row.name.clone(),
            scopes: serde_json::from_value(token_row.scopes.clone()).unwrap_or_default(),
            expires_at: token_row.expires_at,
        };

        if validated_token.is_expired() {
            return Err(AuthError::unauthorized("Personal access token expired"));
        }

        // Update last used timestamp (fire and forget)
        let db = self.db.clone();
        let token_id = token_row.id;
        tokio::spawn(async move {
            let _ = db.update_personal_access_token_last_used(token_id).await;
        });

        // Fetch user info
        let user = self
            .db
            .get_user(token_row.user_id)
            .await
            .map_err(|e| {
                tracing::error!("Failed to fetch user for personal access token: {}", e);
                AuthError::unauthorized("Failed to validate personal access token")
            })?
            .ok_or_else(|| AuthError::unauthorized("User not found for personal access token"))?;

        let roles: Vec<String> = serde_json::from_value(user.roles.clone()).unwrap_or_default();

        // Fetch all user's organizations (same as JWT auth)
        let user_orgs = self
            .db
            .list_user_organizations(user.id)
            .await
            .map_err(|e| {
                tracing::error!("Failed to fetch orgs for personal access token user: {}", e);
                AuthError::unauthorized("Failed to validate personal access token")
            })?;

        let organizations: Vec<OrgMembership> = user_orgs
            .into_iter()
            .map(|o| OrgMembership {
                org_id: o.org_id,
                public_id: o.public_id,
                name: o.name,
                role: o.role.parse::<OrgRole>().unwrap_or(OrgRole::Member),
            })
            .collect();

        Ok(AuthUser {
            id: user.id,
            email: user.email,
            name: user.name,
            is_platform_user: platform_user_from_roles(&roles),
            roles,
            auth_method: AuthMethod::PersonalAccessToken,
            organizations,
        })
    }
}

impl BuiltinAuthBackend {
    /// Build an `AuthUser` from validated JWT claims, enforcing that the subject
    /// user still exists and loading current org memberships from the DB.
    ///
    /// Shared by [`validate_token`](AuthBackend::validate_token) (regular access
    /// tokens) and [`validate_mcp_token`](AuthBackend::validate_mcp_token)
    /// (MCP-scoped tokens). `auth_method` distinguishes the two so downstream
    /// extractors and audit can tell an MCP-resource caller apart.
    async fn auth_user_from_claims(
        &self,
        claims: super::jwt::AccessTokenClaims,
        auth_method: AuthMethod,
    ) -> Result<AuthUser, AuthError> {
        let user_id = Uuid::parse_str(&claims.sub)
            .map_err(|_| AuthError::unauthorized("Invalid user ID in token"))?;

        // Enforce subject user still exists. Deleted users must not remain authenticated
        // with previously issued JWTs until token expiry.
        let user_exists = self.db.get_user(user_id).await.map_err(|e| {
            tracing::error!("Failed to fetch JWT user: {}", e);
            AuthError::unauthorized("Failed to validate token user")
        })?;
        if user_exists.is_none() {
            tracing::warn!(user_id = %user_id, "JWT subject user not found");
            return Err(AuthError::unauthorized("Invalid or expired token"));
        }

        // Fetch organization memberships for the user
        let organizations = fetch_user_organizations(&self.db, user_id).await?;

        // If user has no organizations, fall back to default organization
        if organizations.is_empty() {
            tracing::warn!(
                user_id = %user_id,
                "User has no organizations, falling back to default org"
            );
        }
        let organizations = organizations_or_default(organizations);

        let is_platform_user = platform_user_from_roles(&claims.roles);

        Ok(AuthUser {
            id: user_id,
            email: claims.email,
            name: claims.name,
            roles: claims.roles,
            is_platform_user,
            auth_method,
            organizations,
        })
    }
}

#[async_trait]
impl AuthBackend for BuiltinAuthBackend {
    async fn validate_token(&self, token: &str) -> Result<AuthUser, AuthError> {
        // `validate_access_token` rejects `mcp_access` tokens, so an MCP-scoped
        // token cannot authenticate the general `/api/*` surface (TM-MCP-006).
        let claims = self.jwt_service.validate_access_token(token).map_err(|e| {
            tracing::debug!("JWT validation failed: {}", e);
            AuthError::unauthorized("Invalid or expired token")
        })?;

        self.auth_user_from_claims(claims, AuthMethod::Jwt).await
    }

    async fn validate_mcp_token(
        &self,
        token: &str,
        expected_resource: &str,
    ) -> Result<AuthUser, AuthError> {
        // THREAT[TM-MCP-006]: accept only resource-bound `mcp_access` tokens here.
        // `validate_mcp_access_token` rejects regular session/access tokens and
        // tokens bound to a different audience, so the `/mcp` endpoint cannot be
        // entered with a full-API token and an `/mcp` token cannot escape to the
        // REST API.
        let claims = self
            .jwt_service
            .validate_mcp_access_token(token, expected_resource)
            .map_err(|e| {
                tracing::debug!("MCP JWT validation failed: {}", e);
                AuthError::unauthorized("Invalid or expired MCP token")
            })?;

        self.auth_user_from_claims(claims, AuthMethod::Mcp).await
    }

    async fn validate_personal_access_token(&self, token: &str) -> Result<AuthUser, AuthError> {
        if !is_valid_personal_access_token_format(token) {
            return Err(AuthError::unauthorized(
                "Invalid personal access token format",
            ));
        }

        let token_hash = hash_personal_access_token(token);

        // Security: always validate against DB to enforce real-time token expiry
        // and org membership/role changes.
        let auth_user = self
            .validate_personal_access_token_from_db(&token_hash)
            .await?;

        // Populate cache
        self.personal_access_token_cache
            .insert(token_hash, auth_user.clone())
            .await;

        Ok(auth_user)
    }

    fn auth_routes(&self) -> Option<Router> {
        let auth_routes = routes::routes(self.clone());
        let auth_state =
            super::middleware::AuthState::new(self.config.clone(), Arc::new(self.clone()));
        let api_base_url = self.config.base_url.trim_end_matches('/').to_string();
        let cli_state = super::cli_auth::CliAuthState {
            db: self.db.clone(),
            auth: auth_state,
            frontend_url: self.config.frontend_url.clone(),
            base_url: api_base_url,
        };
        let cli_routes = super::cli_auth::cli_auth_routes(cli_state);
        Some(auth_routes.merge(cli_routes))
    }

    fn on_personal_access_token_deleted(&self) {
        self.invalidate_all_personal_access_token_cache();
    }

    fn public_routes(&self) -> Option<Router> {
        let auth_state =
            super::middleware::AuthState::new(self.config.clone(), Arc::new(self.clone()));
        let api_base_url = self.config.base_url.trim_end_matches('/').to_string();
        let cli_state = super::cli_auth::CliAuthState {
            db: self.db.clone(),
            auth: auth_state.clone(),
            frontend_url: self.config.frontend_url.clone(),
            base_url: api_base_url.clone(),
        };
        let mcp_oauth_state = super::mcp_oauth::McpOAuthState {
            db: self.db.clone(),
            auth: auth_state,
            jwt_service: self.jwt_service.clone(),
            issuer_url: root_url_from_api_base(&api_base_url),
            frontend_url: self.config.frontend_url.clone(),
        };
        Some(
            super::cli_auth::cli_auth_public_routes(cli_state)
                .merge(super::mcp_oauth::mcp_oauth_routes(mcp_oauth_state)),
        )
    }

    fn auth_config_response(&self) -> AuthConfigResponse {
        let mut oauth_providers = Vec::new();

        if self.config.google.is_some() {
            oauth_providers.push("google".to_string());
        }
        if self.config.github.is_some() {
            oauth_providers.push("github".to_string());
        }

        AuthConfigResponse {
            mode: self.config.mode.as_str().to_string(),
            password_auth_enabled: self.config.password_auth_enabled(),
            oauth_providers,
            signup_enabled: self.config.signup_enabled(),
        }
    }
}

/// Fetch organization memberships for a user
pub(super) async fn fetch_user_organizations(
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
            role: row.role.parse::<OrgRole>().unwrap_or(OrgRole::Member),
        })
        .collect())
}

/// Return organizations as-is, or fall back to default org membership if empty
pub(super) fn organizations_or_default(organizations: Vec<OrgMembership>) -> Vec<OrgMembership> {
    if organizations.is_empty() {
        vec![OrgMembership {
            org_id: DEFAULT_ORG_ID,
            public_id: DEFAULT_ORG_PUBLIC_ID.to_string(),
            name: "Default Organization".to_string(),
            role: OrgRole::Member,
        }]
    } else {
        organizations
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a cache with custom TTL for testing.
    fn build_test_cache(ttl: Duration) -> Cache<String, AuthUser> {
        Cache::builder().max_capacity(100).time_to_live(ttl).build()
    }

    fn test_auth_user() -> AuthUser {
        AuthUser {
            id: Uuid::nil(),
            email: "test@example.com".to_string(),
            name: "Test User".to_string(),
            roles: vec!["user".to_string()],
            is_platform_user: false,
            auth_method: AuthMethod::PersonalAccessToken,
            organizations: vec![OrgMembership {
                org_id: 1,
                public_id: "org_test".to_string(),
                name: "Test Org".to_string(),
                role: OrgRole::Member,
            }],
        }
    }

    #[tokio::test]
    async fn test_cache_hit_returns_cached_user() {
        let cache = build_personal_access_token_cache();
        let user = test_auth_user();
        let key_hash = "abc123hash".to_string();

        // Insert into cache
        cache.insert(key_hash.clone(), user.clone()).await;

        // Should hit
        let cached = cache.get(&key_hash).await;
        assert!(cached.is_some());
        let cached = cached.unwrap();
        assert_eq!(cached.id, user.id);
        assert_eq!(cached.email, user.email);
        assert_eq!(cached.organizations.len(), 1);
        assert_eq!(cached.organizations[0].org_id, 1);
    }

    #[tokio::test]
    async fn test_cache_miss_returns_none() {
        let cache = build_personal_access_token_cache();
        let result = cache.get(&"nonexistent".to_string()).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_cache_invalidate_single_key() {
        let cache = build_personal_access_token_cache();
        let user = test_auth_user();
        let key_hash = "abc123hash".to_string();

        cache.insert(key_hash.clone(), user).await;
        assert!(cache.get(&key_hash).await.is_some());

        // Invalidate
        cache.invalidate(&key_hash).await;
        assert!(cache.get(&key_hash).await.is_none());
    }

    #[tokio::test]
    async fn test_cache_invalidate_all() {
        let cache = build_personal_access_token_cache();
        let user = test_auth_user();

        cache.insert("key1".to_string(), user.clone()).await;
        cache.insert("key2".to_string(), user).await;
        assert!(cache.get(&"key1".to_string()).await.is_some());
        assert!(cache.get(&"key2".to_string()).await.is_some());

        // Invalidate all
        cache.invalidate_all();
        // run_pending_tasks needed for invalidate_all to take effect
        cache.run_pending_tasks().await;
        assert!(cache.get(&"key1".to_string()).await.is_none());
        assert!(cache.get(&"key2".to_string()).await.is_none());
    }

    #[tokio::test]
    async fn test_cache_ttl_expiry() {
        // Use a very short TTL
        let cache = build_test_cache(Duration::from_millis(50));
        let user = test_auth_user();
        let key_hash = "ttl_test".to_string();

        cache.insert(key_hash.clone(), user).await;
        assert!(cache.get(&key_hash).await.is_some());

        // Wait for TTL to expire
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(cache.get(&key_hash).await.is_none());
    }

    #[tokio::test]
    async fn test_cache_different_keys_independent() {
        let cache = build_personal_access_token_cache();
        let user1 = test_auth_user();
        let mut user2 = test_auth_user();
        user2.email = "other@example.com".to_string();

        cache.insert("key_a".to_string(), user1).await;
        cache.insert("key_b".to_string(), user2).await;

        // Invalidate only key_a
        cache.invalidate(&"key_a".to_string()).await;

        assert!(cache.get(&"key_a".to_string()).await.is_none());
        let remaining = cache.get(&"key_b".to_string()).await;
        assert!(remaining.is_some());
        assert_eq!(remaining.unwrap().email, "other@example.com");
    }

    #[test]
    fn test_cache_constants() {
        assert_eq!(PAT_CACHE_TTL, Duration::from_secs(300));
        assert_eq!(PAT_CACHE_MAX_CAPACITY, 10_000);
    }

    // Regression tests for the fix that dropped read-through API-key caching.
    // See fix(auth): always revalidate API keys against DB (#1532). These tests
    // lock in that `validate_api_key` goes to the DB on every call, so revoked
    // keys and stale user state cannot re-authenticate via the in-process cache.
    mod revalidation {
        use super::super::super::backend::AuthBackend;
        use super::super::*;
        use crate::storage::StorageBackend;
        use crate::storage::models::{CreatePersonalAccessTokenRow, CreateUserRow, UpdateUser};

        async fn seed_user_with_key(
            db: &Arc<StorageBackend>,
            email: &str,
            name: &str,
        ) -> (uuid::Uuid, uuid::Uuid, String) {
            let user = db
                .create_user(CreateUserRow {
                    email: email.to_string(),
                    name: name.to_string(),
                    avatar_url: None,
                    roles: vec!["user".to_string()],
                    password_hash: None,
                    email_verified: true,
                    auth_provider: None,
                    auth_provider_id: None,
                    external_id: None,
                })
                .await
                .expect("create user");

            let generated = crate::auth::personal_access_token::generate_personal_access_token();
            let token_row = db
                .create_personal_access_token(CreatePersonalAccessTokenRow {
                    user_id: user.id,
                    name: "test-token".to_string(),
                    token_hash: generated.token_hash.clone(),
                    token_prefix: generated.token_prefix.clone(),
                    scopes: vec!["*".to_string()],
                    expires_at: None,
                    metadata: serde_json::json!({}),
                })
                .await
                .expect("create personal access token");

            (user.id, token_row.id, generated.token)
        }

        #[tokio::test]
        async fn validate_api_key_rejects_after_key_deleted_from_db() {
            let db = Arc::new(StorageBackend::in_memory());
            let backend = BuiltinAuthBackend::new(
                AuthConfig::default(),
                db.clone(),
                Arc::new(crate::platform::oss_platform_definition()),
            );
            let (user_id, key_id, plaintext_key) =
                seed_user_with_key(&db, "revoked@example.com", "Revoked User").await;

            // First call succeeds and populates the cache.
            let first = backend.validate_personal_access_token(&plaintext_key).await;
            assert!(first.is_ok(), "initial validation should succeed");

            // Delete the key from the DB but do NOT invalidate the cache. Prior to
            // the fix this would silently keep authenticating from the cache.
            let removed = db
                .delete_personal_access_token(key_id, user_id)
                .await
                .expect("delete");
            assert!(removed);

            // Re-validate: must reject because revalidation always hits the DB.
            let second = backend.validate_personal_access_token(&plaintext_key).await;
            assert!(
                second.is_err(),
                "revoked key must not authenticate even when cached"
            );
        }

        #[tokio::test]
        async fn validate_api_key_reflects_fresh_user_state_on_revalidation() {
            let db = Arc::new(StorageBackend::in_memory());
            let backend = BuiltinAuthBackend::new(
                AuthConfig::default(),
                db.clone(),
                Arc::new(crate::platform::oss_platform_definition()),
            );
            let (user_id, _key_id, plaintext_key) =
                seed_user_with_key(&db, "user@example.com", "Original Name").await;

            let first = backend
                .validate_personal_access_token(&plaintext_key)
                .await
                .expect("first validation");
            assert_eq!(first.name, "Original Name");

            // Rename the user in the DB. With read-through caching this change
            // would not surface until TTL expired.
            db.update_user(
                user_id,
                UpdateUser {
                    name: Some("Renamed User".to_string()),
                    ..Default::default()
                },
            )
            .await
            .expect("rename user");

            let second = backend
                .validate_personal_access_token(&plaintext_key)
                .await
                .expect("second validation");
            assert_eq!(
                second.name, "Renamed User",
                "validate must re-read user state from DB on every call"
            );
        }

        #[tokio::test]
        async fn validate_api_key_rejects_invalid_format_without_db_lookup() {
            let db = Arc::new(StorageBackend::in_memory());
            let backend = BuiltinAuthBackend::new(
                AuthConfig::default(),
                db.clone(),
                Arc::new(crate::platform::oss_platform_definition()),
            );

            let result = backend
                .validate_personal_access_token("not-an-api-key")
                .await;
            assert!(result.is_err(), "malformed key must be rejected");
        }
    }

    // TM-MCP-006: MCP OAuth token audience binding. Proves the OSS backend keeps
    // the `/mcp` and `/api/*` validation paths audience-isolated.
    mod mcp_token_audience {
        use super::super::super::backend::AuthBackend;
        use super::super::super::middleware::AuthMethod;
        use super::super::*;
        use crate::storage::StorageBackend;
        use crate::storage::models::CreateUserRow;

        const RESOURCE: &str = "https://app.example.com/mcp";

        async fn backend_with_user() -> (BuiltinAuthBackend, uuid::Uuid) {
            let db = Arc::new(StorageBackend::in_memory());
            let backend = BuiltinAuthBackend::new(
                AuthConfig::default(),
                db.clone(),
                Arc::new(crate::platform::oss_platform_definition()),
            );
            let user = db
                .create_user(CreateUserRow {
                    email: "mcp@example.com".to_string(),
                    name: "MCP User".to_string(),
                    avatar_url: None,
                    roles: vec!["user".to_string()],
                    password_hash: None,
                    email_verified: true,
                    auth_provider: None,
                    auth_provider_id: None,
                    external_id: None,
                })
                .await
                .expect("create user");
            (backend, user.id)
        }

        #[tokio::test]
        async fn mcp_token_rejected_by_general_validate_token() {
            // (a) An mcp_access token must NOT authenticate the general /api/*
            // path — this is the confused-deputy fix.
            let (backend, user_id) = backend_with_user().await;
            let token = backend
                .jwt_service
                .generate_mcp_access_token(user_id, "mcp@example.com", "MCP User", &[], RESOURCE)
                .unwrap();

            assert!(
                backend.validate_token(&token).await.is_err(),
                "mcp_access token must be rejected by /api/* validate_token"
            );
        }

        #[tokio::test]
        async fn regular_access_token_rejected_by_validate_mcp_token() {
            // (b) A normal access token must NOT authenticate the /mcp path.
            let (backend, user_id) = backend_with_user().await;
            let token = backend
                .jwt_service
                .generate_access_token(user_id, "mcp@example.com", "MCP User", &[])
                .unwrap();

            assert!(
                backend.validate_mcp_token(&token, RESOURCE).await.is_err(),
                "regular access token must be rejected at /mcp"
            );
        }

        #[tokio::test]
        async fn mcp_token_accepted_at_mcp_path_with_mcp_auth_method() {
            let (backend, user_id) = backend_with_user().await;
            let token = backend
                .jwt_service
                .generate_mcp_access_token(user_id, "mcp@example.com", "MCP User", &[], RESOURCE)
                .unwrap();

            let user = backend
                .validate_mcp_token(&token, RESOURCE)
                .await
                .expect("mcp token must validate at /mcp");
            assert_eq!(user.id, user_id);
            assert_eq!(user.auth_method, AuthMethod::Mcp);
        }

        #[tokio::test]
        async fn mcp_token_rejected_for_wrong_resource() {
            let (backend, user_id) = backend_with_user().await;
            let token = backend
                .jwt_service
                .generate_mcp_access_token(user_id, "mcp@example.com", "MCP User", &[], RESOURCE)
                .unwrap();

            assert!(
                backend
                    .validate_mcp_token(&token, "https://evil.example.com/mcp")
                    .await
                    .is_err(),
                "mcp token bound to another resource must be rejected"
            );
        }
    }
}

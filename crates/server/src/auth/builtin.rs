// Built-in authentication backend (JWT + password + OAuth + API keys)
// Decision: Default for OSS. All existing behavior preserved.
// Decision: API key auth results cached in-process (moka, 5-min TTL) to avoid
// 4 sequential DB queries per request. Invalidated on key deletion.

use async_trait::async_trait;
use axum::Router;
use everruns_core::{DEFAULT_ORG_ID, DEFAULT_ORG_PUBLIC_ID, OrgMembership, OrgRole};
use moka::future::Cache;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

use super::api_key::{ValidatedApiKey, hash_api_key, is_valid_api_key_format};
use super::backend::AuthBackend;
use super::config::AuthConfig;
use super::jwt::JwtService;
use super::middleware::{AuthError, AuthMethod, AuthUser};
use super::rate_limit::AuthRateLimiter;
use super::routes::{self, AuthConfigResponse};
use crate::storage::StorageBackend;
use crate::valkey::ValkeyClient;

/// TTL for cached API key auth results.
const API_KEY_CACHE_TTL: Duration = Duration::from_secs(300); // 5 minutes

/// Max entries in the API key auth cache.
const API_KEY_CACHE_MAX_CAPACITY: u64 = 10_000;

/// Built-in authentication backend (JWT + password + OAuth + API keys).
/// This is the default for OSS deployments.
#[derive(Clone)]
pub struct BuiltinAuthBackend {
    pub config: AuthConfig,
    pub jwt_service: Arc<JwtService>,
    pub db: Arc<StorageBackend>,
    pub rate_limiter: AuthRateLimiter,
    pub resource_limits: crate::server::ResourceLimitsConfig,
    /// In-process cache: key_hash -> AuthUser. Avoids 4 sequential DB queries per API-key request.
    api_key_cache: Cache<String, AuthUser>,
}

fn build_api_key_cache() -> Cache<String, AuthUser> {
    Cache::builder()
        .max_capacity(API_KEY_CACHE_MAX_CAPACITY)
        .time_to_live(API_KEY_CACHE_TTL)
        .build()
}

fn root_url_from_api_base(api_base_url: &str) -> String {
    let trimmed = api_base_url.trim_end_matches('/');
    if let Ok(api_prefix) = std::env::var("API_PREFIX") {
        let api_prefix = api_prefix.trim_end_matches('/');
        if !api_prefix.is_empty() && trimmed.ends_with(api_prefix) {
            return trimmed[..trimmed.len() - api_prefix.len()].to_string();
        }
    }
    trimmed.strip_suffix("/api").unwrap_or(trimmed).to_string()
}

impl BuiltinAuthBackend {
    /// Create with in-memory rate limiting (per-instance).
    pub fn new(config: AuthConfig, db: Arc<StorageBackend>) -> Self {
        let jwt_service = Arc::new(JwtService::new(config.jwt.clone()));
        Self {
            config,
            jwt_service,
            db,
            rate_limiter: AuthRateLimiter::new(),
            resource_limits: crate::server::ResourceLimitsConfig::from_env(),
            api_key_cache: build_api_key_cache(),
        }
    }

    /// Create with Valkey-backed distributed rate limiting.
    pub fn with_valkey(config: AuthConfig, db: Arc<StorageBackend>, valkey: ValkeyClient) -> Self {
        let jwt_service = Arc::new(JwtService::new(config.jwt.clone()));
        Self {
            config,
            jwt_service,
            db,
            rate_limiter: AuthRateLimiter::with_valkey(valkey),
            resource_limits: crate::server::ResourceLimitsConfig::from_env(),
            api_key_cache: build_api_key_cache(),
        }
    }

    /// Invalidate a cached API key entry by its hash.
    pub async fn invalidate_api_key_cache(&self, key_hash: &str) {
        self.api_key_cache.invalidate(key_hash).await;
    }

    /// Invalidate all cached API key entries (e.g. after user/org updates).
    pub fn invalidate_all_api_key_cache(&self) {
        self.api_key_cache.invalidate_all();
    }

    /// Entry count in the cache (for testing/metrics).
    #[cfg(test)]
    pub fn api_key_cache_entry_count(&self) -> u64 {
        self.api_key_cache.entry_count()
    }

    /// Validate API key against DB (4 sequential queries). Called on cache miss.
    async fn validate_api_key_from_db(&self, key_hash: &str) -> Result<AuthUser, AuthError> {
        let api_key_row = self
            .db
            .get_api_key_by_hash(key_hash)
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
        let db = self.db.clone();
        let key_id = api_key_row.id;
        tokio::spawn(async move {
            let _ = db.update_api_key_last_used(key_id).await;
        });

        // Fetch user info
        let user = self
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
        let org = self
            .db
            .get_organization(api_key_row.org_id)
            .await
            .map_err(|e| {
                tracing::error!("Failed to fetch org for API key: {}", e);
                AuthError::unauthorized("Failed to validate API key")
            })?
            .ok_or_else(|| AuthError::unauthorized("Organization not found for API key"))?;

        // Get the user's role in this org (default to member for API key auth)
        let member_row = self
            .db
            .get_organization_member(org.org_id, user.id)
            .await
            .ok()
            .flatten();
        let role = member_row
            .and_then(|m| m.role.parse::<OrgRole>().ok())
            .unwrap_or(OrgRole::Member);

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
                role,
            }],
        })
    }
}

#[async_trait]
impl AuthBackend for BuiltinAuthBackend {
    async fn validate_token(&self, token: &str) -> Result<AuthUser, AuthError> {
        let claims = self.jwt_service.validate_access_token(token).map_err(|e| {
            tracing::debug!("JWT validation failed: {}", e);
            AuthError::unauthorized("Invalid or expired token")
        })?;

        let user_id = Uuid::parse_str(&claims.sub)
            .map_err(|_| AuthError::unauthorized("Invalid user ID in token"))?;

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

        Ok(AuthUser {
            id: user_id,
            email: claims.email,
            name: claims.name,
            roles: claims.roles,
            auth_method: AuthMethod::Jwt,
            organizations,
        })
    }

    async fn validate_api_key(&self, key: &str) -> Result<AuthUser, AuthError> {
        if !is_valid_api_key_format(key) {
            return Err(AuthError::unauthorized("Invalid API key format"));
        }

        let key_hash = hash_api_key(key);

        // Check cache first — avoids 4 sequential DB queries on hit
        if let Some(cached) = self.api_key_cache.get(&key_hash).await {
            tracing::trace!("API key auth cache hit");
            return Ok(cached);
        }

        let auth_user = self.validate_api_key_from_db(&key_hash).await?;

        // Populate cache
        self.api_key_cache.insert(key_hash, auth_user.clone()).await;

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

    fn on_api_key_deleted(&self) {
        self.invalidate_all_api_key_cache();
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
            auth_method: AuthMethod::ApiKey,
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
        let cache = build_api_key_cache();
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
        let cache = build_api_key_cache();
        let result = cache.get(&"nonexistent".to_string()).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_cache_invalidate_single_key() {
        let cache = build_api_key_cache();
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
        let cache = build_api_key_cache();
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
        let cache = build_api_key_cache();
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
        assert_eq!(API_KEY_CACHE_TTL, Duration::from_secs(300));
        assert_eq!(API_KEY_CACHE_MAX_CAPACITY, 10_000);
    }
}

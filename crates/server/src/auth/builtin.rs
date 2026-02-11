// Built-in authentication backend (JWT + password + OAuth + API keys)
// Decision: Default for OSS. All existing behavior preserved.

use async_trait::async_trait;
use axum::Router;
use everruns_core::{DEFAULT_ORG_ID, DEFAULT_ORG_PUBLIC_ID, OrgMembership};
use std::sync::Arc;
use uuid::Uuid;

use super::api_key::{ValidatedApiKey, hash_api_key, is_valid_api_key_format};
use super::backend::AuthBackend;
use super::config::{AuthConfig, AuthMode};
use super::jwt::JwtService;
use super::middleware::{AuthError, AuthMethod, AuthUser};
use super::routes::{self, AuthConfigResponse};
use crate::storage::StorageBackend;

/// Built-in authentication backend (JWT + password + OAuth + API keys).
/// This is the default for OSS deployments.
#[derive(Clone)]
pub struct BuiltinAuthBackend {
    pub config: AuthConfig,
    pub jwt_service: Arc<JwtService>,
    pub db: Arc<StorageBackend>,
}

impl BuiltinAuthBackend {
    pub fn new(config: AuthConfig, db: Arc<StorageBackend>) -> Self {
        let jwt_service = Arc::new(JwtService::new(config.jwt.clone()));
        Self {
            config,
            jwt_service,
            db,
        }
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
        let organizations = if organizations.is_empty() {
            tracing::warn!(
                user_id = %user_id,
                "User has no organizations, falling back to default org"
            );
            vec![OrgMembership {
                org_id: DEFAULT_ORG_ID,
                public_id: DEFAULT_ORG_PUBLIC_ID.to_string(),
                name: "Default Organization".to_string(),
            }]
        } else {
            organizations
        };

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

        let api_key_row = self
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

    fn auth_routes(&self) -> Option<Router> {
        Some(routes::routes(self.clone()))
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
            mode: match self.config.mode {
                AuthMode::None => "none".to_string(),
                AuthMode::Admin => "admin".to_string(),
                AuthMode::Full => "full".to_string(),
            },
            password_auth_enabled: self.config.password_auth_enabled(),
            oauth_providers,
            signup_enabled: self.config.mode != AuthMode::Admin && !self.config.disable_signup,
        }
    }
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

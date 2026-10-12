// Per-request check that an MCP access token's grant is still live.
//
// Spec: knowledge/integrations/mcp-connected-clients.md (phase 1).
//
// Decision: the check sits in `BuiltinAuthBackend::validate_mcp_token`, the one
// place MCP OAuth bearer tokens are validated. Every auth mode reaches it,
// including `AuthMode::External`: hosted wrappers delegate MCP tokens to the
// built-in backend because the OSS token endpoint mints them.
//
// Decision: verdicts are cached in-process for `GRANT_CACHE_TTL` (30s), so a
// revoke reaches every replica within that window without a shared cache or a
// fan-out. Revoked and missing grants are cached too: a grant id is never
// revived (re-approval creates a new grant), so neither verdict can go stale
// in the dangerous direction. A lookup error is never cached and rejects the
// request (fail closed).
//
// Decision: tokens without a `grant_id` were minted before grants existed (or by
// a replica still on the previous release) and are accepted until they expire.
// Access tokens are short-lived, and their refresh tokens were backfilled into
// grants, so the next refresh yields a grant-bound token.

use std::sync::Arc;
use std::time::Duration;

use moka::future::Cache;
use uuid::Uuid;

use super::jwt::AccessTokenClaims;
use super::middleware::AuthError;
use crate::storage::StorageBackend;

/// How long a grant verdict is reused before the database is asked again. The
/// upper bound on how long a revoked client keeps working.
const GRANT_CACHE_TTL: Duration = Duration::from_secs(30);

/// `last_used_at` is written at most this often per grant.
const LAST_USED_THROTTLE: Duration = Duration::from_secs(300);

const GRANT_CACHE_MAX_CAPACITY: u64 = 50_000;

/// What the database said about a grant id.
#[derive(Debug, Clone)]
enum GrantVerdict {
    /// The grant exists and is not revoked.
    Active { user_id: Uuid, client_id: String },
    /// Revoked, or no such grant.
    Rejected,
}

#[derive(Clone)]
pub struct McpGrantGuard {
    db: Arc<StorageBackend>,
    verdicts: Cache<Uuid, GrantVerdict>,
    /// Grants whose `last_used_at` was written within `LAST_USED_THROTTLE`.
    recently_touched: Cache<Uuid, ()>,
}

impl McpGrantGuard {
    pub fn new(db: Arc<StorageBackend>) -> Self {
        Self::with_ttl(db, GRANT_CACHE_TTL)
    }

    fn with_ttl(db: Arc<StorageBackend>, ttl: Duration) -> Self {
        Self {
            db,
            verdicts: Cache::builder()
                .max_capacity(GRANT_CACHE_MAX_CAPACITY)
                .time_to_live(ttl)
                .build(),
            recently_touched: Cache::builder()
                .max_capacity(GRANT_CACHE_MAX_CAPACITY)
                .time_to_live(LAST_USED_THROTTLE)
                .build(),
        }
    }

    /// Accept or reject an already signature-checked `mcp_access` token on
    /// the strength of its grant.
    pub async fn check(&self, claims: &AccessTokenClaims) -> Result<(), AuthError> {
        let Some(grant_id) = claims.grant_id else {
            return Ok(());
        };

        let verdict = match self.verdicts.get(&grant_id).await {
            Some(verdict) => verdict,
            None => {
                let row = self.db.get_oauth_grant(grant_id).await.map_err(|error| {
                    tracing::error!(%grant_id, %error, "MCP grant lookup failed; rejecting");
                    AuthError::internal("Could not verify MCP client authorization")
                })?;
                let verdict = match row {
                    Some(row) if row.revoked_at.is_none() => GrantVerdict::Active {
                        user_id: row.user_id,
                        client_id: row.client_id,
                    },
                    _ => GrantVerdict::Rejected,
                };
                self.verdicts.insert(grant_id, verdict.clone()).await;
                verdict
            }
        };

        let GrantVerdict::Active { user_id, client_id } = verdict else {
            tracing::debug!(%grant_id, "MCP token names a revoked or unknown grant");
            return Err(AuthError::unauthorized("Invalid or expired MCP token"));
        };
        // A token is bound to the grant's user and client. A mismatch means a
        // forged or mis-minted token, never a legitimate one.
        if claims.sub != user_id.to_string() || claims.client_id.as_deref() != Some(&client_id) {
            tracing::warn!(%grant_id, "MCP token claims do not match its grant");
            return Err(AuthError::unauthorized("Invalid or expired MCP token"));
        }

        self.touch_last_used(grant_id).await;
        Ok(())
    }

    /// Best effort: a failed write only leaves `last_used_at` a little old.
    async fn touch_last_used(&self, grant_id: Uuid) {
        if self.recently_touched.contains_key(&grant_id) {
            return;
        }
        self.recently_touched.insert(grant_id, ()).await;
        if let Err(error) = self.db.touch_oauth_grant_last_used(grant_id).await {
            tracing::warn!(%grant_id, %error, "failed to update MCP grant last_used_at");
        }
    }

    /// Drop a cached verdict. Lets a revoke served by this replica take effect
    /// here immediately; other replicas catch up within `GRANT_CACHE_TTL`.
    pub async fn forget(&self, grant_id: Uuid) {
        self.verdicts.invalidate(&grant_id).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::config::JwtConfig;
    use crate::auth::jwt::{JwtService, McpTokenGrant};
    use crate::storage::{CreateOAuthClientRow, CreateUserRow};

    const RESOURCE: &str = "https://app.example.com/mcp";

    async fn fixture() -> (Arc<StorageBackend>, Uuid, String) {
        let db = Arc::new(StorageBackend::test_database());
        let user = db
            .create_user(CreateUserRow {
                email: "grant-guard@example.com".to_string(),
                name: "Guard User".to_string(),
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
        let client_id = format!("mcp_client_{}", Uuid::now_v7().simple());
        db.create_oauth_client(CreateOAuthClientRow {
            client_id: client_id.clone(),
            client_secret_hash: "hash".to_string(),
            client_name: "Cursor".to_string(),
            redirect_uris: serde_json::json!(["http://127.0.0.1:3000/callback"]),
        })
        .await
        .expect("create client");
        (db, user.id, client_id)
    }

    fn claims_for(user_id: Uuid, client_id: &str, grant_id: Uuid) -> AccessTokenClaims {
        let jwt = JwtService::new(JwtConfig::default());
        let token = jwt
            .generate_mcp_access_token_for_grant(
                user_id,
                "grant-guard@example.com",
                "Guard User",
                &[],
                RESOURCE,
                McpTokenGrant {
                    client_id,
                    grant_id,
                },
            )
            .expect("mint");
        jwt.validate_mcp_access_token(&token, RESOURCE)
            .expect("valid")
    }

    #[tokio::test]
    async fn active_grant_is_accepted_and_stamps_last_used() {
        let (db, user_id, client_id) = fixture().await;
        let grant = db.approve_oauth_grant(&client_id, user_id).await.unwrap();
        let guard = McpGrantGuard::new(db.clone());

        guard
            .check(&claims_for(user_id, &client_id, grant.id))
            .await
            .expect("active grant accepted");

        let stored = db.get_oauth_grant(grant.id).await.unwrap().unwrap();
        assert!(
            stored.last_used_at.is_some(),
            "check must stamp last_used_at"
        );
    }

    #[tokio::test]
    async fn revoked_grant_is_rejected_once_the_cache_expires() {
        let (db, user_id, client_id) = fixture().await;
        let grant = db.approve_oauth_grant(&client_id, user_id).await.unwrap();
        let guard = McpGrantGuard::with_ttl(db.clone(), Duration::from_millis(50));
        let claims = claims_for(user_id, &client_id, grant.id);
        guard.check(&claims).await.expect("accepted before revoke");

        db.revoke_oauth_grant(grant.id, user_id)
            .await
            .unwrap()
            .expect("revoked");
        tokio::time::sleep(Duration::from_millis(120)).await;

        let err = guard.check(&claims).await.expect_err("revoked grant");
        assert_eq!(err.status, axum::http::StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn forget_applies_a_local_revoke_immediately() {
        let (db, user_id, client_id) = fixture().await;
        let grant = db.approve_oauth_grant(&client_id, user_id).await.unwrap();
        let guard = McpGrantGuard::new(db.clone());
        let claims = claims_for(user_id, &client_id, grant.id);
        guard.check(&claims).await.expect("accepted before revoke");

        db.revoke_oauth_grant(grant.id, user_id).await.unwrap();
        guard.forget(grant.id).await;

        assert!(guard.check(&claims).await.is_err());
    }

    #[tokio::test]
    async fn unknown_grant_and_mismatched_claims_are_rejected() {
        let (db, user_id, client_id) = fixture().await;
        let grant = db.approve_oauth_grant(&client_id, user_id).await.unwrap();
        let guard = McpGrantGuard::new(db.clone());

        let unknown = claims_for(user_id, &client_id, Uuid::now_v7());
        assert!(guard.check(&unknown).await.is_err());

        let other_client = claims_for(user_id, "mcp_client_other", grant.id);
        assert!(guard.check(&other_client).await.is_err());

        let other_user = claims_for(Uuid::now_v7(), &client_id, grant.id);
        assert!(guard.check(&other_user).await.is_err());
    }

    #[tokio::test]
    async fn token_without_grant_is_accepted_without_a_lookup() {
        let (db, user_id, _) = fixture().await;
        let jwt = JwtService::new(JwtConfig::default());
        let token = jwt
            .generate_mcp_access_token(user_id, "a@example.com", "A", &[], RESOURCE)
            .unwrap();
        let claims = jwt.validate_mcp_access_token(&token, RESOURCE).unwrap();
        McpGrantGuard::new(db)
            .check(&claims)
            .await
            .expect("pre-grant token accepted until it expires");
    }

    #[tokio::test]
    async fn lookup_failure_rejects_and_is_not_cached() {
        let (db, user_id, client_id) = fixture().await;
        let grant = db.approve_oauth_grant(&client_id, user_id).await.unwrap();
        let guard = McpGrantGuard::new(db.clone());
        let claims = claims_for(user_id, &client_id, grant.id);

        // Make every grant lookup fail, as a database outage would.
        sqlx::query("ALTER TABLE oauth_grants RENAME TO oauth_grants_unavailable")
            .execute(db.pool())
            .await
            .unwrap();
        let err = guard.check(&claims).await.expect_err("fail closed");
        assert_eq!(err.status, axum::http::StatusCode::INTERNAL_SERVER_ERROR);

        // The failure was not cached as a verdict: once the database is back,
        // the grant is accepted again.
        sqlx::query("ALTER TABLE oauth_grants_unavailable RENAME TO oauth_grants")
            .execute(db.pool())
            .await
            .unwrap();
        guard.check(&claims).await.expect("accepted after recovery");
    }
}

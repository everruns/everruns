// Lazy user connection token resolver
//
// Resolves connection tokens (e.g. GitHub) at tool execution time instead of
// eagerly injecting at session creation. This means:
// - Tokens are always fresh (reconnect mid-session works)
// - Sessions created before connecting still get tokens
// - Tools can show helpful guidance when not connected
//
// GitHub App connections: mints a fresh 1h installation token on each request.
// Legacy OAuth connections: decrypts the stored token.

use crate::kernel_imports::{
    EgressService, McpServerAuthMode, everruns_provider::error::AgentLoopError,
    everruns_provider::error::Result,
};
use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use chrono::{DateTime, Duration, Utc};
use everruns_core::connection_services::UserConnectionResolver;
use everruns_provider::typed_id::SessionId;
use moka::sync::Cache;
use std::sync::Arc;
use std::time::Duration as StdDuration;
use tokio::sync::Mutex;
use uuid::Uuid;

use super::backend::StorageBackend;
use super::encryption::EncryptionService;
use super::models::{
    AgentIdentityConnectionRow, McpOAuthSessionCredentialsRow, UpdateOAuthConnectionTokens,
    UpsertMcpOAuthSessionCredentials, UserConnectionRow,
};
use crate::auth::oauth::GitHubAppService;
use crate::domains::mcp_servers::{McpServerOAuthSettings, McpServerService};
use crate::oauth_client::{
    EgressOAuthRefreshExchange, OAuthRefreshError, OAuthRefreshExchange, OAuthRefreshRequest,
    OAuthTokenResponse,
};

const OAUTH_REFRESH_SKEW: Duration = Duration::seconds(60);
const REFRESH_LOCK_MAX_CAPACITY: u64 = 10_000;
const REFRESH_LOCK_IDLE_TTL: StdDuration = StdDuration::from_secs(10 * 60);

/// Resolves connection tokens for tool execution.
///
/// Session-based lookup priority:
/// 1. If the session has an `agent_identity_id`, resolves from `agent_identity_connections`.
/// 2. Falls back to `user_connections` for the session's resolved owner user.
///
/// Leased-resource cleanup additionally uses explicit owner-user lookups so
/// the same provider identity that created a resource can delete it later.
#[derive(Clone)]
pub struct DbConnectionResolver {
    db: StorageBackend,
    encryption: EncryptionService,
    /// GitHub App service for minting installation tokens (None = legacy OAuth only)
    github_app: Option<GitHubAppTokenMinter>,
    oauth_refresh: Arc<dyn OAuthRefreshExchange>,
    // THREAT[TM-TOOL-025]: per-grant single-flight locks prevent refresh
    // stampedes; the bounded idle cache prevents session-id memory exhaustion.
    refresh_locks: Cache<String, Arc<Mutex<()>>>,
}

#[derive(Debug)]
struct OAuthClientConfig {
    token_endpoint: String,
    client_id: String,
    client_secret: Option<String>,
}

/// Handles GitHub App JWT signing and installation token minting.
/// Cloneable so the resolver can be shared across tool executions.
#[derive(Clone)]
pub struct GitHubAppTokenMinter {
    app_id: String,
    private_key: String,
}

impl GitHubAppTokenMinter {
    pub fn new(app_id: String, private_key: String) -> Self {
        Self {
            app_id,
            private_key,
        }
    }

    /// Mint a fresh installation access token (1h TTL).
    async fn mint_token(&self, installation_id: i64) -> std::result::Result<String, String> {
        use crate::auth::config::GitHubConnectionConfig;

        // Build a minimal config for GitHubAppService
        let config = GitHubConnectionConfig {
            app_id: self.app_id.clone(),
            private_key: self.private_key.clone(),
            app_slug: String::new(),
            setup_url: String::new(),
        };
        let service = GitHubAppService::new(&config);
        service
            .mint_installation_token(installation_id)
            .await
            .map_err(|e| format!("Failed to mint GitHub installation token: {e}"))
    }
}

impl DbConnectionResolver {
    pub fn new(
        db: StorageBackend,
        encryption: EncryptionService,
        github_app: Option<GitHubAppTokenMinter>,
        egress: Arc<dyn EgressService>,
    ) -> Self {
        Self::with_oauth_refresh(
            db,
            encryption,
            github_app,
            Arc::new(EgressOAuthRefreshExchange::new(egress)),
        )
    }

    fn with_oauth_refresh(
        db: StorageBackend,
        encryption: EncryptionService,
        github_app: Option<GitHubAppTokenMinter>,
        oauth_refresh: Arc<dyn OAuthRefreshExchange>,
    ) -> Self {
        Self {
            db,
            encryption,
            github_app,
            oauth_refresh,
            refresh_locks: Cache::builder()
                .max_capacity(REFRESH_LOCK_MAX_CAPACITY)
                .time_to_idle(REFRESH_LOCK_IDLE_TTL)
                .build(),
        }
    }

    fn parse_mcp_oauth_provider(provider: &str) -> Option<Uuid> {
        provider.strip_prefix("mcp_oauth_")?.parse().ok()
    }

    fn decrypt(&self, encrypted: &[u8], label: &str) -> Result<String> {
        self.encryption
            .decrypt_to_string(encrypted)
            .map_err(|e| AgentLoopError::store(format!("Failed to decrypt {label}: {e}")))
    }

    fn needs_refresh(&self, expires_at_encrypted: Option<&[u8]>) -> Result<bool> {
        let Some(encrypted) = expires_at_encrypted else {
            return Ok(false);
        };
        let value = self.decrypt(encrypted, "OAuth token expiry")?;
        let expires_at = DateTime::parse_from_rfc3339(&value)
            .map_err(|e| AgentLoopError::store(format!("Invalid OAuth token expiry: {e}")))?
            .with_timezone(&Utc);
        Ok(expires_at <= Utc::now() + OAUTH_REFRESH_SKEW)
    }

    fn user_connection_needs_refresh(&self, row: &UserConnectionRow) -> bool {
        row.expires_at
            .is_some_and(|expires_at| expires_at <= Utc::now() + OAUTH_REFRESH_SKEW)
    }

    async fn oauth_client_config(
        &self,
        session_id: SessionId,
        server_id: Uuid,
    ) -> Result<Option<OAuthClientConfig>> {
        // THREAT[TM-TENANT-001]: derive the org from the active session, then
        // scope the OAuth server lookup to that org.
        let session = self
            .db
            .get_session_unscoped(session_id)
            .await
            .map_err(|e| AgentLoopError::store(format!("Failed to resolve OAuth session: {e}")))?;
        let Some(session) = session else {
            return Ok(None);
        };
        let row = self
            .db
            .get_mcp_server(session.org_id, server_id)
            .await
            .map_err(|e| AgentLoopError::store(format!("Failed to resolve OAuth server: {e}")))?;
        let Some(row) = row else {
            return Ok(None);
        };
        let settings = McpServerService::settings_from_row(&row);
        if settings.auth_mode != McpServerAuthMode::OAuth {
            return Ok(None);
        }
        self.oauth_client_config_from_settings(settings.oauth.as_ref())
    }

    fn oauth_client_config_from_settings(
        &self,
        oauth: Option<&McpServerOAuthSettings>,
    ) -> Result<Option<OAuthClientConfig>> {
        let Some(oauth) = oauth else {
            return Ok(None);
        };
        let (Some(token_endpoint), Some(client_id)) =
            (oauth.token_endpoint.clone(), oauth.client_id.clone())
        else {
            return Ok(None);
        };
        let client_secret = oauth
            .client_secret_encrypted
            .as_deref()
            .map(|value| {
                let encrypted = BASE64_STANDARD.decode(value).map_err(|e| {
                    AgentLoopError::store(format!("Invalid OAuth client secret encoding: {e}"))
                })?;
                self.decrypt(&encrypted, "OAuth client secret")
            })
            .transpose()?;
        Ok(Some(OAuthClientConfig {
            token_endpoint,
            client_id,
            client_secret,
        }))
    }

    async fn exchange_refresh(
        &self,
        config: OAuthClientConfig,
        refresh_token: String,
    ) -> std::result::Result<OAuthTokenResponse, OAuthRefreshError> {
        match self
            .oauth_refresh
            .exchange(OAuthRefreshRequest {
                token_endpoint: config.token_endpoint,
                client_id: config.client_id,
                client_secret: config.client_secret,
                refresh_token,
            })
            .await
        {
            Ok(token) => Ok(token),
            Err(OAuthRefreshError::InvalidGrant) => Err(OAuthRefreshError::InvalidGrant),
            Err(OAuthRefreshError::Failed(status)) => {
                // Do not expose provider responses or credentials. Returning no
                // token preserves the existing connection_required behavior.
                tracing::warn!(%status, "MCP OAuth token refresh failed");
                Err(OAuthRefreshError::Failed(status))
            }
        }
    }

    fn expiry_from_response(token: &OAuthTokenResponse) -> Option<DateTime<Utc>> {
        token
            .expires_in
            .map(|seconds| Utc::now() + Duration::seconds(seconds))
    }

    async fn resolve_session_oauth_token(
        &self,
        session_id: SessionId,
        server_id: Uuid,
        credentials: McpOAuthSessionCredentialsRow,
    ) -> Result<Option<String>> {
        if !self.needs_refresh(credentials.expires_at_encrypted.as_deref())? {
            return self
                .decrypt(&credentials.access_token_encrypted, "OAuth access token")
                .map(Some);
        }

        let key = format!("session:{session_id}:{server_id}");
        let lock = self
            .refresh_locks
            .get_with(key, || Arc::new(Mutex::new(())));
        let _guard = lock.lock().await;

        // Re-read after taking the lock: another waiter may have refreshed it.
        let Some(credentials) = self
            .db
            .get_mcp_oauth_session_credentials(session_id, server_id)
            .await
            .map_err(|e| AgentLoopError::store(format!("Failed to resolve OAuth grant: {e}")))?
        else {
            return Ok(None);
        };
        if !self.needs_refresh(credentials.expires_at_encrypted.as_deref())? {
            return self
                .decrypt(&credentials.access_token_encrypted, "OAuth access token")
                .map(Some);
        }
        let Some(refresh_token_encrypted) = credentials.refresh_token_encrypted else {
            return Ok(None);
        };
        let refresh_token = self.decrypt(&refresh_token_encrypted, "OAuth refresh token")?;
        let Some(config) = self.oauth_client_config(session_id, server_id).await? else {
            return Ok(None);
        };
        let Ok(token) = self.exchange_refresh(config, refresh_token.clone()).await else {
            return Ok(None);
        };
        let rotated_refresh = token.refresh_token.as_deref().unwrap_or(&refresh_token);
        let expires_at = Self::expiry_from_response(&token);
        // THREAT[TM-TOOL-025]: replace the complete rotated grant in one
        // storage transaction before exposing the new access token.
        self.db
            .upsert_mcp_oauth_session_credentials(UpsertMcpOAuthSessionCredentials {
                session_id,
                server_id,
                access_token_encrypted: self
                    .encryption
                    .encrypt_string(&token.access_token)
                    .map_err(|e| {
                        AgentLoopError::store(format!("Failed to encrypt OAuth access token: {e}"))
                    })?,
                refresh_token_encrypted: Some(
                    self.encryption
                        .encrypt_string(rotated_refresh)
                        .map_err(|e| {
                            AgentLoopError::store(format!(
                                "Failed to encrypt OAuth refresh token: {e}"
                            ))
                        })?,
                ),
                expires_at_encrypted: expires_at
                    .map(|value| self.encryption.encrypt_string(&value.to_rfc3339()))
                    .transpose()
                    .map_err(|e| {
                        AgentLoopError::store(format!("Failed to encrypt OAuth token expiry: {e}"))
                    })?,
            })
            .await
            .map_err(|e| {
                AgentLoopError::store(format!("Failed to persist refreshed OAuth grant: {e}"))
            })?;
        Ok(Some(token.access_token))
    }

    async fn resolve_user_oauth_token(
        &self,
        session_id: SessionId,
        server_id: Uuid,
        user_id: Uuid,
        row: UserConnectionRow,
    ) -> Result<Option<String>> {
        let Some(access_token_encrypted) = row.access_token_encrypted.as_deref() else {
            return Ok(None);
        };
        if row.connection_type != "oauth" || !self.user_connection_needs_refresh(&row) {
            return self
                .decrypt(access_token_encrypted, "OAuth access token")
                .map(Some);
        }

        let key = format!("user-connection:{}", row.id);
        let lock = self
            .refresh_locks
            .get_with(key, || Arc::new(Mutex::new(())));
        let _guard = lock.lock().await;
        let Some(row) = self
            .db
            .get_user_connection(user_id, &row.provider)
            .await
            .map_err(|e| AgentLoopError::store(format!("Failed to resolve OAuth grant: {e}")))?
        else {
            return Ok(None);
        };
        let Some(access_token_encrypted) = row.access_token_encrypted.as_deref() else {
            return Ok(None);
        };
        if !self.user_connection_needs_refresh(&row) {
            return self
                .decrypt(access_token_encrypted, "OAuth access token")
                .map(Some);
        }
        let Some(refresh_token_encrypted) = row.refresh_token_encrypted.as_deref() else {
            return Ok(None);
        };
        let refresh_token = self.decrypt(refresh_token_encrypted, "OAuth refresh token")?;
        let Some(config) = self.oauth_client_config(session_id, server_id).await? else {
            return Ok(None);
        };
        let Ok(token) = self.exchange_refresh(config, refresh_token.clone()).await else {
            return Ok(None);
        };
        let rotated_refresh = token.refresh_token.as_deref().unwrap_or(&refresh_token);
        let fresh_access_token = token.access_token.clone();
        // THREAT[TM-TOOL-025]: replace the complete rotated grant atomically
        // before exposing the new access token.
        let updated = self
            .db
            .update_user_connection_oauth_tokens(UpdateOAuthConnectionTokens {
                connection_id: row.id,
                access_token_encrypted: self
                    .encryption
                    .encrypt_string(&fresh_access_token)
                    .map_err(|e| {
                        AgentLoopError::store(format!("Failed to encrypt OAuth access token: {e}"))
                    })?,
                refresh_token_encrypted: self.encryption.encrypt_string(rotated_refresh).map_err(
                    |e| {
                        AgentLoopError::store(format!("Failed to encrypt OAuth refresh token: {e}"))
                    },
                )?,
                expires_at: Self::expiry_from_response(&token),
                scopes: token.scope,
            })
            .await
            .map_err(|e| {
                AgentLoopError::store(format!("Failed to persist refreshed OAuth grant: {e}"))
            })?;
        if updated.is_none() {
            return Ok(None);
        }
        Ok(Some(fresh_access_token))
    }

    async fn resolve_identity_oauth_token(
        &self,
        session_id: SessionId,
        server_id: Uuid,
        row: AgentIdentityConnectionRow,
    ) -> Result<Option<String>> {
        let Some(access_token_encrypted) = row.access_token_encrypted.as_deref() else {
            return Ok(None);
        };
        if row.connection_type != "oauth"
            || row
                .expires_at
                .is_none_or(|expiry| expiry > Utc::now() + OAUTH_REFRESH_SKEW)
        {
            return self
                .decrypt(access_token_encrypted, "OAuth access token")
                .map(Some);
        }

        let key = format!("identity-connection:{}", row.id);
        let lock = self
            .refresh_locks
            .get_with(key, || Arc::new(Mutex::new(())));
        let _guard = lock.lock().await;
        let Some(row) = self
            .db
            .get_agent_identity_connection_row_for_session(session_id, &row.provider)
            .await
            .map_err(|e| AgentLoopError::store(format!("Failed to resolve identity grant: {e}")))?
        else {
            return Ok(None);
        };
        let Some(access_token_encrypted) = row.access_token_encrypted.as_deref() else {
            return Ok(None);
        };
        if row
            .expires_at
            .is_none_or(|expiry| expiry > Utc::now() + OAUTH_REFRESH_SKEW)
        {
            return self
                .decrypt(access_token_encrypted, "OAuth access token")
                .map(Some);
        }
        let Some(refresh_token_encrypted) = row.refresh_token_encrypted.as_deref() else {
            return Ok(None);
        };
        let refresh_token = self.decrypt(refresh_token_encrypted, "OAuth refresh token")?;
        let Some(config) = self.oauth_client_config(session_id, server_id).await? else {
            return Ok(None);
        };
        let token = match self.exchange_refresh(config, refresh_token.clone()).await {
            Ok(token) => token,
            Err(OAuthRefreshError::InvalidGrant) => {
                self.db
                    .delete_agent_identity_connection(row.agent_identity_id, &row.provider)
                    .await
                    .map_err(|e| {
                        AgentLoopError::store(format!(
                            "Failed to revoke invalid identity OAuth grant: {e}"
                        ))
                    })?;
                return Ok(None);
            }
            Err(OAuthRefreshError::Failed(_)) => return Ok(None),
        };
        let rotated_refresh = token.refresh_token.as_deref().unwrap_or(&refresh_token);
        let fresh_access_token = token.access_token.clone();
        let updated = self
            .db
            .update_agent_identity_connection_oauth_tokens(UpdateOAuthConnectionTokens {
                connection_id: row.id,
                access_token_encrypted: self
                    .encryption
                    .encrypt_string(&fresh_access_token)
                    .map_err(|e| {
                        AgentLoopError::store(format!("Failed to encrypt OAuth access token: {e}"))
                    })?,
                refresh_token_encrypted: self.encryption.encrypt_string(rotated_refresh).map_err(
                    |e| {
                        AgentLoopError::store(format!("Failed to encrypt OAuth refresh token: {e}"))
                    },
                )?,
                expires_at: Self::expiry_from_response(&token),
                scopes: token.scope,
            })
            .await
            .map_err(|e| {
                AgentLoopError::store(format!("Failed to persist refreshed OAuth grant: {e}"))
            })?;
        if updated.is_none() {
            return Ok(None);
        }
        Ok(Some(fresh_access_token))
    }
}

#[async_trait]
impl UserConnectionResolver for DbConnectionResolver {
    async fn get_connection_token(
        &self,
        session_id: SessionId,
        provider: &str,
    ) -> Result<Option<String>> {
        // GitHub App path: mint a fresh installation token
        if provider == "github"
            && let Some(ref minter) = self.github_app
        {
            let installation_id = self
                .db
                .get_installation_id_for_session(session_id, provider)
                .await
                .map_err(|e| {
                    AgentLoopError::store(format!("Failed to resolve GitHub installation: {e}"))
                })?;

            if let Some(id) = installation_id {
                let token = minter.mint_token(id).await.map_err(AgentLoopError::store)?;
                return Ok(Some(token));
            }
        }

        if let Some(server_id) = Self::parse_mcp_oauth_provider(provider) {
            if let Some(credentials) = self
                .db
                .get_mcp_oauth_session_credentials(session_id, server_id)
                .await
                .map_err(|e| {
                    AgentLoopError::store(format!("Failed to resolve session OAuth grant: {e}"))
                })?
            {
                return self
                    .resolve_session_oauth_token(session_id, server_id, credentials)
                    .await;
            }

            if let Some(user_id) = self
                .db
                .get_connection_user_for_session(session_id, provider)
                .await
                .map_err(|e| {
                    AgentLoopError::store(format!("Failed to resolve connection owner: {e}"))
                })?
                && let Some(row) = self
                    .db
                    .get_user_connection(user_id, provider)
                    .await
                    .map_err(|e| {
                        AgentLoopError::store(format!("Failed to resolve connection: {e}"))
                    })?
            {
                return self
                    .resolve_user_oauth_token(session_id, server_id, user_id, row)
                    .await;
            }
        }

        // Legacy path: decrypt stored OAuth token
        let encrypted = self
            .db
            .get_connection_token_for_session(session_id, provider)
            .await
            .map_err(|e| AgentLoopError::store(format!("Failed to resolve connection: {e}")))?;

        match encrypted {
            Some(blob) => {
                let token = self.encryption.decrypt_to_string(&blob).map_err(|e| {
                    AgentLoopError::store(format!("Failed to decrypt connection token: {e}"))
                })?;
                Ok(Some(token))
            }
            None => Ok(None),
        }
    }

    /// Resolve an MCP credential as a pure function of `acts_as`.
    ///
    /// Each arm reads exactly one store. There is deliberately no path from one
    /// arm to another and no fallback to [`Self::get_connection_token`], whose
    /// identity-preferring lookup is the substitution EVE-1029 removes. The
    /// fallback stays for the non-MCP providers that depend on it today; MCP no
    /// longer routes through it.
    ///
    /// THREAT[TM-TOOL-041]: resolver-side invariant, not validation. It must
    /// hold for configs written before validation existed or written straight
    /// to the database, so it is enforced here rather than at the write path.
    async fn get_mcp_connection_token(
        &self,
        session_id: SessionId,
        provider: &str,
        acts_as: everruns_core::McpServerActsAs,
    ) -> Result<Option<String>> {
        let Some(server_id) = Self::parse_mcp_oauth_provider(provider) else {
            return Ok(None);
        };

        match acts_as {
            // Reads no connection store at all. Literal headers only, and those
            // are applied by the caller, not here.
            everruns_core::McpServerActsAs::None => Ok(None),

            // Only the agent identity's own grant. Never a user connection, and
            // never a session-scoped grant — those are authorized by a human in
            // the session, which is user auth wearing a service label.
            everruns_core::McpServerActsAs::Service => {
                let row = self
                    .db
                    .get_agent_identity_connection_row_for_session(session_id, provider)
                    .await
                    .map_err(|e| {
                        AgentLoopError::store(format!("Failed to resolve identity grant: {e}"))
                    })?;
                match row {
                    Some(row) => {
                        self.resolve_identity_oauth_token(session_id, server_id, row)
                            .await
                    }
                    None => Ok(None),
                }
            }

            // Only the invoking user's grant, and only when a human actually
            // initiated the session. An unattended run has no invoking user to
            // act as, so it fails closed instead of borrowing the owner's.
            everruns_core::McpServerActsAs::User => {
                if !self
                    .db
                    .session_has_human_initiator(session_id)
                    .await
                    .map_err(|e| {
                        AgentLoopError::store(format!("Failed to resolve session initiator: {e}"))
                    })?
                {
                    return Ok(None);
                }

                // A session-scoped grant is authorized in-session by the
                // invoking human, so it is that user's credential and is
                // preferred while it lasts.
                if let Some(credentials) = self
                    .db
                    .get_mcp_oauth_session_credentials(session_id, server_id)
                    .await
                    .map_err(|e| {
                        AgentLoopError::store(format!("Failed to resolve session OAuth grant: {e}"))
                    })?
                {
                    return self
                        .resolve_session_oauth_token(session_id, server_id, credentials)
                        .await;
                }

                let Some(row) = self
                    .db
                    .get_owner_user_connection_for_session(session_id, provider)
                    .await
                    .map_err(|e| {
                        AgentLoopError::store(format!("Failed to resolve user connection: {e}"))
                    })?
                else {
                    return Ok(None);
                };

                let user_id = row.user_id;
                self.resolve_user_oauth_token(session_id, server_id, user_id, row)
                    .await
            }
        }
    }

    async fn get_connection_user(
        &self,
        session_id: SessionId,
        provider: &str,
    ) -> Result<Option<Uuid>> {
        self.db
            .get_connection_user_for_session(session_id, provider)
            .await
            .map_err(|e| AgentLoopError::store(format!("Failed to resolve connection owner: {e}")))
    }

    async fn get_connection_metadata(
        &self,
        session_id: SessionId,
        provider: &str,
    ) -> Result<Option<serde_json::Value>> {
        self.db
            .get_connection_metadata_for_session(session_id, provider)
            .await
            .map_err(|e| {
                AgentLoopError::store(format!("Failed to resolve connection metadata: {e}"))
            })
    }

    async fn get_connection_token_for_user(
        &self,
        user_id: Uuid,
        provider: &str,
    ) -> Result<Option<String>> {
        // GitHub App path: mint a fresh installation token for the specific user connection.
        if provider == "github"
            && let Some(ref minter) = self.github_app
        {
            let installation_id = self
                .db
                .get_installation_id_for_user(user_id, provider)
                .await
                .map_err(|e| {
                    AgentLoopError::store(format!(
                        "Failed to resolve GitHub installation for cleanup: {e}"
                    ))
                })?;

            if let Some(id) = installation_id {
                let token = minter.mint_token(id).await.map_err(AgentLoopError::store)?;
                return Ok(Some(token));
            }
        }

        let encrypted = self
            .db
            .get_connection_token_for_user(user_id, provider)
            .await
            .map_err(|e| AgentLoopError::store(format!("Failed to resolve connection: {e}")))?;

        match encrypted {
            Some(blob) => {
                let token = self.encryption.decrypt_to_string(&blob).map_err(|e| {
                    AgentLoopError::store(format!("Failed to decrypt connection token: {e}"))
                })?;
                Ok(Some(token))
            }
            None => Ok(None),
        }
    }
}

/// Resolver used when encryption is not configured (notably dev mode).
///
/// Returns `None` for every lookup. Without encryption we cannot decrypt
/// stored connection tokens, so behaving as if no connections exist is the
/// only safe option. Tools that need a connection should surface their own
/// "not connected" guidance rather than panicking at the trait boundary.
#[derive(Clone, Default)]
pub struct NoopConnectionResolver;

#[async_trait]
impl UserConnectionResolver for NoopConnectionResolver {
    async fn get_connection_token(
        &self,
        _session_id: SessionId,
        _provider: &str,
    ) -> Result<Option<String>> {
        Ok(None)
    }
}

#[cfg(test)]
#[path = "connection_resolver_tests.rs"]
mod tests;

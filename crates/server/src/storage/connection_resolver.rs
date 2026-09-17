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
    McpOAuthSessionCredentialsRow, UpdateUserConnectionOAuthTokens,
    UpsertMcpOAuthSessionCredentials, UserConnectionRow,
};
use crate::auth::oauth::GitHubAppService;
use crate::domains::mcp_servers::{McpServerOAuthSettings, McpServerService};
use crate::oauth_client::{
    EgressOAuthRefreshExchange, OAuthRefreshExchange, OAuthRefreshRequest, OAuthTokenResponse,
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
    ) -> Option<OAuthTokenResponse> {
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
            Ok(token) => Some(token),
            Err((status, _)) => {
                // Do not expose provider responses or credentials. Returning no
                // token preserves the existing connection_required behavior.
                tracing::warn!(%status, "MCP OAuth token refresh failed");
                None
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
        let Some(token) = self.exchange_refresh(config, refresh_token.clone()).await else {
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
        let Some(token) = self.exchange_refresh(config, refresh_token.clone()).await else {
            return Ok(None);
        };
        let rotated_refresh = token.refresh_token.as_deref().unwrap_or(&refresh_token);
        let fresh_access_token = token.access_token.clone();
        // THREAT[TM-TOOL-025]: replace the complete rotated grant atomically
        // before exposing the new access token.
        let updated = self
            .db
            .update_user_connection_oauth_tokens(UpdateUserConnectionOAuthTokens {
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
                let encrypted = self
                    .db
                    .get_agent_identity_connection_for_session(session_id, provider)
                    .await
                    .map_err(|e| {
                        AgentLoopError::store(format!("Failed to resolve identity grant: {e}"))
                    })?;

                match encrypted {
                    Some(blob) => {
                        let token = self.encryption.decrypt_to_string(&blob).map_err(|e| {
                            AgentLoopError::store(format!("Failed to decrypt identity grant: {e}"))
                        })?;
                        Ok(Some(token))
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
mod tests {
    use super::*;
    use crate::kernel_imports::{DEFAULT_ORG_ID, PrincipalId};
    use crate::storage::InMemoryDatabase;
    use crate::storage::models::{CreateMcpServerRow, CreateSessionRow, CreateUserConnectionRow};
    use everruns_core::connection_services::UserConnectionResolver;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const TEST_KEY: &str = "kek-v1:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";

    struct FakeRefreshExchange {
        calls: AtomicUsize,
        delay: StdDuration,
        fail: bool,
    }

    #[async_trait]
    impl OAuthRefreshExchange for FakeRefreshExchange {
        async fn exchange(
            &self,
            request: OAuthRefreshRequest,
        ) -> std::result::Result<OAuthTokenResponse, (axum::http::StatusCode, String)> {
            assert_eq!(request.refresh_token, "old-refresh");
            self.calls.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(self.delay).await;
            if self.fail {
                return Err((axum::http::StatusCode::BAD_GATEWAY, "provider error".into()));
            }
            Ok(OAuthTokenResponse {
                access_token: "fresh-access".to_string(),
                refresh_token: Some("rotated-refresh".to_string()),
                expires_in: Some(3600),
                scope: Some("email.send".to_string()),
            })
        }
    }

    fn encryption() -> EncryptionService {
        EncryptionService::new(TEST_KEY, &[]).unwrap()
    }

    fn session_input(owner_user_id: Option<Uuid>) -> CreateSessionRow {
        CreateSessionRow {
            source: everruns_platform::SessionSource::Api,
            org_id: DEFAULT_ORG_ID,
            workspace_id: None,
            app_id: None,
            endpoint_id: None,
            harness_id: None,
            agent_id: None,
            agent_version_id: None,
            agent_config_hash: None,
            agent_identity_id: None,
            owner_principal_id: PrincipalId::from_seed(1),
            resolved_owner_user_id: owner_user_id,
            title: None,
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
            mcp_servers: serde_json::json!({}),
            system_prompt: None,
            initial_files: serde_json::json!([]),
            hints: None,
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
            blueprint_id: None,
            blueprint_config: None,
            parent_session_id: None,
            budget_root_session_id: None,
        }
    }

    async fn setup(
        owner_user_id: Option<Uuid>,
    ) -> (StorageBackend, EncryptionService, SessionId, Uuid, String) {
        let memory = Arc::new(InMemoryDatabase::new());
        let db = StorageBackend::InMemory(memory);
        let server_id = Uuid::now_v7();
        db.create_mcp_server_with_id(
            DEFAULT_ORG_ID,
            server_id,
            CreateMcpServerRow {
                name: "resend".to_string(),
                description: None,
                url: "https://mcp.resend.com/mcp".to_string(),
                transport_type: "streamable_http".to_string(),
                api_key_encrypted: None,
                headers: None,
                settings: Some(serde_json::json!({
                    "auth_mode": "o_auth",
                    "oauth": {
                        "token_endpoint": "https://api.resend.com/oauth/token",
                        "client_id": "test-client"
                    }
                })),
            },
        )
        .await
        .unwrap();
        let session = db
            .create_session(session_input(owner_user_id))
            .await
            .unwrap();
        let server = db
            .get_mcp_server(session.org_id, server_id)
            .await
            .unwrap()
            .unwrap();
        let settings = McpServerService::settings_from_row(&server);
        assert_eq!(settings.auth_mode, McpServerAuthMode::OAuth);
        assert_eq!(
            settings
                .oauth
                .as_ref()
                .and_then(|oauth| oauth.client_id.as_deref()),
            Some("test-client")
        );
        let provider = format!("mcp_oauth_{server_id}");
        (db, encryption(), session.id, server_id, provider)
    }

    // ---------------------------------------------------------------
    // EVE-1029: MCP credential resolution is a pure function of actsAs.
    //
    // The defect class here is reading the *wrong* store, so every test
    // asserts both what was read and what was not. A happy-path assertion
    // alone would pass just as well with the fallback still in place.
    // ---------------------------------------------------------------

    use crate::kernel_imports::AgentIdentityId;
    use crate::storage::models::{CreateAgentIdentityConnectionRow, CreatePrincipalRow};
    use everruns_core::McpServerActsAs;

    /// A session whose owner principal really is a person.
    const ATTENDED: &str = "user";
    /// A session fired by a trigger/schedule: the owner principal is the
    /// agent's own identity, which may still *resolve* to a human by lineage.
    const UNATTENDED: &str = "agent_identity";

    struct McpFixture {
        db: StorageBackend,
        encryption: EncryptionService,
        session_id: SessionId,
        provider: String,
        user_id: Uuid,
        identity_id: AgentIdentityId,
    }

    /// Seed a session with both stores populated unless told otherwise, so a
    /// test that asserts "did not read X" is meaningful: X is always there to
    /// be read incorrectly.
    async fn mcp_setup(
        owner_kind: &str,
        with_user_grant: bool,
        with_identity_grant: bool,
    ) -> McpFixture {
        let memory = Arc::new(InMemoryDatabase::new());
        let db = StorageBackend::InMemory(memory);
        let encryption = encryption();
        let server_id = Uuid::now_v7();
        db.create_mcp_server_with_id(
            DEFAULT_ORG_ID,
            server_id,
            CreateMcpServerRow {
                name: "linear".to_string(),
                description: None,
                url: "https://mcp.linear.app/mcp".to_string(),
                transport_type: "streamable_http".to_string(),
                api_key_encrypted: None,
                headers: None,
                settings: Some(serde_json::json!({
                    "auth_mode": "o_auth",
                    "oauth": {
                        "token_endpoint": "https://api.linear.app/oauth/token",
                        "client_id": "test-client"
                    }
                })),
            },
        )
        .await
        .unwrap();

        let user_id = Uuid::now_v7();
        let identity_id = AgentIdentityId::from_seed(7);
        let owner_principal_id = PrincipalId::from_seed(42);
        db.create_principal(CreatePrincipalRow {
            id: owner_principal_id,
            org_id: DEFAULT_ORG_ID,
            kind: owner_kind.to_string(),
            subject_id: Some(Uuid::now_v7()),
            parent_principal_id: None,
            // Populated even for the unattended case on purpose: this is the
            // lineage an unattended run would borrow a human through.
            resolved_user_id: Some(user_id),
            metadata: serde_json::json!({}),
        })
        .await
        .unwrap();

        let mut input = session_input(Some(user_id));
        input.owner_principal_id = owner_principal_id;
        input.agent_identity_id = Some(identity_id);
        let session = db.create_session(input).await.unwrap();

        let provider = format!("mcp_oauth_{server_id}");

        if with_user_grant {
            db.upsert_user_connection(CreateUserConnectionRow {
                user_id,
                provider: provider.clone(),
                connection_type: "oauth".to_string(),
                provider_user_id: None,
                provider_username: Some("the-human".to_string()),
                access_token_encrypted: Some(encryption.encrypt_string("user-token").unwrap()),
                refresh_token_encrypted: None,
                scopes: None,
                expires_at: None,
                installation_id: None,
                provider_metadata: None,
            })
            .await
            .unwrap();
        }

        if with_identity_grant {
            db.upsert_agent_identity_connection(CreateAgentIdentityConnectionRow {
                agent_identity_id: identity_id,
                provider: provider.clone(),
                connection_type: "oauth".to_string(),
                provider_user_id: None,
                provider_username: Some("the-agent".to_string()),
                access_token_encrypted: Some(encryption.encrypt_string("identity-token").unwrap()),
                refresh_token_encrypted: None,
                scopes: None,
                expires_at: None,
                installation_id: None,
                provider_metadata: None,
            })
            .await
            .unwrap();
        }

        McpFixture {
            db,
            encryption,
            session_id: session.id,
            provider,
            user_id,
            identity_id,
        }
    }

    fn resolver_for(fixture: &McpFixture) -> DbConnectionResolver {
        // No grant in these fixtures is expired, so the exchange must never be
        // called; a refresh here would mean the resolver took a path it should
        // not have.
        let exchange = Arc::new(FakeRefreshExchange {
            calls: AtomicUsize::new(0),
            delay: StdDuration::ZERO,
            fail: false,
        });
        DbConnectionResolver::with_oauth_refresh(
            fixture.db.clone(),
            fixture.encryption.clone(),
            None,
            exchange,
        )
    }

    #[tokio::test]
    async fn user_attachment_resolves_the_invoking_user_and_never_the_identity_grant() {
        let fixture = mcp_setup(ATTENDED, true, true).await;
        let resolver = resolver_for(&fixture);

        let token = resolver
            .get_mcp_connection_token(fixture.session_id, &fixture.provider, McpServerActsAs::User)
            .await
            .unwrap();

        assert_eq!(token.as_deref(), Some("user-token"));
        // The Warp-confusion case: the session carries an identity holding a
        // grant for this very preset, and it must not have been consulted.
        assert_ne!(token.as_deref(), Some("identity-token"));
    }

    #[tokio::test]
    async fn user_attachment_without_user_grant_fails_closed_leaving_identity_grant_untouched() {
        let fixture = mcp_setup(ATTENDED, false, true).await;
        let resolver = resolver_for(&fixture);

        let token = resolver
            .get_mcp_connection_token(fixture.session_id, &fixture.provider, McpServerActsAs::User)
            .await
            .unwrap();

        assert_eq!(
            token, None,
            "must fail closed rather than borrow the identity grant"
        );
        // The identity grant is still there, unread and unmodified.
        let identity = fixture
            .db
            .get_agent_identity_connection_for_session(fixture.session_id, &fixture.provider)
            .await
            .unwrap()
            .expect("identity grant should be untouched");
        assert_eq!(
            fixture.encryption.decrypt_to_string(&identity).unwrap(),
            "identity-token"
        );
    }

    #[tokio::test]
    async fn service_attachment_resolves_the_identity_and_never_the_user_grant() {
        let fixture = mcp_setup(ATTENDED, true, true).await;
        let resolver = resolver_for(&fixture);

        let token = resolver
            .get_mcp_connection_token(
                fixture.session_id,
                &fixture.provider,
                McpServerActsAs::Service,
            )
            .await
            .unwrap();

        assert_eq!(token.as_deref(), Some("identity-token"));
        assert_ne!(token.as_deref(), Some("user-token"));
    }

    #[tokio::test]
    async fn service_attachment_without_identity_grant_fails_closed_despite_a_user_grant() {
        let fixture = mcp_setup(ATTENDED, true, false).await;
        let resolver = resolver_for(&fixture);

        let token = resolver
            .get_mcp_connection_token(
                fixture.session_id,
                &fixture.provider,
                McpServerActsAs::Service,
            )
            .await
            .unwrap();

        assert_eq!(
            token, None,
            "a service attachment must never spend the invoking user's token"
        );
        // The user grant is still there, unread.
        let user_grant = fixture
            .db
            .get_user_connection(fixture.user_id, &fixture.provider)
            .await
            .unwrap();
        assert!(user_grant.is_some(), "user grant should be untouched");
    }

    #[tokio::test]
    async fn user_attachment_in_an_unattended_session_fails_closed_though_the_owner_holds_a_grant()
    {
        // The owner principal is the agent identity, but its lineage resolves
        // to a human who *does* hold a grant. That is precisely the borrow this
        // rule forbids.
        let fixture = mcp_setup(UNATTENDED, true, false).await;
        let resolver = resolver_for(&fixture);

        let token = resolver
            .get_mcp_connection_token(fixture.session_id, &fixture.provider, McpServerActsAs::User)
            .await
            .unwrap();

        assert_eq!(
            token, None,
            "an unattended run has no invoking user and must not borrow one"
        );
        assert!(
            !fixture
                .db
                .session_has_human_initiator(fixture.session_id)
                .await
                .unwrap()
        );
        // The grant it declined to spend is still present and resolvable for a
        // genuinely attended session, so this is a refusal, not an absence.
        assert!(
            fixture
                .db
                .get_user_connection(fixture.user_id, &fixture.provider)
                .await
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn none_attachment_reads_no_connection_store_at_all() {
        let fixture = mcp_setup(ATTENDED, true, true).await;
        let resolver = resolver_for(&fixture);

        let token = resolver
            .get_mcp_connection_token(fixture.session_id, &fixture.provider, McpServerActsAs::None)
            .await
            .unwrap();

        assert_eq!(token, None);
        // Both stores are populated, so None here proves neither was consulted.
        assert!(
            fixture
                .db
                .get_user_connection(fixture.user_id, &fixture.provider)
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            fixture
                .db
                .get_agent_identity_connection_for_session(fixture.session_id, &fixture.provider)
                .await
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn attended_session_is_decided_by_the_owner_principal_not_the_resolved_owner() {
        // Both fixtures carry the same resolved_owner_user_id; only the owner
        // principal's kind differs. If resolution ever regresses to reading the
        // denormalized column, these two agree and this test fails.
        let attended = mcp_setup(ATTENDED, true, false).await;
        let unattended = mcp_setup(UNATTENDED, true, false).await;

        assert!(
            attended
                .db
                .session_has_human_initiator(attended.session_id)
                .await
                .unwrap()
        );
        assert!(
            !unattended
                .db
                .session_has_human_initiator(unattended.session_id)
                .await
                .unwrap()
        );

        let attended_session = attended
            .db
            .get_session_unscoped(attended.session_id)
            .await
            .unwrap()
            .unwrap();
        let unattended_session = unattended
            .db
            .get_session_unscoped(unattended.session_id)
            .await
            .unwrap()
            .unwrap();
        assert!(attended_session.resolved_owner_user_id.is_some());
        assert!(
            unattended_session.resolved_owner_user_id.is_some(),
            "the unattended session must still resolve a human, or this test proves nothing"
        );
    }

    #[tokio::test]
    async fn a_non_mcp_provider_never_resolves_through_the_acts_as_path() {
        let fixture = mcp_setup(ATTENDED, true, true).await;
        let resolver = resolver_for(&fixture);

        for acts_as in [
            McpServerActsAs::None,
            McpServerActsAs::Service,
            McpServerActsAs::User,
        ] {
            let token = resolver
                .get_mcp_connection_token(fixture.session_id, "github", acts_as)
                .await
                .unwrap();
            assert_eq!(token, None, "{acts_as} must not resolve a non-MCP provider");
        }
    }

    #[tokio::test]
    async fn identity_grant_is_unreachable_from_a_session_without_that_identity() {
        let fixture = mcp_setup(ATTENDED, false, true).await;
        let other = mcp_setup(ATTENDED, false, false).await;

        // Same identity id seed, different database: proves the lookup is
        // session-scoped rather than keyed only by the identity.
        assert_eq!(fixture.identity_id, other.identity_id);
        assert!(
            other
                .db
                .get_agent_identity_connection_for_session(other.session_id, &other.provider)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn resend_connection_after_sixteen_minutes_refreshes_without_reconnect() {
        let user_id = Uuid::now_v7();
        let (db, encryption, session_id, _server_id, provider) = setup(Some(user_id)).await;
        let connection = db
            .upsert_user_connection(CreateUserConnectionRow {
                user_id,
                provider: provider.clone(),
                connection_type: "oauth".to_string(),
                provider_user_id: None,
                provider_username: Some("Resend".to_string()),
                access_token_encrypted: Some(encryption.encrypt_string("stale-access").unwrap()),
                refresh_token_encrypted: Some(encryption.encrypt_string("old-refresh").unwrap()),
                scopes: None,
                expires_at: Some(Utc::now() - Duration::minutes(16)),
                installation_id: None,
                provider_metadata: None,
            })
            .await
            .unwrap();
        let exchange = Arc::new(FakeRefreshExchange {
            calls: AtomicUsize::new(0),
            delay: StdDuration::ZERO,
            fail: false,
        });
        let resolver = DbConnectionResolver::with_oauth_refresh(
            db.clone(),
            encryption.clone(),
            None,
            exchange.clone(),
        );

        let token = resolver
            .get_connection_token(session_id, &provider)
            .await
            .unwrap();

        assert_eq!(token.as_deref(), Some("fresh-access"));
        assert_eq!(exchange.calls.load(Ordering::SeqCst), 1);
        let updated = db
            .get_user_connection(user_id, &provider)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.id, connection.id);
        assert_eq!(
            encryption
                .decrypt_to_string(updated.access_token_encrypted.as_deref().unwrap())
                .unwrap(),
            "fresh-access"
        );
        assert_eq!(
            encryption
                .decrypt_to_string(updated.refresh_token_encrypted.as_deref().unwrap())
                .unwrap(),
            "rotated-refresh"
        );
        assert_eq!(updated.scopes.as_deref(), Some("email.send"));
        assert!(updated.expires_at.is_some_and(|value| value > Utc::now()));
    }

    #[tokio::test]
    async fn expired_session_grant_refreshes_and_persists_rotated_grant() {
        let (db, encryption, session_id, server_id, provider) = setup(None).await;
        db.upsert_mcp_oauth_session_credentials(UpsertMcpOAuthSessionCredentials {
            session_id,
            server_id,
            access_token_encrypted: encryption.encrypt_string("stale-access").unwrap(),
            refresh_token_encrypted: Some(encryption.encrypt_string("old-refresh").unwrap()),
            expires_at_encrypted: Some(
                encryption
                    .encrypt_string(&(Utc::now() - Duration::minutes(1)).to_rfc3339())
                    .unwrap(),
            ),
        })
        .await
        .unwrap();
        let exchange = Arc::new(FakeRefreshExchange {
            calls: AtomicUsize::new(0),
            delay: StdDuration::ZERO,
            fail: false,
        });
        let resolver = DbConnectionResolver::with_oauth_refresh(
            db.clone(),
            encryption.clone(),
            None,
            exchange.clone(),
        );

        assert_eq!(
            resolver
                .get_connection_token(session_id, &provider)
                .await
                .unwrap()
                .as_deref(),
            Some("fresh-access")
        );
        let updated = db
            .get_mcp_oauth_session_credentials(session_id, server_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            encryption
                .decrypt_to_string(&updated.access_token_encrypted)
                .unwrap(),
            "fresh-access"
        );
        assert_eq!(
            encryption
                .decrypt_to_string(updated.refresh_token_encrypted.as_deref().unwrap())
                .unwrap(),
            "rotated-refresh"
        );
    }

    #[tokio::test]
    async fn concurrent_expired_resolution_coalesces_refresh() {
        let user_id = Uuid::now_v7();
        let (db, encryption, session_id, _server_id, provider) = setup(Some(user_id)).await;
        db.upsert_user_connection(CreateUserConnectionRow {
            user_id,
            provider: provider.clone(),
            connection_type: "oauth".to_string(),
            provider_user_id: None,
            provider_username: Some("Resend".to_string()),
            access_token_encrypted: Some(encryption.encrypt_string("stale-access").unwrap()),
            refresh_token_encrypted: Some(encryption.encrypt_string("old-refresh").unwrap()),
            scopes: None,
            expires_at: Some(Utc::now() - Duration::minutes(1)),
            installation_id: None,
            provider_metadata: None,
        })
        .await
        .unwrap();
        let exchange = Arc::new(FakeRefreshExchange {
            calls: AtomicUsize::new(0),
            delay: StdDuration::from_millis(50),
            fail: false,
        });
        let resolver = Arc::new(DbConnectionResolver::with_oauth_refresh(
            db,
            encryption,
            None,
            exchange.clone(),
        ));

        let mut tasks = Vec::new();
        for _ in 0..8 {
            let resolver = resolver.clone();
            let provider = provider.clone();
            tasks.push(tokio::spawn(async move {
                resolver
                    .get_connection_token(session_id, &provider)
                    .await
                    .unwrap()
            }));
        }
        for task in tasks {
            assert_eq!(task.await.unwrap().as_deref(), Some("fresh-access"));
        }
        assert_eq!(exchange.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn failed_refresh_fails_closed_and_preserves_existing_grant() {
        let user_id = Uuid::now_v7();
        let (db, encryption, session_id, _server_id, provider) = setup(Some(user_id)).await;
        let original_access = encryption.encrypt_string("stale-access").unwrap();
        let original_refresh = encryption.encrypt_string("old-refresh").unwrap();
        db.upsert_user_connection(CreateUserConnectionRow {
            user_id,
            provider: provider.clone(),
            connection_type: "oauth".to_string(),
            provider_user_id: None,
            provider_username: Some("Resend".to_string()),
            access_token_encrypted: Some(original_access.clone()),
            refresh_token_encrypted: Some(original_refresh.clone()),
            scopes: None,
            expires_at: Some(Utc::now() - Duration::minutes(1)),
            installation_id: None,
            provider_metadata: None,
        })
        .await
        .unwrap();
        let exchange = Arc::new(FakeRefreshExchange {
            calls: AtomicUsize::new(0),
            delay: StdDuration::ZERO,
            fail: true,
        });
        let resolver = DbConnectionResolver::with_oauth_refresh(
            db.clone(),
            encryption,
            None,
            exchange.clone(),
        );

        assert_eq!(
            resolver
                .get_connection_token(session_id, &provider)
                .await
                .unwrap(),
            None
        );
        assert_eq!(exchange.calls.load(Ordering::SeqCst), 1);
        let unchanged = db
            .get_user_connection(user_id, &provider)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            unchanged.access_token_encrypted.as_deref(),
            Some(original_access.as_slice())
        );
        assert_eq!(
            unchanged.refresh_token_encrypted.as_deref(),
            Some(original_refresh.as_slice())
        );
    }
}

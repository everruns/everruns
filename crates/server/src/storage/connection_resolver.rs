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
    EgressService, McpServerAuthMode, contracts::error::AgentLoopError, contracts::error::Result,
};
use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use chrono::{DateTime, Duration, Utc};
use everruns_contracts::typed_id::SessionId;
use everruns_core::connection_services::UserConnectionResolver;
use moka::sync::Cache;
use std::sync::Arc;
use std::time::Duration as StdDuration;
use tokio::sync::Mutex;
use uuid::Uuid;

use super::backend::StorageBackend;
use super::encryption::EncryptionService;
use super::models::{
    McpOAuthSessionCredentialsRow, UpdateOAuthConnectionTokens, UpsertMcpOAuthSessionCredentials,
    VirtualUserConnectionRow,
};
use crate::auth::oauth::GitHubAppService;
use crate::domains::mcp_servers::{McpServerOAuthSettings, McpServerService};
use crate::oauth_client::{
    EgressOAuthRefreshExchange, OAuthRefreshError, OAuthRefreshExchange, OAuthRefreshRequest,
    OAuthTokenResponse,
};

const OAUTH_REFRESH_SKEW: Duration = Duration::seconds(60);

/// Which org an identity grant's OAuth client is looked up in: the session's,
/// or an explicit org for session-less callers.
#[derive(Clone, Copy)]
enum OAuthScope {
    Session(SessionId),
    Org(i64),
}
const REFRESH_LOCK_MAX_CAPACITY: u64 = 10_000;
const REFRESH_LOCK_IDLE_TTL: StdDuration = StdDuration::from_secs(10 * 60);

/// Resolves connection tokens for tool execution.
///
/// The verified speaker of the current input owns user grants. Autonomous inputs
/// use the actual responding agent's active service account. Session owners and
/// earlier speakers never provide authority. Resource cleanup uses its captured owner.
#[derive(Clone)]
pub struct DbConnectionResolver {
    db: StorageBackend,
    input_message_id: Option<Uuid>,
    encryption: EncryptionService,
    /// GitHub App service for minting installation tokens (None = legacy OAuth only)
    github_app: Option<GitHubAppTokenMinter>,
    /// GitHub client for per-agent Apps (see `crate::github_apps`).
    github_apps: crate::github_apps::GitHubAppApi,
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
    resource: Option<String>,
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
            client_id: None,
            client_secret: None,
            endpoints: Default::default(),
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
            input_message_id: None,
            encryption,
            github_app,
            github_apps: crate::github_apps::GitHubAppApi::new(
                crate::github_apps::GitHubEndpoints::from_env(),
            ),
            oauth_refresh,
            refresh_locks: Cache::builder()
                .max_capacity(REFRESH_LOCK_MAX_CAPACITY)
                .time_to_idle(REFRESH_LOCK_IDLE_TTL)
                .build(),
        }
    }

    /// Point per-agent GitHub App calls at another API (tests).
    #[cfg(test)]
    pub(crate) fn with_github_apps_api(mut self, api: crate::github_apps::GitHubAppApi) -> Self {
        self.github_apps = api;
        self
    }

    pub fn bound_to_input_message(&self, id: Uuid) -> Self {
        let mut bound = self.clone();
        bound.input_message_id = Some(id);
        bound
    }
    async fn runtime_subject(
        &self,
        session: SessionId,
    ) -> Result<Option<everruns_contracts::typed_id::VirtualUserId>> {
        // THREAT[TM-AUTHZ-021]: Shared transcripts and workspaces cannot receive private grants,
        // even when the operator selected their own identity. Service grants remain explicit.
        if self
            .db
            .is_playground_session(session)
            .await
            .map_err(|e| AgentLoopError::store(e.to_string()))?
        {
            return Ok(None);
        }
        match self.input_message_id {
            Some(id) => self
                .db
                .runtime_invocation_subject(session, id)
                .await
                .map_err(|e| AgentLoopError::store(e.to_string())),
            None => Ok(None),
        }
    }
    async fn service_connection(
        &self,
        session: SessionId,
        provider: &str,
    ) -> Result<Option<VirtualUserConnectionRow>> {
        if let Some(message) = self.input_message_id
            && !self
                .db
                .runtime_invocation_exists(session, message)
                .await
                .map_err(|e| AgentLoopError::store(e.to_string()))?
        {
            return Ok(None);
        }
        let Some(s) = self
            .db
            .get_session_unscoped(session)
            .await
            .map_err(|e| AgentLoopError::store(e.to_string()))?
        else {
            return Ok(None);
        };
        let responder = match self.input_message_id {
            Some(id) => self
                .db
                .runtime_invocation_responder(session, id)
                .await
                .map_err(|e| AgentLoopError::store(e.to_string()))?
                .map(everruns_contracts::typed_id::AgentId::from_uuid),
            None => s.agent_id,
        };
        let Some(agent_id) = responder else {
            return Ok(None);
        };
        let Some(agent) = self
            .db
            .get_agent(s.org_id, agent_id)
            .await
            .map_err(|e| AgentLoopError::store(e.to_string()))?
        else {
            return Ok(None);
        };
        if agent.status != "active" {
            return Ok(None);
        };
        let Some(id) = agent.virtual_user_id else {
            return Ok(None);
        };
        let Some(v) = self
            .db
            .get_virtual_user(s.org_id, id)
            .await
            .map_err(|e| AgentLoopError::store(e.to_string()))?
        else {
            return Ok(None);
        };
        if v.status != "active" || v.usage != "service" {
            return Ok(None);
        };
        self.db
            .get_virtual_user_connection(id, provider)
            .await
            .map_err(|e| AgentLoopError::store(e.to_string()))
    }
    async fn selected_connection(
        &self,
        session: SessionId,
        provider: &str,
    ) -> Result<Option<VirtualUserConnectionRow>> {
        if let Some(id) = self.runtime_subject(session).await? {
            return self
                .db
                .get_virtual_user_connection(id, provider)
                .await
                .map_err(|e| AgentLoopError::store(e.to_string()));
        }
        if let Some(message) = self.input_message_id
            && self
                .db
                .runtime_invocation_has_subject(session, message)
                .await
                .map_err(|e| AgentLoopError::store(e.to_string()))?
        {
            return Ok(None);
        }
        self.service_connection(session, provider).await
    }
    /// The token an agent's own service identity holds for an MCP catalog
    /// server (`provider` = `mcp_oauth_{server_id}`), resolved without a
    /// session: an `mcp_event` trigger subscribes as the agent before any
    /// session exists. Applies `service_connection`'s checks (active agent,
    /// active service identity) and refreshes a grant near expiry.
    pub async fn agent_service_mcp_token(
        &self,
        org_id: i64,
        agent_id: everruns_contracts::typed_id::AgentId,
        provider: &str,
    ) -> Result<Option<String>> {
        let Some(server) = Self::parse_mcp_oauth_provider(provider) else {
            return Ok(None);
        };
        let store = |e: anyhow::Error| AgentLoopError::store(e.to_string());
        let Some(agent) = self.db.get_agent(org_id, agent_id).await.map_err(store)? else {
            return Ok(None);
        };
        let Some(identity_id) = agent.virtual_user_id.filter(|_| agent.status == "active") else {
            return Ok(None);
        };
        let identity = self
            .db
            .get_virtual_user(org_id, identity_id)
            .await
            .map_err(store)?
            .filter(|v| v.status == "active" && v.usage == "service");
        if identity.is_none() {
            return Ok(None);
        }
        let Some(row) = self
            .db
            .get_virtual_user_connection(identity_id, provider)
            .await
            .map_err(store)?
        else {
            return Ok(None);
        };
        self.resolve_identity_oauth_token(OAuthScope::Org(org_id), server, row)
            .await
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
        self.oauth_client_config_in_org(session.org_id, server_id)
            .await
    }

    async fn oauth_client_config_in_org(
        &self,
        org_id: i64,
        server_id: Uuid,
    ) -> Result<Option<OAuthClientConfig>> {
        let row = self
            .db
            .get_mcp_server(org_id, server_id)
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
            resource: oauth.resource.clone(),
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
                resource: config.resource,
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
        if self.runtime_subject(session_id).await? != credentials.virtual_user_id {
            return Ok(None);
        }
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
        if self.runtime_subject(session_id).await? != credentials.virtual_user_id {
            return Ok(None);
        }
        if !self.needs_refresh(credentials.expires_at_encrypted.as_deref())? {
            return self
                .decrypt(&credentials.access_token_encrypted, "OAuth access token")
                .map(Some);
        }
        let Some(refresh_token_encrypted) = credentials.refresh_token_encrypted.as_deref() else {
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
        let expires_at = Self::expiry_from_response(&token);
        // THREAT[TM-TOOL-025]: replace the complete rotated grant in one
        // storage transaction before exposing the new access token.
        let persisted = self
            .db
            .rotate_runtime_session_grant(
                &credentials,
                UpsertMcpOAuthSessionCredentials {
                    virtual_user_id: credentials.virtual_user_id,
                    session_id,
                    server_id,
                    access_token_encrypted: self
                        .encryption
                        .encrypt_string(&token.access_token)
                        .map_err(|e| {
                            AgentLoopError::store(format!(
                                "Failed to encrypt OAuth access token: {e}"
                            ))
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
                            AgentLoopError::store(format!(
                                "Failed to encrypt OAuth token expiry: {e}"
                            ))
                        })?,
                },
            )
            .await
            .map_err(|e| {
                AgentLoopError::store(format!("Failed to persist refreshed OAuth grant: {e}"))
            })?;
        if !persisted {
            return Ok(None);
        }
        Ok(Some(token.access_token))
    }

    async fn resolve_identity_oauth_token(
        &self,
        scope: OAuthScope,
        server_id: Uuid,
        row: VirtualUserConnectionRow,
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
            .get_virtual_user_connection(row.virtual_user_id, &row.provider)
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
        let config = match scope {
            OAuthScope::Session(session_id) => {
                self.oauth_client_config(session_id, server_id).await?
            }
            OAuthScope::Org(org_id) => self.oauth_client_config_in_org(org_id, server_id).await?,
        };
        let Some(config) = config else {
            return Ok(None);
        };
        let token = match self.exchange_refresh(config, refresh_token.clone()).await {
            Ok(token) => token,
            Err(OAuthRefreshError::InvalidGrant) => {
                self.db
                    .revoke_runtime_connection_if_unchanged(row.id, access_token_encrypted)
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
            .rotate_runtime_connection(
                access_token_encrypted,
                UpdateOAuthConnectionTokens {
                    connection_id: row.id,
                    access_token_encrypted: self
                        .encryption
                        .encrypt_string(&fresh_access_token)
                        .map_err(|e| {
                            AgentLoopError::store(format!(
                                "Failed to encrypt OAuth access token: {e}"
                            ))
                        })?,
                    refresh_token_encrypted: self
                        .encryption
                        .encrypt_string(rotated_refresh)
                        .map_err(|e| {
                            AgentLoopError::store(format!(
                                "Failed to encrypt OAuth refresh token: {e}"
                            ))
                        })?,
                    expires_at: Self::expiry_from_response(&token),
                    scopes: token.scope,
                },
            )
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
    fn for_execution(&self, id: Uuid) -> Option<Arc<dyn UserConnectionResolver>> {
        Some(Arc::new(self.bound_to_input_message(id)))
    }
    async fn get_connection_token(
        &self,
        session: SessionId,
        provider: &str,
    ) -> Result<Option<String>> {
        let Some(row) = self.selected_connection(session, provider).await? else {
            return Ok(None);
        };
        // Per-agent GitHub App: an agent identity that created its own App
        // (manifest flow) mints with that App's key, so the trigger, the tools
        // and the GitHub MCP all act as the same installation.
        if provider == "github"
            && let Some(installation_id) = row.installation_id
            && let Some(app) = self
                .db
                .get_github_app_for_identity(
                    self.db
                        .get_session_unscoped(session)
                        .await
                        .map_err(|e| AgentLoopError::store(e.to_string()))?
                        .ok_or_else(|| AgentLoopError::session_not_found(session))?
                        .org_id,
                    row.virtual_user_id,
                )
                .await
                .map_err(|e| {
                    AgentLoopError::store(format!("Failed to resolve agent GitHub App: {e}"))
                })?
        {
            let private_key = self
                .encryption
                .decrypt_to_string(&app.private_key_encrypted)
                .map_err(|e| {
                    AgentLoopError::store(format!("Failed to decrypt GitHub App key: {e}"))
                })?;
            let credentials = crate::github_apps::AppCredentials {
                app_id: app.app_id,
                private_key_pem: private_key,
            };
            let token = self
                .github_apps
                .mint_installation_token(&credentials, installation_id)
                .await
                .map_err(|e| {
                    AgentLoopError::store(format!("Failed to mint GitHub installation token: {e}"))
                })?;
            return Ok(Some(token));
        }

        if provider == "github"
            && let Some(minter) = &self.github_app
            && let Some(id) = row.installation_id
        {
            return minter
                .mint_token(id)
                .await
                .map(Some)
                .map_err(AgentLoopError::store);
        }
        if let Some(server) = Self::parse_mcp_oauth_provider(provider) {
            return self
                .resolve_identity_oauth_token(OAuthScope::Session(session), server, row)
                .await;
        }
        row.access_token_encrypted
            .as_deref()
            .map(|v| self.decrypt(v, "connection token"))
            .transpose()
    }
    async fn get_mcp_connection_token(
        &self,
        session: SessionId,
        provider: &str,
        acts_as: everruns_core::McpServerActsAs,
    ) -> Result<Option<String>> {
        let Some(server) = Self::parse_mcp_oauth_provider(provider) else {
            return Ok(None);
        };
        let row = match acts_as {
            everruns_core::McpServerActsAs::None => return Ok(None),
            everruns_core::McpServerActsAs::Service => {
                self.service_connection(session, provider).await?
            }
            everruns_core::McpServerActsAs::User => {
                let Some(id) = self.runtime_subject(session).await? else {
                    return Ok(None);
                };
                if let Some(grant) = self
                    .db
                    .get_mcp_oauth_session_credentials(session, server)
                    .await
                    .map_err(|e| AgentLoopError::store(e.to_string()))?
                    && grant.virtual_user_id == Some(id)
                {
                    return self
                        .resolve_session_oauth_token(session, server, grant)
                        .await;
                }
                self.db
                    .get_virtual_user_connection(id, provider)
                    .await
                    .map_err(|e| AgentLoopError::store(e.to_string()))?
            }
        };
        match row {
            Some(row) => {
                self.resolve_identity_oauth_token(OAuthScope::Session(session), server, row)
                    .await
            }
            None => Ok(None),
        }
    }

    async fn invalidate_mcp_connection(
        &self,
        session_id: SessionId,
        provider: &str,
        acts_as: everruns_core::McpServerActsAs,
        rejected_credential_fingerprint: &str,
    ) -> Result<()> {
        if acts_as != everruns_core::McpServerActsAs::Service {
            return Ok(());
        }
        let Some(server_id) = Self::parse_mcp_oauth_provider(provider) else {
            return Ok(());
        };
        let session = self
            .db
            .get_session_unscoped(session_id)
            .await
            .map_err(|e| AgentLoopError::store(format!("Failed to resolve OAuth session: {e}")))?;
        let Some(session) = session else {
            return Ok(());
        };
        if self
            .db
            .get_mcp_server(session.org_id, server_id)
            .await
            .map_err(|e| AgentLoopError::store(format!("Failed to resolve OAuth server: {e}")))?
            .is_none()
        {
            return Ok(());
        }
        let agent_id = match self.input_message_id {
            Some(message) => self
                .db
                .runtime_invocation_responder(session_id, message)
                .await
                .map_err(|e| AgentLoopError::store(e.to_string()))?
                .map(everruns_contracts::typed_id::AgentId::from_uuid),
            None => session.agent_id,
        };
        let Some(agent_id) = agent_id else {
            return Ok(());
        };
        let row = self.service_connection(session_id, provider).await?;
        let Some(row) = row else {
            return Ok(());
        };
        let Some(access_token_encrypted) = row.access_token_encrypted.as_deref() else {
            return Ok(());
        };
        let access_token = self.decrypt(access_token_encrypted, "OAuth access token")?;
        if everruns_internal_protocol::credential_fingerprint(&access_token)
            != rejected_credential_fingerprint
        {
            return Ok(());
        }

        self.db
            .invalidate_mcp_service_connection_if_access_token_matches(
                row.virtual_user_id,
                provider,
                access_token_encrypted,
                session.org_id,
                server_id,
                agent_id.uuid(),
            )
            .await
            .map_err(|e| {
                AgentLoopError::store(format!("Failed to invalidate MCP service connection: {e}"))
            })?;
        Ok(())
    }

    async fn get_connection_user(&self, s: SessionId, p: &str) -> Result<Option<Uuid>> {
        Ok(self
            .selected_connection(s, p)
            .await?
            .map(|r| r.virtual_user_id.uuid()))
    }
    async fn get_connection_metadata(
        &self,
        s: SessionId,
        p: &str,
    ) -> Result<Option<serde_json::Value>> {
        Ok(self
            .selected_connection(s, p)
            .await?
            .and_then(|r| r.provider_metadata))
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

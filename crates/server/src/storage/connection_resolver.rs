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

use async_trait::async_trait;
use everruns_core::traits::UserConnectionResolver;
use everruns_core::typed_id::SessionId;
use everruns_core::{AgentLoopError, Result};
use uuid::Uuid;

use super::backend::StorageBackend;
use super::encryption::EncryptionService;
use crate::auth::oauth::GitHubAppService;

/// Resolves connection tokens for tool execution.
///
/// Session-based lookup priority:
/// 1. If the session has an `agent_identity_id`, resolves from `agent_identity_connections`.
/// 2. Falls back to `user_connections` via org membership.
///
/// Leased-resource cleanup additionally uses explicit owner-user lookups so
/// the same provider identity that created a resource can delete it later.
#[derive(Clone)]
pub struct DbConnectionResolver {
    db: StorageBackend,
    encryption: EncryptionService,
    /// GitHub App service for minting installation tokens (None = legacy OAuth only)
    github_app: Option<GitHubAppTokenMinter>,
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
    ) -> Self {
        Self {
            db,
            encryption,
            github_app,
        }
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

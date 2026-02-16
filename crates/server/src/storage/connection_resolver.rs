// Lazy user connection token resolver
//
// Resolves connection tokens (e.g. GitHub) at tool execution time instead of
// eagerly injecting at session creation. This means:
// - Tokens are always fresh (reconnect mid-session works)
// - Sessions created before connecting still get tokens
// - Tools can show helpful guidance when not connected

use async_trait::async_trait;
use everruns_core::traits::UserConnectionResolver;
use everruns_core::typed_id::SessionId;
use everruns_core::{AgentLoopError, Result};

use super::backend::StorageBackend;
use super::encryption::EncryptionService;

/// Resolves user connection tokens by looking up the session's org members.
///
/// Query path: session_id → org_id → org_members → user_connections.
/// For single-member orgs (default dev mode), this is unambiguous.
/// For multi-member orgs, returns the first match (future: track session creator).
#[derive(Clone)]
pub struct DbConnectionResolver {
    db: StorageBackend,
    encryption: EncryptionService,
}

impl DbConnectionResolver {
    pub fn new(db: StorageBackend, encryption: EncryptionService) -> Self {
        Self { db, encryption }
    }
}

#[async_trait]
impl UserConnectionResolver for DbConnectionResolver {
    async fn get_connection_token(
        &self,
        session_id: SessionId,
        provider: &str,
    ) -> Result<Option<String>> {
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
}

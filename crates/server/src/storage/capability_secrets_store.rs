// Database-backed CapabilitySecretStore implementation
//
// This module implements the core CapabilitySecretStore trait for persisting
// capability-level secrets (e.g., API keys for web search providers) to the database.

use async_trait::async_trait;
use everruns_core::{AgentId, AgentLoopError, CapabilitySecretStore, Result};

use super::encryption::EncryptionService;
use super::repositories::Database;

// ============================================================================
// DbCapabilitySecretStore - Stores capability secrets in database
// ============================================================================

/// Database-backed capability secret store
///
/// Stores capability-specific secrets (like API keys) encrypted in the database.
/// Secrets are scoped per agent per capability.
#[derive(Clone)]
pub struct DbCapabilitySecretStore {
    db: Database,
    encryption: Option<EncryptionService>,
}

impl DbCapabilitySecretStore {
    /// Create a new secret store with encryption enabled
    pub fn new(db: Database, encryption: EncryptionService) -> Self {
        Self {
            db,
            encryption: Some(encryption),
        }
    }

    /// Create a new secret store without encryption
    /// (all operations will fail)
    pub fn new_without_encryption(db: Database) -> Self {
        Self {
            db,
            encryption: None,
        }
    }

    /// Get the encryption service, returning an error if not configured
    fn encryption(&self) -> Result<&EncryptionService> {
        self.encryption.as_ref().ok_or_else(|| {
            AgentLoopError::store(
                "Encryption not configured. Set SECRETS_ENCRYPTION_KEY environment variable.",
            )
        })
    }
}

#[async_trait]
impl CapabilitySecretStore for DbCapabilitySecretStore {
    async fn get_secret(
        &self,
        agent_id: AgentId,
        capability_id: &str,
        secret_name: &str,
    ) -> Result<Option<String>> {
        let encryption = self.encryption()?;

        let row = self
            .db
            .get_capability_secret(agent_id.uuid(), capability_id, secret_name)
            .await
            .map_err(|e| AgentLoopError::store(e.to_string()))?;

        match row {
            Some(r) => {
                // Decrypt the value
                let decrypted = encryption
                    .decrypt_to_string(&r.value_encrypted)
                    .map_err(|e| AgentLoopError::store(format!("Decryption failed: {}", e)))?;
                Ok(Some(decrypted))
            }
            None => Ok(None),
        }
    }

    async fn set_secret(
        &self,
        agent_id: AgentId,
        capability_id: &str,
        secret_name: &str,
        value: &str,
    ) -> Result<()> {
        let encryption = self.encryption()?;

        // Encrypt the value
        let encrypted = encryption
            .encrypt_string(value)
            .map_err(|e| AgentLoopError::store(format!("Encryption failed: {}", e)))?;

        self.db
            .upsert_capability_secret(agent_id.uuid(), capability_id, secret_name, encrypted)
            .await
            .map_err(|e| AgentLoopError::store(e.to_string()))?;

        Ok(())
    }

    async fn delete_secret(
        &self,
        agent_id: AgentId,
        capability_id: &str,
        secret_name: &str,
    ) -> Result<bool> {
        self.db
            .delete_capability_secret(agent_id.uuid(), capability_id, secret_name)
            .await
            .map_err(|e| AgentLoopError::store(e.to_string()))
    }

    async fn has_secret(
        &self,
        agent_id: AgentId,
        capability_id: &str,
        secret_name: &str,
    ) -> Result<bool> {
        self.db
            .has_capability_secret(agent_id.uuid(), capability_id, secret_name)
            .await
            .map_err(|e| AgentLoopError::store(e.to_string()))
    }

    async fn list_secrets(&self, agent_id: AgentId, capability_id: &str) -> Result<Vec<String>> {
        self.db
            .list_capability_secrets(agent_id.uuid(), capability_id)
            .await
            .map_err(|e| AgentLoopError::store(e.to_string()))
    }
}

// ============================================================================
// Factory functions
// ============================================================================

/// Create a database-backed capability secret store with encryption
pub fn create_db_capability_secret_store(
    db: Database,
    encryption: EncryptionService,
) -> DbCapabilitySecretStore {
    DbCapabilitySecretStore::new(db, encryption)
}

/// Create a database-backed capability secret store without encryption
/// (all operations will fail - encryption is required for secrets)
pub fn create_db_capability_secret_store_without_encryption(
    db: Database,
) -> DbCapabilitySecretStore {
    DbCapabilitySecretStore::new_without_encryption(db)
}

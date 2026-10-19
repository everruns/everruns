// Database-backed SessionStorageStore implementation
//
// This module implements the core SessionStorageStore trait for persisting
// session key/value pairs and encrypted secrets to the database.

use crate::kernel_imports::{
    contracts::error::AgentLoopError, contracts::error::Result, contracts::error::StoreResultExt,
    contracts::typed_id::SessionId, session_services::KeyInfo, session_services::SecretInfo,
    session_services::SessionStorageStore,
};
use async_trait::async_trait;
use everruns_contracts::runtime::{BoundSecret, BoundSecretInfo, SecretScope};

use crate::storage::encryption::EncryptionService;
use crate::storage::repositories::Database;
use crate::storage::{UpsertSessionKeyValue, UpsertSessionSecret};

// ============================================================================
// DbSessionStorageStore - Stores session data in database
// ============================================================================

/// Database-backed session storage store
///
/// Stores session key/value pairs and encrypted secrets in the database.
/// Used by tools that need to persist data within a session.
#[derive(Clone)]
pub struct DbSessionStorageStore {
    db: Database,
    encryption: Option<EncryptionService>,
}

impl DbSessionStorageStore {
    /// Create a new storage store with encryption enabled
    pub fn new(db: Database, encryption: EncryptionService) -> Self {
        Self {
            db,
            encryption: Some(encryption),
        }
    }

    /// Create a new storage store without encryption
    /// (secrets operations will fail)
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
impl SessionStorageStore for DbSessionStorageStore {
    // ========================================================================
    // Key/Value operations (plain text)
    // ========================================================================

    async fn set_value(&self, session_id: SessionId, key: &str, value: &str) -> Result<()> {
        let input = UpsertSessionKeyValue {
            session_id,
            key: key.to_string(),
            value: value.to_string(),
        };

        self.db.upsert_session_key_value(input).await.store_err()?;

        Ok(())
    }

    async fn get_value(&self, session_id: SessionId, key: &str) -> Result<Option<String>> {
        let row = self
            .db
            .get_session_key_value(session_id.uuid(), key)
            .await
            .store_err()?;

        Ok(row.map(|r| r.value))
    }

    async fn take_value(&self, session_id: SessionId, key: &str) -> Result<Option<String>> {
        self.db
            .take_session_key_value(session_id.uuid(), key)
            .await
            .map_err(|e| AgentLoopError::store(e.to_string()))
    }

    async fn delete_value(&self, session_id: SessionId, key: &str) -> Result<bool> {
        self.db
            .delete_session_key_value(session_id.uuid(), key)
            .await
            .map_err(|e| AgentLoopError::store(e.to_string()))
    }

    async fn list_keys(&self, session_id: SessionId) -> Result<Vec<KeyInfo>> {
        let rows = self
            .db
            .list_session_keys(session_id.uuid())
            .await
            .store_err()?;

        Ok(rows
            .into_iter()
            .map(|r| KeyInfo {
                key: r.key,
                created_at: r.created_at,
                updated_at: r.updated_at,
            })
            .collect())
    }

    // ========================================================================
    // Secret operations (encrypted)
    // ========================================================================

    async fn set_secret(&self, session_id: SessionId, name: &str, value: &str) -> Result<()> {
        let encryption = self.encryption()?;
        // THREAT[TM-BASH-030]: the runtime may not swap a bound key.
        if self
            .db
            .list_bound_secrets(session_id.uuid())
            .await
            .store_err()?
            .iter()
            .any(|row| row.name == name)
        {
            return Err(bound_error(name));
        }

        // Encrypt the value
        let encrypted = encryption
            .encrypt_string(value)
            .map_err(|e| AgentLoopError::store(format!("Encryption failed: {}", e)))?;

        let input = UpsertSessionSecret {
            session_id,
            name: name.to_string(),
            value_encrypted: encrypted,
            binding: None,
        };

        self.db.upsert_session_secret(input).await.store_err()?;

        Ok(())
    }

    async fn get_secret(&self, session_id: SessionId, name: &str) -> Result<Option<String>> {
        let encryption = self.encryption()?;

        let row = self
            .db
            .get_session_secret(session_id.uuid(), name)
            .await
            .store_err()?;

        match row {
            // THREAT[TM-BASH-030]: bound values are write-only.
            Some(r) if r.binding.is_some() => Err(bound_error(name)),
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

    async fn delete_secret(&self, session_id: SessionId, name: &str) -> Result<bool> {
        self.db
            .delete_session_secret(session_id.uuid(), name)
            .await
            .map_err(|e| AgentLoopError::store(e.to_string()))
    }

    async fn list_secrets(&self, session_id: SessionId) -> Result<Vec<SecretInfo>> {
        let rows = self
            .db
            .list_session_secrets(session_id.uuid())
            .await
            .store_err()?;

        Ok(rows
            .into_iter()
            .map(|r| SecretInfo {
                name: r.name,
                created_at: r.created_at,
                updated_at: r.updated_at,
            })
            .collect())
    }

    async fn list_bound_secrets(&self, session_id: SessionId) -> Result<Vec<BoundSecretInfo>> {
        let rows = self
            .db
            .list_bound_secrets(session_id.uuid())
            .await
            .store_err()?;
        Ok(rows.iter().map(bound_info).collect())
    }

    async fn resolve_bound_secrets(&self, session_id: SessionId) -> Result<Vec<BoundSecret>> {
        let rows = self
            .db
            .list_bound_secrets(session_id.uuid())
            .await
            .store_err()?;
        if rows.is_empty() {
            return Ok(Vec::new());
        }
        let encryption = self.encryption()?;
        rows.iter()
            .map(|row| {
                let value = encryption
                    .decrypt_to_string(&row.value_encrypted)
                    .map_err(|e| AgentLoopError::store(format!("Decryption failed: {}", e)))?;
                Ok(BoundSecret {
                    info: bound_info(row),
                    value,
                })
            })
            .collect()
    }
}

fn bound_error(name: &str) -> AgentLoopError {
    AgentLoopError::store(format!(
        "'{name}' is a bound secret: its value is write-only. Use ${name} in shell HTTP requests to the bound URL."
    ))
}

fn bound_info(row: &crate::storage::BoundSecretRow) -> BoundSecretInfo {
    BoundSecretInfo {
        name: row.name.clone(),
        binding: row.binding.0.clone(),
        scope: if row.scope == "agent" {
            SecretScope::Agent
        } else {
            SecretScope::Session
        },
    }
}

// ============================================================================
// Factory functions
// ============================================================================

/// Create a database-backed session storage store with encryption
pub fn create_db_session_storage_store(
    db: Database,
    encryption: EncryptionService,
) -> DbSessionStorageStore {
    DbSessionStorageStore::new(db, encryption)
}

/// Create a database-backed session storage store without encryption
/// (secrets operations will fail, but key/value operations will work)
pub fn create_db_session_storage_store_without_encryption(db: Database) -> DbSessionStorageStore {
    DbSessionStorageStore::new_without_encryption(db)
}

// Note: Integration tests for this module are covered by smoke tests
// since they require a running database and encryption service.

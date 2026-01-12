// LLM Resolver service for resolving models with decrypted provider credentials
//
// This service handles the resolution of LLM models with their provider credentials,
// including API key decryption. Used by gRPC service for worker communication.

use crate::storage::{EncryptionService, StorageBackend};
use anyhow::{anyhow, Result};
use std::sync::Arc;
use uuid::Uuid;

/// Resolved model with provider credentials (decrypted API key)
///
/// This is the service-layer representation of a model with its provider details.
/// Used for internal communication (gRPC) where decrypted credentials are needed.
#[derive(Debug, Clone)]
pub struct ResolvedModel {
    /// The model identifier (e.g., "gpt-4", "claude-3-opus")
    pub model_id: String,
    /// Provider type (e.g., "openai", "anthropic")
    pub provider_type: String,
    /// Decrypted API key (if available)
    pub api_key: Option<String>,
    /// Provider base URL override (if set)
    pub base_url: Option<String>,
}

pub struct LlmResolverService {
    db: Arc<StorageBackend>,
    encryption: Option<Arc<EncryptionService>>,
}

impl LlmResolverService {
    pub fn new(db: Arc<StorageBackend>, encryption: Option<Arc<EncryptionService>>) -> Self {
        Self { db, encryption }
    }

    /// Resolve a model by ID with decrypted provider credentials
    pub async fn resolve_model(&self, model_id: Uuid) -> Result<Option<ResolvedModel>> {
        let encryption = match &self.encryption {
            Some(enc) => enc.as_ref().clone(),
            None => return Err(anyhow!("Encryption service not configured")),
        };

        // Look up the model
        let model_row = self.db.get_llm_model(model_id).await?;

        let model_row = match model_row {
            Some(row) => row,
            None => return Ok(None),
        };

        // Look up the provider
        let provider_row = self.db.get_llm_provider(model_row.provider_id).await?;

        let provider_row = match provider_row {
            Some(row) => row,
            None => return Ok(None),
        };

        // Decrypt the API key
        let provider_with_key = self
            .db
            .get_provider_with_api_key(&provider_row, &encryption)?;

        Ok(Some(ResolvedModel {
            model_id: model_row.model_id,
            provider_type: provider_with_key.provider_type,
            api_key: provider_with_key.api_key,
            base_url: provider_with_key.base_url,
        }))
    }

    /// Resolve the default model with decrypted provider credentials
    pub async fn resolve_default_model(&self) -> Result<Option<ResolvedModel>> {
        let encryption = match &self.encryption {
            Some(enc) => enc.as_ref().clone(),
            None => return Err(anyhow!("Encryption service not configured")),
        };

        // Look up the default model
        let model_row = self.db.get_default_llm_model().await?;

        let model_row = match model_row {
            Some(row) => row,
            None => return Ok(None),
        };

        // Look up the provider
        let provider_row = self.db.get_llm_provider(model_row.provider_id).await?;

        let provider_row = match provider_row {
            Some(row) => row,
            None => return Ok(None),
        };

        // Decrypt the API key
        let provider_with_key = self
            .db
            .get_provider_with_api_key(&provider_row, &encryption)?;

        Ok(Some(ResolvedModel {
            model_id: model_row.model_id,
            provider_type: provider_with_key.provider_type,
            api_key: provider_with_key.api_key,
            base_url: provider_with_key.base_url,
        }))
    }

    /// Check if encryption service is available
    pub fn has_encryption(&self) -> bool {
        self.encryption.is_some()
    }
}

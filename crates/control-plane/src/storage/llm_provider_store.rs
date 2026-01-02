// Database-backed LlmProviderStore implementation
//
// This module implements the core LlmProviderStore trait for retrieving
// LLM provider and model configurations from the database.

use async_trait::async_trait;
use everruns_core::{
    llm_models::LlmProviderType,
    traits::{LlmProviderStore, ModelWithProvider},
    AgentLoopError, Result,
};
use uuid::Uuid;

use super::{encryption::EncryptionService, repositories::Database};

// ============================================================================
// DbLlmProviderStore - Retrieves LLM provider configurations from database
// ============================================================================

/// Database-backed LLM provider store
///
/// Retrieves LLM model and provider configurations from the database,
/// including decrypted API keys.
///
/// Used by ReasonAtom to resolve model and provider info dynamically.
#[derive(Clone)]
pub struct DbLlmProviderStore {
    db: Database,
    encryption: EncryptionService,
}

impl DbLlmProviderStore {
    pub fn new(db: Database, encryption: EncryptionService) -> Self {
        Self { db, encryption }
    }
}

#[async_trait]
impl LlmProviderStore for DbLlmProviderStore {
    async fn get_model_with_provider(&self, model_id: Uuid) -> Result<Option<ModelWithProvider>> {
        // Look up the model
        let model_row = self
            .db
            .get_llm_model(model_id)
            .await
            .map_err(|e| AgentLoopError::store(e.to_string()))?;

        let model_row = match model_row {
            Some(row) => row,
            None => return Ok(None),
        };

        // Look up the provider
        let provider_row = self
            .db
            .get_llm_provider(model_row.provider_id)
            .await
            .map_err(|e| AgentLoopError::store(e.to_string()))?;

        let provider_row = match provider_row {
            Some(row) => row,
            None => return Ok(None),
        };

        // Decrypt the API key
        let provider_with_key = self
            .db
            .get_provider_with_api_key(&provider_row, &self.encryption)
            .map_err(|e| AgentLoopError::store(e.to_string()))?;

        // Parse provider type
        let provider_type = parse_provider_type(&provider_with_key.provider_type);

        Ok(Some(ModelWithProvider {
            model: model_row.model_id,
            provider_type,
            api_key: provider_with_key.api_key,
            base_url: provider_with_key.base_url,
        }))
    }

    async fn get_default_model(&self) -> Result<Option<ModelWithProvider>> {
        // Look up the default model (is_default = true)
        let model_row = self
            .db
            .get_default_llm_model()
            .await
            .map_err(|e| AgentLoopError::store(e.to_string()))?;

        let model_row = match model_row {
            Some(row) => row,
            None => return Ok(None),
        };

        // Look up the provider
        let provider_row = self
            .db
            .get_llm_provider(model_row.provider_id)
            .await
            .map_err(|e| AgentLoopError::store(e.to_string()))?;

        let provider_row = match provider_row {
            Some(row) => row,
            None => return Ok(None),
        };

        // Decrypt the API key
        let provider_with_key = self
            .db
            .get_provider_with_api_key(&provider_row, &self.encryption)
            .map_err(|e| AgentLoopError::store(e.to_string()))?;

        // Parse provider type
        let provider_type = parse_provider_type(&provider_with_key.provider_type);

        Ok(Some(ModelWithProvider {
            model: model_row.model_id,
            provider_type,
            api_key: provider_with_key.api_key,
            base_url: provider_with_key.base_url,
        }))
    }
}

/// Parse provider type string to enum
fn parse_provider_type(provider_type_str: &str) -> LlmProviderType {
    match provider_type_str.to_lowercase().as_str() {
        "openai" => LlmProviderType::Openai,
        "anthropic" => LlmProviderType::Anthropic,
        "azure_openai" | "azure-openai" | "azureopenai" => LlmProviderType::AzureOpenAI,
        _ => LlmProviderType::Openai, // Default to OpenAI
    }
}

// ============================================================================
// Factory functions
// ============================================================================

/// Create a database-backed LLM provider store
pub fn create_db_llm_provider_store(
    db: Database,
    encryption: EncryptionService,
) -> DbLlmProviderStore {
    DbLlmProviderStore::new(db, encryption)
}

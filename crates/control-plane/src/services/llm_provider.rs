// LLM Provider service for business logic
//
// Merges config-based providers (readonly) with database providers.
// Database providers take priority over config providers with the same name.

use crate::config::ProvidersConfig;
use crate::storage::{
    models::{CreateLlmProviderRow, LlmProviderRow, UpdateLlmProvider},
    Database, EncryptionService,
};
use anyhow::{anyhow, Result};
use everruns_core::llm_models::LlmProvider;
use everruns_core::{LlmProviderStatus, LlmProviderType};
use std::collections::HashSet;
use std::sync::Arc;
use uuid::Uuid;

use crate::api::llm_providers::{CreateLlmProviderRequest, UpdateLlmProviderRequest};

pub struct LlmProviderService {
    db: Arc<Database>,
    encryption: Option<Arc<EncryptionService>>,
    config: Arc<ProvidersConfig>,
}

impl LlmProviderService {
    pub fn new(
        db: Arc<Database>,
        encryption: Option<Arc<EncryptionService>>,
        config: Arc<ProvidersConfig>,
    ) -> Self {
        Self {
            db,
            encryption,
            config,
        }
    }

    pub async fn create(&self, req: CreateLlmProviderRequest) -> Result<LlmProvider> {
        // Encrypt API key if provided
        let api_key_encrypted = if let Some(api_key) = &req.api_key {
            let encryption = self
                .encryption
                .as_ref()
                .ok_or_else(|| anyhow!("Encryption not configured. Cannot store API key."))?;
            Some(encryption.encrypt_string(api_key)?)
        } else {
            None
        };

        let input = CreateLlmProviderRow {
            name: req.name,
            provider_type: req.provider_type.to_string(),
            base_url: req.base_url,
            api_key_encrypted,
            settings: None,
        };

        let row = self.db.create_llm_provider(input).await?;
        Ok(Self::row_to_provider(&row))
    }

    pub async fn get(&self, id: Uuid) -> Result<Option<LlmProvider>> {
        // First check database
        if let Some(row) = self.db.get_llm_provider(id).await? {
            return Ok(Some(Self::row_to_provider(&row)));
        }

        // Then check config
        Ok(self.config.get_provider(id).cloned())
    }

    pub async fn list(&self) -> Result<Vec<LlmProvider>> {
        // Get database providers
        let db_rows = self.db.list_llm_providers().await?;
        let db_providers: Vec<LlmProvider> = db_rows.iter().map(Self::row_to_provider).collect();

        // Collect names of DB providers (they take priority) - use owned Strings to avoid borrow issues
        let db_names: HashSet<String> = db_providers.iter().map(|p| p.name.clone()).collect();

        // Add config providers that don't conflict with DB providers
        let mut result = db_providers;
        for config_provider in self.config.providers() {
            if !db_names.contains(&config_provider.name) {
                result.push(config_provider.clone());
            }
        }

        Ok(result)
    }

    /// Check if a provider is readonly (from config)
    pub fn is_readonly(&self, id: Uuid) -> bool {
        self.config.get_provider(id).is_some()
    }

    pub async fn update(
        &self,
        id: Uuid,
        req: UpdateLlmProviderRequest,
    ) -> Result<Option<LlmProvider>> {
        // Check if this is a config provider (readonly)
        if self.config.get_provider(id).is_some() {
            return Err(anyhow!(
                "Cannot modify read-only provider from configuration"
            ));
        }

        // Encrypt API key if provided
        let api_key_encrypted = if let Some(api_key) = &req.api_key {
            let encryption = self
                .encryption
                .as_ref()
                .ok_or_else(|| anyhow!("Encryption not configured. Cannot store API key."))?;
            Some(encryption.encrypt_string(api_key)?)
        } else {
            None
        };

        let input = UpdateLlmProvider {
            name: req.name,
            provider_type: req.provider_type.map(|t| t.to_string()),
            base_url: req.base_url,
            api_key_encrypted,
            status: req.status.map(|s| match s {
                LlmProviderStatus::Active => "active".to_string(),
                LlmProviderStatus::Disabled => "disabled".to_string(),
            }),
            settings: None,
        };

        let row = self.db.update_llm_provider(id, input).await?;
        Ok(row.as_ref().map(Self::row_to_provider))
    }

    pub async fn delete(&self, id: Uuid) -> Result<bool> {
        // Check if this is a config provider (readonly)
        if self.config.get_provider(id).is_some() {
            return Err(anyhow!(
                "Cannot delete read-only provider from configuration"
            ));
        }

        self.db.delete_llm_provider(id).await
    }

    fn row_to_provider(row: &LlmProviderRow) -> LlmProvider {
        LlmProvider {
            id: row.id,
            name: row.name.clone(),
            provider_type: row.provider_type.parse().unwrap_or(LlmProviderType::Openai),
            base_url: row.base_url.clone(),
            api_key_set: row.api_key_set,
            status: match row.status.as_str() {
                "active" => LlmProviderStatus::Active,
                _ => LlmProviderStatus::Disabled,
            },
            created_at: row.created_at,
            updated_at: row.updated_at,
            readonly: false, // Database providers are not readonly
        }
    }
}

// LLM Provider service for business logic

use crate::storage::{
    models::{CreateLlmProviderRow, LlmProviderRow, UpdateLlmProvider},
    Database, EncryptionService,
};
use anyhow::{anyhow, Result};
use everruns_core::llm_models::LlmProvider;
use everruns_core::{LlmProviderStatus, LlmProviderType};
use std::sync::Arc;
use uuid::Uuid;

use crate::llm_providers::{CreateLlmProviderRequest, UpdateLlmProviderRequest};

pub struct LlmProviderService {
    db: Arc<Database>,
    encryption: Option<Arc<EncryptionService>>,
}

impl LlmProviderService {
    pub fn new(db: Arc<Database>, encryption: Option<Arc<EncryptionService>>) -> Self {
        Self { db, encryption }
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
        let row = self.db.get_llm_provider(id).await?;
        Ok(row.as_ref().map(Self::row_to_provider))
    }

    pub async fn list(&self) -> Result<Vec<LlmProvider>> {
        let rows = self.db.list_llm_providers().await?;
        Ok(rows.iter().map(Self::row_to_provider).collect())
    }

    pub async fn update(
        &self,
        id: Uuid,
        req: UpdateLlmProviderRequest,
    ) -> Result<Option<LlmProvider>> {
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
        }
    }
}

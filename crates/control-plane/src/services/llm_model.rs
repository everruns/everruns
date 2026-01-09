// LLM Model service for business logic
//
// Merges config-based models (readonly) with database models.
// Database models take priority over config models with the same provider+model_id.

use crate::config::{model_with_provider_to_model, ProvidersConfig};
use crate::storage::{
    models::{CreateLlmModelRow, LlmModelRow, LlmModelWithProviderRow, UpdateLlmModel},
    Database,
};
use anyhow::{anyhow, Result};
use everruns_core::{
    get_model_profile, LlmModel, LlmModelStatus, LlmModelWithProvider, LlmProviderType,
};
use std::collections::HashSet;
use std::sync::Arc;
use uuid::Uuid;

use crate::api::llm_models::{CreateLlmModelRequest, UpdateLlmModelRequest};

pub struct LlmModelService {
    db: Arc<Database>,
    config: Arc<ProvidersConfig>,
}

impl LlmModelService {
    pub fn new(db: Arc<Database>, config: Arc<ProvidersConfig>) -> Self {
        Self { db, config }
    }

    pub async fn create(&self, provider_id: Uuid, req: CreateLlmModelRequest) -> Result<LlmModel> {
        // Check if this is a config provider (readonly)
        if self.config.get_provider(provider_id).is_some() {
            return Err(anyhow!(
                "Cannot add models to read-only provider from configuration"
            ));
        }

        // If setting this model as default, clear all other defaults first ("last wins")
        if req.is_default {
            self.db.clear_all_model_defaults().await?;
        }

        let input = CreateLlmModelRow {
            provider_id,
            model_id: req.model_id,
            display_name: req.display_name,
            capabilities: req.capabilities,
            is_default: req.is_default,
        };

        let row = self.db.create_llm_model(input).await?;
        Ok(Self::row_to_model(&row))
    }

    pub async fn get_with_provider(&self, id: Uuid) -> Result<Option<LlmModelWithProvider>> {
        // First check database
        if let Some(row) = self.db.get_llm_model_with_provider(id).await? {
            return Ok(Some(Self::row_to_model_with_provider(&row)));
        }

        // Then check config
        Ok(self.config.get_model(id).cloned())
    }

    pub async fn list_for_provider(&self, provider_id: Uuid) -> Result<Vec<LlmModel>> {
        // Get database models for this provider
        let db_rows = self.db.list_llm_models_for_provider(provider_id).await?;
        let db_models: Vec<LlmModel> = db_rows.iter().map(Self::row_to_model).collect();

        // Collect model_ids of DB models (they take priority) - use owned Strings to avoid borrow issues
        let db_model_ids: HashSet<String> = db_models.iter().map(|m| m.model_id.clone()).collect();

        // Add config models that don't conflict with DB models
        let mut result = db_models;
        for config_model in self.config.get_models_for_provider(provider_id) {
            if !db_model_ids.contains(&config_model.model_id) {
                result.push(model_with_provider_to_model(config_model));
            }
        }

        Ok(result)
    }

    pub async fn list_all(&self) -> Result<Vec<LlmModelWithProvider>> {
        // Get database models
        let db_rows = self.db.list_all_llm_models().await?;
        let db_models: Vec<LlmModelWithProvider> = db_rows
            .iter()
            .map(Self::row_to_model_with_provider)
            .collect();

        // Collect (provider_id, model_id) pairs of DB models (they take priority) - use owned Strings to avoid borrow issues
        let db_model_keys: HashSet<(Uuid, String)> = db_models
            .iter()
            .map(|m| (m.provider_id, m.model_id.clone()))
            .collect();

        // Add config models that don't conflict with DB models
        let mut result = db_models;
        for config_model in self.config.models() {
            let key = (config_model.provider_id, config_model.model_id.clone());
            if !db_model_keys.contains(&key) {
                result.push(config_model.clone());
            }
        }

        Ok(result)
    }

    /// Check if a model is readonly (from config)
    pub fn is_readonly(&self, id: Uuid) -> bool {
        self.config.get_model(id).is_some()
    }

    pub async fn update(&self, id: Uuid, req: UpdateLlmModelRequest) -> Result<Option<LlmModel>> {
        // Check if this is a config model (readonly)
        if self.config.get_model(id).is_some() {
            return Err(anyhow!("Cannot modify read-only model from configuration"));
        }

        // If setting this model as default, clear all other defaults first ("last wins")
        if req.is_default == Some(true) {
            self.db.clear_all_model_defaults().await?;
        }

        let input = UpdateLlmModel {
            model_id: req.model_id,
            display_name: req.display_name,
            capabilities: req.capabilities,
            is_default: req.is_default,
            status: req.status.map(|s| match s {
                LlmModelStatus::Active => "active".to_string(),
                LlmModelStatus::Disabled => "disabled".to_string(),
            }),
        };

        let row = self.db.update_llm_model(id, input).await?;
        Ok(row.as_ref().map(Self::row_to_model))
    }

    pub async fn delete(&self, id: Uuid) -> Result<bool> {
        // Check if this is a config model (readonly)
        if self.config.get_model(id).is_some() {
            return Err(anyhow!("Cannot delete read-only model from configuration"));
        }

        self.db.delete_llm_model(id).await
    }

    /// Get the default model (checks both config and database)
    pub async fn get_default(&self) -> Result<Option<LlmModelWithProvider>> {
        // First check database for a default model
        if let Some(row) = self.db.get_default_llm_model().await? {
            return Ok(Some(Self::row_to_model_with_provider(&row)));
        }

        // Fall back to config default model
        Ok(self.config.get_default_model().cloned())
    }

    fn row_to_model(row: &LlmModelRow) -> LlmModel {
        let capabilities: Vec<String> =
            serde_json::from_value(row.capabilities.clone()).unwrap_or_default();
        LlmModel {
            id: row.id,
            provider_id: row.provider_id,
            model_id: row.model_id.clone(),
            display_name: row.display_name.clone(),
            capabilities,
            is_default: row.is_default,
            status: match row.status.as_str() {
                "active" => LlmModelStatus::Active,
                _ => LlmModelStatus::Disabled,
            },
            created_at: row.created_at,
            updated_at: row.updated_at,
            readonly: false, // Database models are not readonly
        }
    }

    fn row_to_model_with_provider(row: &LlmModelWithProviderRow) -> LlmModelWithProvider {
        let capabilities: Vec<String> =
            serde_json::from_value(row.capabilities.clone()).unwrap_or_default();
        let provider_type: LlmProviderType =
            row.provider_type.parse().unwrap_or(LlmProviderType::Openai);

        // Look up profile based on provider_type and model_id (readonly, not from DB)
        let profile = get_model_profile(&provider_type, &row.model_id);

        LlmModelWithProvider {
            id: row.id,
            provider_id: row.provider_id,
            model_id: row.model_id.clone(),
            display_name: row.display_name.clone(),
            capabilities,
            is_default: row.is_default,
            status: match row.status.as_str() {
                "active" => LlmModelStatus::Active,
                _ => LlmModelStatus::Disabled,
            },
            created_at: row.created_at,
            updated_at: row.updated_at,
            provider_name: row.provider_name.clone(),
            provider_type,
            profile,
            readonly: false, // Database models are not readonly
        }
    }
}

// LLM Model service for business logic

use anyhow::Result;
use everruns_contracts::{LlmModel, LlmModelStatus, LlmModelWithProvider, LlmProviderType};
use everruns_storage::{
    models::{CreateLlmModelRow, LlmModelRow, LlmModelWithProviderRow, UpdateLlmModel},
    Database,
};
use std::sync::Arc;
use uuid::Uuid;

use crate::llm_models::{CreateLlmModelRequest, UpdateLlmModelRequest};

pub struct LlmModelService {
    db: Arc<Database>,
}

impl LlmModelService {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    pub async fn create(&self, provider_id: Uuid, req: CreateLlmModelRequest) -> Result<LlmModel> {
        let input = CreateLlmModelRow {
            provider_id,
            model_id: req.model_id,
            display_name: req.display_name,
            capabilities: req.capabilities,
            context_window: req.context_window,
            is_default: req.is_default,
        };

        let row = self.db.create_llm_model(input).await?;
        Ok(Self::row_to_model(&row))
    }

    pub async fn get(&self, id: Uuid) -> Result<Option<LlmModel>> {
        let row = self.db.get_llm_model(id).await?;
        Ok(row.as_ref().map(Self::row_to_model))
    }

    pub async fn list_for_provider(&self, provider_id: Uuid) -> Result<Vec<LlmModel>> {
        let rows = self.db.list_llm_models_for_provider(provider_id).await?;
        Ok(rows.iter().map(Self::row_to_model).collect())
    }

    pub async fn list_all(&self) -> Result<Vec<LlmModelWithProvider>> {
        let rows = self.db.list_all_llm_models().await?;
        Ok(rows.iter().map(Self::row_to_model_with_provider).collect())
    }

    pub async fn update(&self, id: Uuid, req: UpdateLlmModelRequest) -> Result<Option<LlmModel>> {
        let input = UpdateLlmModel {
            model_id: req.model_id,
            display_name: req.display_name,
            capabilities: req.capabilities,
            context_window: req.context_window,
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
        self.db.delete_llm_model(id).await
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
            context_window: row.context_window,
            is_default: row.is_default,
            status: match row.status.as_str() {
                "active" => LlmModelStatus::Active,
                _ => LlmModelStatus::Disabled,
            },
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }

    fn row_to_model_with_provider(row: &LlmModelWithProviderRow) -> LlmModelWithProvider {
        let capabilities: Vec<String> =
            serde_json::from_value(row.capabilities.clone()).unwrap_or_default();
        LlmModelWithProvider {
            id: row.id,
            provider_id: row.provider_id,
            model_id: row.model_id.clone(),
            display_name: row.display_name.clone(),
            capabilities,
            context_window: row.context_window,
            is_default: row.is_default,
            status: match row.status.as_str() {
                "active" => LlmModelStatus::Active,
                _ => LlmModelStatus::Disabled,
            },
            created_at: row.created_at,
            updated_at: row.updated_at,
            provider_name: row.provider_name.clone(),
            provider_type: row.provider_type.parse().unwrap_or(LlmProviderType::Openai),
        }
    }
}

// In-memory storage: LLM Providers, LLM Models, LLM Models (continued), LLM Generations (Usage Tracking)

use anyhow::Result;

use super::super::models::*;
use super::InMemoryDatabase;
use everruns_core::{ModelId, ProviderId};
use uuid::Uuid;

impl InMemoryDatabase {
    // ============================================
    // LLM Providers
    // ============================================

    pub async fn create_llm_provider(
        &self,
        org_id: i64,
        input: CreateLlmProviderRow,
    ) -> Result<LlmProviderRow> {
        let now = Self::now();
        let id = ProviderId::new();
        let api_key_set = input.api_key_encrypted.is_some();
        let row = LlmProviderRow {
            id,
            org_id,
            name: input.name,
            provider_type: input.provider_type,
            base_url: input.base_url,
            api_key_encrypted: input.api_key_encrypted,
            api_key_set,
            status: "active".to_string(), // Default status for new providers
            settings: input.settings.unwrap_or(serde_json::json!({})),
            last_synced_at: None,
            created_at: now,
            updated_at: now,
        };
        self.llm_providers.write().insert(id, row.clone());
        Ok(row)
    }

    /// Create a provider with a specific ID (for seeding)
    /// Returns None if provider already exists (idempotent)
    /// Create or update LLM provider with a specific ID (for seeding).
    /// Returns Some(row) if created or updated, None if unchanged.
    pub async fn create_llm_provider_with_id(
        &self,
        org_id: i64,
        id: Uuid,
        input: CreateLlmProviderRow,
    ) -> Result<Option<LlmProviderRow>> {
        let id = ProviderId::from_uuid(id);
        let mut providers = self.llm_providers.write();
        let now = Self::now();

        if let Some(existing) = providers.get(&id) {
            if existing.name == input.name && existing.provider_type == input.provider_type {
                return Ok(None); // Unchanged
            }
            let row = LlmProviderRow {
                name: input.name,
                provider_type: input.provider_type,
                updated_at: now,
                ..existing.clone()
            };
            providers.insert(id, row.clone());
            return Ok(Some(row));
        }

        let api_key_set = input.api_key_encrypted.is_some();
        let row = LlmProviderRow {
            id,
            org_id,
            name: input.name,
            provider_type: input.provider_type,
            base_url: input.base_url,
            api_key_encrypted: input.api_key_encrypted,
            api_key_set,
            status: "active".to_string(),
            settings: input.settings.unwrap_or(serde_json::json!({})),
            last_synced_at: None,
            created_at: now,
            updated_at: now,
        };
        providers.insert(id, row.clone());
        Ok(Some(row))
    }

    pub async fn get_llm_provider(&self, org_id: i64, id: Uuid) -> Result<Option<LlmProviderRow>> {
        Ok(self
            .llm_providers
            .read()
            .get(&ProviderId::from_uuid(id))
            .filter(|p| p.org_id == org_id)
            .cloned())
    }

    pub async fn list_llm_providers(&self, org_id: i64) -> Result<Vec<LlmProviderRow>> {
        let providers = self.llm_providers.read();
        let mut result: Vec<_> = providers
            .values()
            .filter(|p| p.org_id == org_id)
            .cloned()
            .collect();
        result.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(result)
    }

    pub async fn update_llm_provider(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateLlmProvider,
    ) -> Result<Option<LlmProviderRow>> {
        let id = ProviderId::from_uuid(id);
        let mut providers = self.llm_providers.write();
        if let Some(provider) = providers.get_mut(&id) {
            if provider.org_id != org_id {
                return Ok(None);
            }
            if let Some(name) = input.name {
                provider.name = name;
            }
            if let Some(base_url) = input.base_url {
                provider.base_url = Some(base_url);
            }
            if let Some(api_key_encrypted) = input.api_key_encrypted {
                provider.api_key_encrypted = Some(api_key_encrypted);
                provider.api_key_set = true;
            }
            if let Some(status) = input.status {
                provider.status = status;
            }
            if let Some(settings) = input.settings {
                provider.settings = settings;
            }
            provider.updated_at = Self::now();
            return Ok(Some(provider.clone()));
        }
        Ok(None)
    }

    pub async fn delete_llm_provider(&self, org_id: i64, id: Uuid) -> Result<bool> {
        let id = ProviderId::from_uuid(id);
        // Check org_id before deletion
        {
            let providers = self.llm_providers.read();
            if let Some(provider) = providers.get(&id) {
                if provider.org_id != org_id {
                    return Ok(false);
                }
            } else {
                return Ok(false);
            }
        }
        // Delete models first
        {
            let mut models = self.llm_models.write();
            let to_remove: Vec<ModelId> = models
                .iter()
                .filter(|(_, m)| m.provider_id == id)
                .map(|(mid, _)| *mid)
                .collect();
            for mid in to_remove {
                models.remove(&mid);
            }
        }
        Ok(self.llm_providers.write().remove(&id).is_some())
    }

    /// Update provider's last_synced_at timestamp
    pub async fn update_provider_last_synced(
        &self,
        org_id: i64,
        id: Uuid,
        last_synced_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<()> {
        let id = ProviderId::from_uuid(id);
        if let Some(provider) = self.llm_providers.write().get_mut(&id) {
            if provider.org_id != org_id {
                return Ok(());
            }
            provider.last_synced_at = Some(last_synced_at);
            provider.updated_at = Self::now();
        }
        Ok(())
    }

    /// Get a provider with its decrypted API key
    pub fn get_provider_with_api_key(
        &self,
        provider: &LlmProviderRow,
        encryption: &super::super::EncryptionService,
    ) -> Result<LlmProviderWithApiKey> {
        let api_key = if let Some(ref encrypted) = provider.api_key_encrypted {
            Some(encryption.decrypt_to_string(encrypted)?)
        } else {
            None
        };

        // Convert settings from sqlx JsonValue to serde_json::Value
        let settings: serde_json::Value =
            serde_json::from_str(&provider.settings.to_string()).unwrap_or_default();

        Ok(LlmProviderWithApiKey {
            id: provider.id,
            name: provider.name.clone(),
            provider_type: provider.provider_type.clone(),
            base_url: provider.base_url.clone(),
            api_key,
            settings,
        })
    }

    // ============================================
    // LLM Models
    // ============================================

    pub async fn get_default_llm_model(
        &self,
        org_id: i64,
    ) -> Result<Option<LlmModelWithProviderRow>> {
        let org_settings = self.org_settings.read();
        let default_model_id = org_settings.get(&org_id).and_then(|s| s.default_model_id);
        let default_model_id = match default_model_id {
            Some(id) => id,
            None => return Ok(None),
        };
        let models = self.llm_models.read();
        let providers = self.llm_providers.read();

        if let Some(model) = models.get(&default_model_id)
            && model.org_id == org_id
            && let Some(provider) = providers.get(&model.provider_id)
            && provider.org_id == org_id
            && model.status == "active"
            && provider.status == "active"
        {
            return Ok(Some(LlmModelWithProviderRow {
                id: model.id,
                org_id: model.org_id,
                provider_id: model.provider_id,
                model_id: model.model_id.clone(),
                display_name: model.display_name.clone(),
                capabilities: model.capabilities.clone(),
                is_favorite: model.is_favorite,
                enabled: model.enabled,
                status: model.status.clone(),
                source: model.source.clone(),
                last_seen_at: model.last_seen_at,
                provider_metadata: model.provider_metadata.clone(),
                created_at: model.created_at,
                updated_at: model.updated_at,
                provider_name: provider.name.clone(),
                provider_type: provider.provider_type.clone(),
            }));
        }
        Ok(None)
    }

    // ============================================
    // LLM Models (continued)
    // ============================================

    pub async fn create_llm_model(
        &self,
        org_id: i64,
        input: CreateLlmModelRow,
    ) -> Result<LlmModelRow> {
        let now = Self::now();
        let id = ModelId::new();
        let row = LlmModelRow {
            id,
            org_id,
            provider_id: input.provider_id,
            model_id: input.model_id,
            display_name: input.display_name,
            capabilities: serde_json::to_value(&input.capabilities)?,
            is_favorite: input.is_favorite,
            enabled: input.enabled,
            status: "active".to_string(),
            source: input.source,
            last_seen_at: None,
            provider_metadata: input.provider_metadata,
            created_at: now,
            updated_at: now,
        };
        self.llm_models.write().insert(id, row.clone());
        Ok(row)
    }

    /// Create or update a model with a specific ID (for seeding).
    /// Returns Some(row) if created or updated, None if unchanged.
    pub async fn create_llm_model_with_id(
        &self,
        org_id: i64,
        id: Uuid,
        input: CreateLlmModelRow,
    ) -> Result<Option<LlmModelRow>> {
        let id = ModelId::from_uuid(id);
        let mut models = self.llm_models.write();
        let now = Self::now();

        if let Some(existing) = models.get(&id).cloned() {
            // Check if seed-controlled fields differ
            if existing.display_name == input.display_name
                && existing.is_favorite == input.is_favorite
                && existing.enabled == input.enabled
            {
                return Ok(None); // Unchanged
            }
            let row = LlmModelRow {
                display_name: input.display_name,
                is_favorite: input.is_favorite,
                enabled: input.enabled,
                updated_at: now,
                ..existing
            };
            models.insert(id, row.clone());
            return Ok(Some(row));
        }

        let row = LlmModelRow {
            id,
            org_id,
            provider_id: input.provider_id,
            model_id: input.model_id,
            display_name: input.display_name,
            capabilities: serde_json::to_value(&input.capabilities)?,
            is_favorite: input.is_favorite,
            enabled: input.enabled,
            status: "active".to_string(),
            source: input.source,
            last_seen_at: None,
            provider_metadata: input.provider_metadata,
            created_at: now,
            updated_at: now,
        };
        models.insert(id, row.clone());
        Ok(Some(row))
    }

    pub async fn get_llm_model(&self, org_id: i64, id: Uuid) -> Result<Option<LlmModelRow>> {
        let id = ModelId::from_uuid(id);
        Ok(self
            .llm_models
            .read()
            .get(&id)
            .filter(|m| m.org_id == org_id)
            .cloned())
    }

    pub async fn get_llm_model_with_provider(
        &self,
        org_id: i64,
        id: Uuid,
    ) -> Result<Option<LlmModelWithProviderRow>> {
        let id = ModelId::from_uuid(id);
        let models = self.llm_models.read();
        let providers = self.llm_providers.read();

        if let Some(model) = models.get(&id)
            && model.org_id == org_id
            && let Some(provider) = providers.get(&model.provider_id)
            && provider.org_id == org_id
        {
            return Ok(Some(LlmModelWithProviderRow {
                id: model.id,
                org_id: model.org_id,
                provider_id: model.provider_id,
                model_id: model.model_id.clone(),
                display_name: model.display_name.clone(),
                capabilities: model.capabilities.clone(),
                is_favorite: model.is_favorite,
                enabled: model.enabled,
                status: model.status.clone(),
                source: model.source.clone(),
                last_seen_at: model.last_seen_at,
                provider_metadata: model.provider_metadata.clone(),
                created_at: model.created_at,
                updated_at: model.updated_at,
                provider_name: provider.name.clone(),
                provider_type: provider.provider_type.clone(),
            }));
        }
        Ok(None)
    }

    pub async fn list_llm_models_for_provider(
        &self,
        org_id: i64,
        provider_id: Uuid,
    ) -> Result<Vec<LlmModelRow>> {
        let provider_id = ProviderId::from_uuid(provider_id);
        let models = self.llm_models.read();
        let mut result: Vec<_> = models
            .values()
            .filter(|m| m.provider_id == provider_id && m.org_id == org_id)
            .cloned()
            .collect();
        result.sort_by(|a, b| a.display_name.cmp(&b.display_name));
        Ok(result)
    }

    pub async fn list_all_llm_models(&self, org_id: i64) -> Result<Vec<LlmModelWithProviderRow>> {
        let models = self.llm_models.read();
        let providers = self.llm_providers.read();

        let mut result: Vec<_> = models
            .values()
            .filter(|model| model.org_id == org_id && model.status == "active")
            .filter_map(|model| {
                providers
                    .get(&model.provider_id)
                    .filter(|provider| provider.org_id == org_id && provider.status == "active")
                    .map(|provider| LlmModelWithProviderRow {
                        id: model.id,
                        org_id: model.org_id,
                        provider_id: model.provider_id,
                        model_id: model.model_id.clone(),
                        display_name: model.display_name.clone(),
                        capabilities: model.capabilities.clone(),
                        is_favorite: model.is_favorite,
                        enabled: model.enabled,
                        status: model.status.clone(),
                        source: model.source.clone(),
                        last_seen_at: model.last_seen_at,
                        provider_metadata: model.provider_metadata.clone(),
                        created_at: model.created_at,
                        updated_at: model.updated_at,
                        provider_name: provider.name.clone(),
                        provider_type: provider.provider_type.clone(),
                    })
            })
            .collect();
        // Sort by enabled first, then favorite, then display_name
        result.sort_by(|a, b| {
            b.enabled
                .cmp(&a.enabled)
                .then_with(|| b.is_favorite.cmp(&a.is_favorite))
                .then_with(|| a.display_name.cmp(&b.display_name))
        });
        Ok(result)
    }

    pub async fn update_llm_model(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateLlmModel,
    ) -> Result<Option<LlmModelRow>> {
        let id = ModelId::from_uuid(id);
        let mut models = self.llm_models.write();
        // Check existence and org ownership
        if models.get(&id).is_none_or(|m| m.org_id != org_id) {
            return Ok(None);
        }
        let model = models.get_mut(&id).unwrap();
        if let Some(display_name) = input.display_name {
            model.display_name = display_name;
        }
        if let Some(capabilities) = input.capabilities {
            model.capabilities = serde_json::to_value(&capabilities)?;
        }
        if let Some(is_favorite) = input.is_favorite {
            model.is_favorite = is_favorite;
        }
        if let Some(enabled) = input.enabled {
            model.enabled = enabled;
        }
        if let Some(status) = input.status {
            model.status = status;
        }
        model.updated_at = Self::now();
        Ok(Some(model.clone()))
    }

    pub async fn delete_llm_model(&self, org_id: i64, id: Uuid) -> Result<bool> {
        let id = ModelId::from_uuid(id);
        let mut models = self.llm_models.write();
        if let Some(model) = models.get(&id) {
            if model.org_id != org_id {
                return Ok(false);
            }
        } else {
            return Ok(false);
        }
        Ok(models.remove(&id).is_some())
    }

    pub async fn get_llm_model_by_model_id(
        &self,
        org_id: i64,
        model_id: &str,
    ) -> Result<Option<LlmModelWithProviderRow>> {
        let models = self.llm_models.read();
        let providers = self.llm_providers.read();

        for model in models.values() {
            if model.model_id == model_id
                && model.org_id == org_id
                && model.status == "active"
                && let Some(provider) = providers.get(&model.provider_id)
                && provider.org_id == org_id
                && provider.status == "active"
            {
                return Ok(Some(LlmModelWithProviderRow {
                    id: model.id,
                    org_id: model.org_id,
                    provider_id: model.provider_id,
                    model_id: model.model_id.clone(),
                    display_name: model.display_name.clone(),
                    capabilities: model.capabilities.clone(),
                    is_favorite: model.is_favorite,
                    enabled: model.enabled,
                    status: model.status.clone(),
                    source: model.source.clone(),
                    last_seen_at: model.last_seen_at,
                    provider_metadata: model.provider_metadata.clone(),
                    created_at: model.created_at,
                    updated_at: model.updated_at,
                    provider_name: provider.name.clone(),
                    provider_type: provider.provider_type.clone(),
                }));
            }
        }
        Ok(None)
    }

    // ============================================
    // LLM Generations (Usage Tracking)
    // ============================================
    //
    // In-memory implementations for dev mode.
    // Note: llm_generations table is not stored in memory since it's only
    // used for analytics. We just update the denormalized totals.

    #[allow(clippy::too_many_arguments)]
    pub async fn create_llm_generation(
        &self,
        _org_id: i64,
        _session_id: Uuid,
        _turn_id: Option<Uuid>,
        _event_id: Option<Uuid>,
        _model: String,
        _provider: Option<String>,
        _input_tokens: i64,
        _output_tokens: i64,
        _cache_read_tokens: i64,
        _cache_creation_tokens: i64,
        _duration_ms: Option<i32>,
        _finish_reason: Option<String>,
        _created_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<()> {
        // In dev mode, we don't store individual generations
        // Usage totals are updated via increment_session_usage/increment_agent_usage
        Ok(())
    }

    pub async fn increment_session_usage(
        &self,
        session_id: Uuid,
        input_tokens: i64,
        output_tokens: i64,
        cache_read_tokens: i64,
        cache_creation_tokens: i64,
    ) -> Result<()> {
        let mut sessions = self.sessions.write();
        if let Some(session) = sessions.get_mut(&session_id) {
            session.total_input_tokens += input_tokens;
            session.total_output_tokens += output_tokens;
            session.total_cache_read_tokens += cache_read_tokens;
            session.total_cache_creation_tokens += cache_creation_tokens;
            // Update updated_at on every update (mimics DB trigger)
            session.updated_at = Self::now();
        }
        Ok(())
    }

    pub async fn increment_agent_usage(
        &self,
        agent_id: Uuid,
        input_tokens: i64,
        output_tokens: i64,
        cache_read_tokens: i64,
        cache_creation_tokens: i64,
    ) -> Result<()> {
        let mut agents = self.agents.write();
        if let Some(agent) = agents.get_mut(&agent_id) {
            agent.total_input_tokens += input_tokens;
            agent.total_output_tokens += output_tokens;
            agent.total_cache_read_tokens += cache_read_tokens;
            agent.total_cache_creation_tokens += cache_creation_tokens;
        }
        Ok(())
    }
}

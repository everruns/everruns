use super::*;

impl ModelService {
    pub(super) fn order_personal_catalog(rows: &mut [ModelWithProviderRow]) {
        rows.sort_by(|a, b| {
            a.provider_id
                .uuid()
                .cmp(&b.provider_id.uuid())
                .then_with(|| {
                    if a.provider_type == "chatgpt" {
                        let rank = |row: &ModelWithProviderRow| {
                            row.provider_metadata
                                .as_ref()
                                .and_then(|m| m["catalog_order"].as_u64())
                                .unwrap_or(u64::MAX)
                        };
                        rank(a).cmp(&rank(b))
                    } else {
                        std::cmp::Ordering::Equal
                    }
                })
        });
    }

    pub(super) async fn get_visible_provider(
        &self,
        caller: &Caller,
        id: Uuid,
    ) -> Result<crate::storage::models::ProviderRow> {
        let provider = self.get_provider(caller.org_id, id).await?;
        if !crate::services::chatgpt::visible(&provider.settings, caller) {
            return Err(ResourceNotFoundError::new("Provider").into());
        }
        Ok(provider)
    }
    pub(super) async fn visible_provider_ids(
        &self,
        caller: &Caller,
    ) -> Result<std::collections::HashSet<ProviderId>> {
        Ok(self
            .db
            .list_providers(caller.org_id)
            .await?
            .into_iter()
            .filter(|p| crate::services::chatgpt::visible(&p.settings, caller))
            .map(|p| p.id)
            .collect())
    }

    pub(super) async fn get_provider(
        &self,
        org_id: i64,
        provider_id: Uuid,
    ) -> Result<crate::storage::models::ProviderRow> {
        self.db
            .get_provider(org_id, provider_id)
            .await?
            .ok_or_else(|| ResourceNotFoundError::new("Provider").into())
    }

    pub(super) fn require_unmanaged_provider(
        provider: &crate::storage::models::ProviderRow,
    ) -> Result<()> {
        if provider.managed {
            return Err(Self::managed_catalog_error());
        }
        Ok(())
    }

    pub(super) fn managed_catalog_error() -> anyhow::Error {
        everruns_core::PolicyError::denied(
            "provider_managed",
            "This provider's model catalog is managed by the host and cannot be modified.",
        )
        .into()
    }

    pub(super) fn row_to_model(row: &ModelRow) -> Model {
        let capabilities: Vec<String> =
            serde_json::from_value(row.capabilities.clone()).unwrap_or_default();
        Model {
            id: row.id,
            provider_id: row.provider_id,
            model_id: row.model_id.clone(),
            profile_key: crate::services::model_catalog::key(row.provider_metadata.as_ref()),
            service: crate::services::model_catalog::service(row.provider_metadata.as_ref()),
            display_name: row.display_name.clone(),
            capabilities,
            enabled: row.enabled,
            is_favorite: row.is_favorite,
            source: row.source.parse().unwrap_or(ModelSource::Manual),
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }

    pub(super) fn row_to_model_with_provider(row: &ModelWithProviderRow) -> ModelWithProvider {
        let capabilities: Vec<String> =
            serde_json::from_value(row.capabilities.clone()).unwrap_or_default();
        let provider_type: DriverId = row.provider_type.parse().unwrap_or(DriverId::OpenAI);

        let key = crate::services::model_catalog::key(row.provider_metadata.as_ref());
        let stored = everruns_contracts::model_profile_data::profile_entries_for_provider(
            &row.provider_type,
        )
        .into_iter()
        .find(|e| e.key == key)
        .map(|e| e.profile)
        .or_else(|| crate::services::model_catalog::profile(row.provider_metadata.as_ref()));
        let discovered = Self::extract_discovered_profile(row);
        let profile = match (stored, discovered) {
            (Some(curated), Some(discovered)) => Some(Self::merge_profiles(curated, discovered)),
            (curated, discovered) => curated.or(discovered),
        };

        // A model is healthy when its provider is active and has an API key
        // configured. This will likely grow to include live reachability
        // checks; keep the derivation in one place.
        let healthy = row.provider_status == "active"
            && (row.provider_api_key_set || provider_type == DriverId::LlmSim);

        // Vendor/brand tag from the model registry (drives UI branding),
        // independent of the configured provider type.
        let model_vendor = everruns_contracts::model_profile_data::all_profile_entries()
            .into_iter()
            .find(|entry| entry.key == key)
            .map(|entry| entry.vendor);

        ModelWithProvider {
            id: row.id,
            provider_id: row.provider_id,
            model_id: row.model_id.clone(),
            profile_key: crate::services::model_catalog::key(row.provider_metadata.as_ref()),
            service: crate::services::model_catalog::service(row.provider_metadata.as_ref()),
            display_name: row.display_name.clone(),
            capabilities,
            enabled: row.enabled,
            is_favorite: row.is_favorite,
            source: row.source.parse().unwrap_or(ModelSource::Manual),
            created_at: row.created_at,
            updated_at: row.updated_at,
            provider_name: row.provider_name.clone(),
            provider_type,
            healthy,
            profile,
            model_vendor,
        }
    }

    /// Extract the discovered profile from provider_metadata JSON.
    pub(super) fn extract_discovered_profile(row: &ModelWithProviderRow) -> Option<ModelProfile> {
        let metadata = row.provider_metadata.as_ref()?;
        let profile_val = metadata.get("discovered_profile")?;
        serde_json::from_value(profile_val.clone()).ok()
    }

    /// Merge a hardcoded profile with a discovered profile.
    /// Hardcoded values take precedence; discovered values fill gaps.
    pub(super) fn merge_profiles(
        hardcoded: ModelProfile,
        discovered: ModelProfile,
    ) -> ModelProfile {
        ModelProfile {
            // Hardcoded always wins for curated fields
            name: hardcoded.name,
            family: hardcoded.family,
            description: hardcoded.description.or(discovered.description),
            release_date: hardcoded.release_date.or(discovered.release_date),
            last_updated: hardcoded.last_updated.or(discovered.last_updated),
            attachment: hardcoded.attachment,
            reasoning: hardcoded.reasoning,
            temperature: hardcoded.temperature,
            knowledge: hardcoded.knowledge.or(discovered.knowledge),
            tool_call: hardcoded.tool_call,
            structured_output: hardcoded.structured_output,
            open_weights: hardcoded.open_weights,
            cost: hardcoded.cost.or(discovered.cost),
            // Limits: hardcoded values are authoritative; use discovered values only as fallback
            limits: hardcoded.limits.or(discovered.limits),
            modalities: hardcoded.modalities.or(discovered.modalities),
            reasoning_effort: hardcoded.reasoning_effort.or(discovered.reasoning_effort),
            speed: hardcoded.speed.or(discovered.speed),
            verbosity: hardcoded.verbosity.or(discovered.verbosity),
            tool_search: hardcoded.tool_search,
            supported_parameters: if hardcoded.supported_parameters.is_empty() {
                discovered.supported_parameters
            } else {
                hardcoded.supported_parameters
            },
            supports_phases: hardcoded.supports_phases,
            supports_server_compaction: hardcoded.supports_server_compaction,
            decisions: hardcoded.decisions,
        }
    }
}

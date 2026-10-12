use super::*;

#[async_trait]
impl ProviderCredentialStore for DirectProviderCredentialStore {
    async fn get_decision_model(
        &self,
        model_id: Option<&str>,
        session_id: everruns_contracts::typed_id::SessionId,
    ) -> Result<Option<everruns_core::connection_services::DecisionModelBinding>> {
        self.provider_resolver
            .resolve_decision_model(self.org_id, model_id, session_id.uuid())
            .await
            .map_err(|_| store_error("Decision model is unavailable"))
    }

    async fn get_system_decision_model(
        &self,
        session_id: everruns_contracts::typed_id::SessionId,
    ) -> Result<everruns_core::connection_services::SystemDecisionModel> {
        self.provider_resolver
            .resolve_system_decision_model(self.org_id, Some(session_id.uuid()))
            .await
            .map_err(|_| store_error("Organization decision model is unavailable"))
    }

    async fn get_default_provider_credentials(
        &self,
        provider_type: &str,
    ) -> Result<Option<ProviderCredentials>> {
        Ok(self
            .provider_resolver
            .resolve_provider_credentials(self.org_id, provider_type)
            .await
            .map_err(|e| store_error(format!("Failed to resolve provider credentials: {e}")))?
            .map(|resolved| ProviderCredentials {
                api_key: resolved.api_key,
                base_url: resolved.base_url,
            }))
    }
}

use super::*;

#[async_trait]
impl ProviderCredentialStore for GrpcOrgAdapter {
    async fn get_decision_model(
        &self,
        model_id: Option<&str>,
        session_id: SessionId,
    ) -> Result<Option<crate::core::connection_services::DecisionModelBinding>> {
        let request = proto::GetDefaultProviderCredentialsRequest {
            org_id: self.org_id,
            provider_type: String::new(),
            provider_id: String::new(),
            session_id: Some(uuid_to_proto(session_id.uuid())),
            decision_model_id: Some(model_id.unwrap_or("").to_owned()),
        };
        let response = self
            .client
            .inner
            .client()
            .get_default_provider_credentials(request)
            .await
            .map_err(grpc_status_to_error)?
            .into_inner();
        if !response.found {
            return Ok(None);
        }
        // Older control planes cannot authorize service-specific bindings.
        serde_json::from_str(&response.decision_binding_json)
            .map(Some)
            .map_err(|_| AgentLoopError::store("Invalid decision model binding"))
    }

    async fn get_default_provider_credentials(
        &self,
        provider_type: &str,
    ) -> Result<Option<ProviderCredentials>> {
        self.client
            .get_default_provider_credentials(self.org_id, provider_type)
            .await
    }
}

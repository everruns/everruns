//! Whole-response Gemini transport using the shared state fold.
use super::*;

impl GeminiChatDriver {
    pub(super) async fn native_response(
        &self,
        endpoint: &everruns_contracts::ProviderEndpoint,
        config: &LlmCallConfig,
        request: &GeminiRequest,
        grounded: bool,
    ) -> Result<LlmResponseStream> {
        let url = endpoint
            .url(&format!("models/{}:generateContent", config.model))
            .ok_or_else(|| AgentLoopError::config("Gemini provider has no base URL"))?;
        let (response, retries) = self
            .send_generate_content_request(
                endpoint,
                request,
                &url,
                &config.model,
                &config.extra_headers,
            )
            .await?;
        let resource: GeminiStreamResponse = response.json().await.map_err(|error| {
            AgentLoopError::llm_kind(
                everruns_contracts::llm_error::LlmErrorKind::MalformedResponse,
                format!("Invalid Gemini JSON response: {error}"),
            )
        })?;
        if resource
            .candidates
            .as_ref()
            .is_none_or(|candidates| candidates.is_empty())
        {
            return Err(AgentLoopError::llm_kind(
                everruns_contracts::llm_error::LlmErrorKind::MalformedResponse,
                "Gemini JSON response has no candidates",
            ));
        }
        let mut state = GeminiStreamState {
            model: config.model.clone(),
            grounded,
            retry_metadata: retries.had_retries().then_some(retries),
            ..Default::default()
        };
        state.response(resource);
        state.finish();
        Ok(everruns_contracts::driver_helpers::attach_request_body(
            Box::pin(futures::stream::iter(state.pending.into_iter().map(Ok))),
            config
                .capture_request
                .then(|| serde_json::to_value(request).ok())
                .flatten(),
        ))
    }
}

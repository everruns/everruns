use super::*;
use crate::decision_driver::{DecisionDriver, DecisionDriverCapabilities, NativePrimitives};
use crate::decisions::{DecisionOutcome, DecisionQuestion, DecisionRequest};
use crate::driver_registry::{
    EmbedRequest, EmbedResponse, EmbeddingsDriver, EmbeddingsDriverError,
};
use std::sync::atomic::{AtomicUsize, Ordering};
struct Auth(Arc<AtomicUsize>);
#[async_trait]
impl ProviderAuth for Auth {
    async fn headers(&self, _: ProviderAuthRequest<'_>) -> Result<Vec<(String, String)>> {
        Ok(vec![(
            "authorization".into(),
            format!("Bearer {}", self.0.fetch_add(1, Ordering::SeqCst) + 1),
        )])
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
struct Wire;
async fn authenticate(endpoint: &ProviderEndpoint, service: &str) -> Result<()> {
    let resolved = endpoint
        .resolve("POST", endpoint.url(service).unwrap(), b"payload")
        .await?;
    assert_eq!(resolved.headers.len(), 1);
    assert!(resolved.headers[0].1.starts_with("Bearer "));
    Ok(())
}
#[async_trait]
impl ChatDriver for Wire {
    async fn chat_completion_stream(
        &self,
        endpoint: &ProviderEndpoint,
        _: Vec<crate::Message>,
        _: &crate::LlmCallConfig,
    ) -> Result<crate::LlmResponseStream> {
        authenticate(endpoint, "chat").await?;
        Ok(Box::pin(futures::stream::empty()))
    }
}
#[async_trait]
impl DecisionDriver for Wire {
    fn id(&self) -> &str {
        "wire"
    }
    fn capabilities(&self) -> DecisionDriverCapabilities {
        DecisionDriverCapabilities::new(NativePrimitives::ALL, true)
    }
    async fn evaluate(
        &self,
        endpoint: &ProviderEndpoint,
        request: DecisionRequest,
    ) -> Result<DecisionOutcome> {
        authenticate(endpoint, "systemone").await?;
        Ok(DecisionOutcome {
            model: request.model.unwrap(),
            ..Default::default()
        })
    }
}
#[async_trait]
impl EmbeddingsDriver for Wire {
    async fn embed(
        &self,
        endpoint: &ProviderEndpoint,
        _: EmbedRequest,
    ) -> std::result::Result<EmbedResponse, EmbeddingsDriverError> {
        authenticate(endpoint, "embeddings")
            .await
            .map_err(|e| EmbeddingsDriverError::Provider(e.to_string()))?;
        Err(EmbeddingsDriverError::Provider("fixture completed".into()))
    }
}
#[tokio::test]
async fn one_provider_authenticates_chat_decisions_and_embeddings_per_call() {
    let calls = Arc::new(AtomicUsize::new(0));
    let provider = Provider::new("account", Wire)
        .with_decisions(Wire)
        .with_embeddings(Wire)
        .auth(Auth(calls.clone()))
        .base_url("https://provider.example/v1");
    let _stream = provider
        .chat_completion_stream(
            vec![],
            &crate::LlmCallConfig {
                model: "chat".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let ask = DecisionRequest::new("state")
        .ask("q", DecisionQuestion::noul("True?"))
        .on(crate::ModelSpec::on("account", "vendor/model"));
    assert_eq!(
        provider.evaluate_decisions(ask).await.unwrap().model,
        "vendor/model"
    );
    let _ = provider
        .embed(EmbedRequest {
            model: "embedding".into(),
            texts: vec!["state".into()],
        })
        .await;
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    assert!(
        provider
            .evaluate_decisions(
                DecisionRequest::new("state").on(crate::ModelSpec::on("another", "model"))
            )
            .await
            .is_err()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 3);
}

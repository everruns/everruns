//! Browsing a provider's catalog through the facade alone: a downstream driver
//! answers `list_models`, and the selection converts straight back into a model
//! an agent runs.

use async_trait::async_trait;
use everruns::{
    Agent, AgentLoopError, ChatDriver, DiscoveredModel, DriverId, InMemoryEngine, LlmCallConfig,
    LlmMessage, LlmResponseStream, LlmStreamEvent, Provider, ProviderEndpoint, models,
};

#[derive(Clone)]
struct CatalogProtocol;

#[async_trait]
impl ChatDriver for CatalogProtocol {
    async fn chat_completion_stream(
        &self,
        _endpoint: &ProviderEndpoint,
        _messages: Vec<LlmMessage>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponseStream, AgentLoopError> {
        let model = config.model.clone();
        Ok(Box::pin(futures::stream::iter([
            Ok(LlmStreamEvent::TextDelta(format!("answered by {model}"))),
            Ok(LlmStreamEvent::Done(Box::default())),
        ])))
    }

    async fn list_models(
        &self,
        _endpoint: &ProviderEndpoint,
    ) -> Result<Option<Vec<DiscoveredModel>>, AgentLoopError> {
        Ok(Some(vec![
            DiscoveredModel {
                model_id: "house-model-v1".to_string(),
                display_name: Some("House Model".to_string()),
                created_at: None,
                owned_by: None,
                capabilities: vec!["chat".to_string()],
                discovered_profile: None,
            },
            DiscoveredModel {
                model_id: "gpt-5.6-terra".to_string(),
                display_name: None,
                created_at: None,
                owned_by: None,
                capabilities: vec!["chat".to_string()],
                discovered_profile: None,
            },
        ]))
    }
}

fn provider() -> Provider {
    // The runtime key is the application's; the driver kind is what profile
    // lookups resolve against.
    Provider::new("my-gateway", CatalogProtocol)
        .base_url("https://gateway.example/v1")
        .with_driver_id(DriverId::OpenAI)
}

#[tokio::test]
async fn a_catalog_is_ordered_and_carries_profile_metadata() {
    let catalog = models::list(provider()).await.unwrap();

    // Newest first where the provider dates its catalog, by id otherwise.
    let ids: Vec<&str> = catalog.iter().map(|model| model.id()).collect();
    assert_eq!(ids, vec!["gpt-5.6-terra", "house-model-v1"]);

    let known = &catalog[0];
    // The provider returned a bare id; the profile registry supplies display.
    assert!(known.display_name().is_some());
    assert!(known.description().is_some());
    assert!(known.context_window().is_some_and(|window| window > 0));
    assert!(known.supports_tools());

    let unknown = &catalog[1];
    // The provider's own name survives, and an unknown id simply has no profile.
    assert_eq!(unknown.display_name(), Some("House Model"));
    assert!(unknown.profile().is_none());
    assert!(unknown.context_window().is_none());
}

#[tokio::test]
async fn a_selected_model_runs_without_restating_the_provider() {
    let selected = models::list(provider()).await.unwrap()[0].model();
    assert_eq!(
        selected.profile().map(|profile| profile.family).as_deref(),
        Some("gpt-5.6-terra")
    );

    let agent = Agent::builder()
        .instructions("Reply through the selected model.")
        .model(selected)
        .build()
        .unwrap();
    let turn = InMemoryEngine::new()
        .create(agent)
        .run("hello")
        .await
        .unwrap();
    assert_eq!(turn.response, "answered by gpt-5.6-terra");
}

#[tokio::test]
async fn a_provider_without_a_catalog_says_so() {
    #[derive(Clone)]
    struct NoCatalog;

    #[async_trait]
    impl ChatDriver for NoCatalog {
        async fn chat_completion_stream(
            &self,
            _endpoint: &ProviderEndpoint,
            _messages: Vec<LlmMessage>,
            _config: &LlmCallConfig,
        ) -> Result<LlmResponseStream, AgentLoopError> {
            Ok(Box::pin(futures::stream::iter([Ok(LlmStreamEvent::Done(
                Box::default(),
            ))])))
        }
    }

    let error = models::list(Provider::new("bare", NoCatalog))
        .await
        .expect_err("no catalog");
    assert!(matches!(error, everruns::CatalogError::NoCatalog));
}

#[test]
fn browsing_needs_only_the_everruns_api() {
    let source = include_str!("model_catalog.rs");
    assert!(!source.contains(concat!("everruns_", "provider")));
    assert!(!source.contains(concat!("everruns_", "core")));
}

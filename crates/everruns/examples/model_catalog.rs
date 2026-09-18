//! Ask a provider which models it offers, then run the one that was picked.
//!
//! Offline (no API key, a stub driver stands in for a provider's models API):
//! ```text
//! cargo run -p everruns --example model_catalog
//! ```
//! OpenAI (requires OPENAI_API_KEY):
//! ```text
//! cargo run -p everruns --features openai --example model_catalog -- --live
//! ```

use async_trait::async_trait;
use everruns::llm::Message;
use everruns::{
    AgentLoopError, ChatDriver, DiscoveredModel, DriverId, LlmCallConfig, LlmResponseStream,
    LlmStreamEvent, Provider, ProviderEndpoint, models,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let live = std::env::args().nth(1).is_some_and(|arg| arg == "--live");

    let provider: Provider = if live {
        #[cfg(feature = "openai")]
        {
            println!("OpenAI's own catalog, over HTTP.\n");
            everruns::OpenAI::from_env()?.into()
        }
        #[cfg(not(feature = "openai"))]
        {
            return Err("Live mode requires: cargo run -p everruns --features openai --example model_catalog -- --live".into());
        }
    } else {
        println!("Offline: a stub driver answers the catalog call.\n");
        // A driver declares the models it serves; everything below is the same
        // whether that answer came from a stub or from OpenAI over HTTP.
        Provider::new("my-gateway", StubCatalog).with_driver_id(DriverId::OpenAI)
    };

    // One provider call. Ids come back exactly as chat calls expect them,
    // merged with the profile registry for names, descriptions, and limits the
    // provider's API does not report.
    let catalog = match models::list(provider).await {
        Ok(catalog) => catalog,
        // Not every provider can enumerate its models; that is not a failure.
        Err(models::CatalogError::NoCatalog) => {
            println!("This provider offers no catalog; keep your curated list.");
            return Ok(());
        }
        Err(error) => return Err(error.into()),
    };

    for model in catalog.iter().take(10) {
        let name = model.display_name().unwrap_or(model.id());
        print!("{:<28} {name}", model.id());
        if let Some(window) = model.context_window() {
            print!("  ({window} tokens)");
        }
        if model.supports_reasoning() {
            print!("  [reasoning]");
        }
        println!();
    }

    // A selection converts straight back into a model, bundled with the
    // provider it was discovered through: no string handling, nothing to
    // reconfigure.
    let Some(picked) = catalog.into_iter().find(|model| model.supports_tools()) else {
        println!("\nNo tool-calling model in this catalog.");
        return Ok(());
    };
    println!("\npicked {}", picked.id());
    if let Some(profile) = picked.profile()
        && let Some(cost) = profile.cost.as_ref()
    {
        println!(
            "input ${:.2} / output ${:.2} per million tokens",
            cost.input, cost.output
        );
    }

    if live {
        let answer = picked
            .model()
            .complete("Name the three primary colors.")
            .await?;
        println!("\n{answer}");
    }

    Ok(())
}

/// Stands in for a provider's models API so the example runs offline.
#[derive(Clone)]
struct StubCatalog;

#[async_trait]
impl ChatDriver for StubCatalog {
    async fn chat_completion_stream(
        &self,
        _endpoint: &ProviderEndpoint,
        _messages: Vec<Message>,
        _config: &LlmCallConfig,
    ) -> Result<LlmResponseStream, AgentLoopError> {
        Ok(Box::pin(futures::stream::iter([
            Ok(LlmStreamEvent::TextDelta("red, yellow, blue".to_string())),
            Ok(LlmStreamEvent::Done(Box::default())),
        ])))
    }

    async fn list_models(
        &self,
        _endpoint: &ProviderEndpoint,
    ) -> Result<Option<Vec<DiscoveredModel>>, AgentLoopError> {
        Ok(Some(
            ["gpt-5.6-terra", "gpt-5-mini", "house-model-v1"]
                .into_iter()
                .map(|model_id| DiscoveredModel {
                    model_id: model_id.to_string(),
                    display_name: None,
                    created_at: None,
                    owned_by: None,
                    capabilities: vec!["chat".to_string()],
                    discovered_profile: None,
                })
                .collect(),
        ))
    }
}

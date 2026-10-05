//! Build an authenticated provider from the enabled-vendor registry.
//!
//! Set OPENAI_API_KEY, then run:
//! cargo run -p everruns-drivers --features openai --example registry-chat -- <model-id> [prompt]

use everruns_contracts::{DriverId, LlmCallConfig, Message, MessageRole, ProviderConfig};
use everruns_drivers::{DriverRegistry, register_drivers};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let model = args
        .next()
        .ok_or("usage: registry-chat <model-id> [prompt]")?;
    let prompt = args
        .next()
        .unwrap_or_else(|| "Explain what an LLM driver does in one sentence.".into());

    let mut registry = DriverRegistry::new();
    register_drivers(&mut registry);
    eprintln!(
        "Available drivers: {:?}",
        registry.registered_provider_ids()
    );

    // The caller supplies credentials; the registry never reads the environment.
    let config =
        ProviderConfig::new(DriverId::OpenAI).with_api_key(std::env::var("OPENAI_API_KEY")?);
    let provider = registry.create_provider(&config)?;
    let response = provider
        .chat_completion(
            vec![Message::text(MessageRole::User, prompt)],
            &LlmCallConfig::new(model),
        )
        .await?;
    println!("{}", response.text);
    Ok(())
}

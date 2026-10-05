//! Collect a full reply using the Anthropic Messages driver.
//!
//! Set ANTHROPIC_API_KEY, then run:
//! cargo run -p everruns-drivers --features anthropic --example anthropic-chat -- <model-id> [prompt]

use everruns_contracts::{LlmCallConfig, Message, MessageRole};
use everruns_drivers::anthropic;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let model = args
        .next()
        .ok_or("usage: anthropic-chat <model-id> [prompt]")?;
    let prompt = args
        .next()
        .unwrap_or_else(|| "Explain what an LLM driver does in one sentence.".into());
    let provider = anthropic::from_env("anthropic")?;
    let config = LlmCallConfig::new(model);
    let response = provider
        .chat_completion(vec![Message::text(MessageRole::User, prompt)], &config)
        .await?;

    println!("{}", response.text);
    eprintln!("Token usage: {:?}", response.metadata.total_tokens);
    Ok(())
}

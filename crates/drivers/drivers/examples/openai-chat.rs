//! Stream a reply using the OpenAI Responses driver.
//!
//! Set OPENAI_API_KEY, then run:
//! cargo run -p everruns-drivers --features openai --example openai-chat -- <model-id> [prompt]
//! OPENAI_BASE_URL optionally selects a proxy or compatible endpoint.

use everruns_contracts::{LlmCallConfig, LlmStreamEvent, Message, MessageRole};
use everruns_drivers::openai;
use futures::StreamExt;
use std::io::{self, Write};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let model = args
        .next()
        .ok_or("usage: openai-chat <model-id> [prompt]")?;
    let prompt = args
        .next()
        .unwrap_or_else(|| "Explain what an LLM driver does in one sentence.".into());
    let provider = openai::from_env("openai")?;
    let config = LlmCallConfig::new(model);
    let mut stream = provider
        .chat_completion_stream(vec![Message::text(MessageRole::User, prompt)], &config)
        .await?;

    let mut stdout = io::stdout().lock();
    let mut completed = false;
    while let Some(event) = stream.next().await {
        match event? {
            LlmStreamEvent::TextDelta(delta) => {
                write!(stdout, "{delta}")?;
                stdout.flush()?;
            }
            LlmStreamEvent::Done(metadata) => {
                completed = true;
                eprintln!("\nToken usage: {:?}", metadata.total_tokens);
            }
            LlmStreamEvent::Error(error) => return Err(error.into()),
            _ => {}
        }
    }
    if !completed {
        return Err("stream ended without a completion event".into());
    }
    writeln!(stdout)?;
    Ok(())
}

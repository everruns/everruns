//! Call a model directly — no agent, no session, no history.
//!
//! Offline (no API key):
//! ```text
//! cargo run -p everruns --example direct_llm
//! ```
//! OpenAI (requires OPENAI_API_KEY and API credits):
//! ```text
//! cargo run -p everruns --features openai --example direct_llm -- --live
//! ```
//! An optional positional argument replaces the prompt.

use everruns::{LlmSimConfig, LlmStreamEvent, Model};
use futures::StreamExt;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1).peekable();
    let live = args.peek().is_some_and(|arg| arg == "--live");
    if live {
        args.next();
    }
    let prompt = args
        .next()
        .unwrap_or_else(|| "Name the three primary colors.".into());
    if args.next().is_some() || prompt.trim().is_empty() {
        return Err("Usage: direct_llm [--live] [PROMPT]".into());
    }

    let model = if live {
        #[cfg(feature = "openai")]
        {
            println!("OpenAI gpt-5.6-terra over HTTP.\n");
            Model::new("gpt-5.6-terra", everruns::OpenAI::from_env()?)
        }
        #[cfg(not(feature = "openai"))]
        {
            return Err("Live mode requires: cargo run -p everruns --features openai --example direct_llm -- --live".into());
        }
    } else {
        println!("Offline simulator: echoes the prompt; no model inference.\n");
        Model::simulated_with_config(LlmSimConfig::echo())
    };

    // One prompt, one answer. This is the whole API for the common case.
    println!("> {prompt}");
    println!("{}\n", model.complete(&prompt).await?);

    // The builder adds a system message and per-call controls, and returns the
    // full response rather than just its text.
    let response = model
        .completion()
        .system("Answer in one short sentence.")
        .user(&prompt)
        .max_tokens(128)
        .send()
        .await?;
    println!("with a system message: {}", response.text);
    if let Some(total) = response.metadata.total_tokens {
        println!("tokens: {total}");
    }
    println!();

    // The same call, observed as it arrives. Each event is a provider event;
    // the stream ends with Done and the call's metadata.
    print!("streamed: ");
    let mut stream = model.completion().user(&prompt).stream().await?;
    while let Some(event) = stream.next().await {
        match event? {
            LlmStreamEvent::TextDelta(delta) => print!("{delta}"),
            LlmStreamEvent::Done(metadata) => {
                println!();
                if let Some(reason) = metadata.finish_reason {
                    println!("finish reason: {reason}");
                }
            }
            _ => {}
        }
    }

    Ok(())
}

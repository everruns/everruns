//! Call a model directly — no agent, no session, no history.
//!
//! Requires OPENAI_API_KEY:
//! ```text
//! cargo run -p everruns --features openai --example direct_llm
//! ```
//! An optional positional argument replaces the prompt.

use everruns::{LlmStreamEvent, Model, OpenAI};
use futures::StreamExt;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let prompt = args
        .next()
        .unwrap_or_else(|| "Name the three primary colors.".into());
    if args.next().is_some() || prompt.trim().is_empty() {
        return Err("Usage: direct_llm [PROMPT]".into());
    }

    println!("OpenAI gpt-6.1-sol over HTTP.\n");
    let model = Model::new("gpt-6.1-sol", OpenAI::from_env()?);

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

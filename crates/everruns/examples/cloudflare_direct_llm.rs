//! Call a model directly through Cloudflare AI Gateway — no agent, no session,
//! no history.
//!
//! This is the LLM interface rather than the agent interface: a [`Model`] is a
//! model id plus the provider that reaches it, and `complete` is the whole API
//! for the common case. [`Model::completion`] adds a system message, per-call
//! controls, and streaming.
//!
//! Cloudflare reaches two kinds of model over one account, and the id says
//! which: `@cf/...` runs on Workers AI and draws on the account's own
//! allocation, while `vendor/model` is routed to that upstream vendor and
//! billed to the Cloudflare account.
//!
//! Set `CLOUDFLARE_API_TOKEN` and `CLOUDFLARE_ACCOUNT_ID` (and optionally
//! `CLOUDFLARE_AI_GATEWAY_ID` to pin a named gateway), then:
//!
//! ```text
//! cargo run -p everruns --example cloudflare_direct_llm
//! ```
//!
//! An optional positional argument replaces the model id:
//!
//! ```text
//! cargo run -p everruns --example cloudflare_direct_llm -- openai/gpt-6-luna
//! ```

use everruns::{LlmStreamEvent, Model};
use everruns_drivers::cloudflare;
use futures::StreamExt;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let model_id = args
        .next()
        .unwrap_or_else(|| "@cf/meta/llama-3.3-70b-instruct-fp8-fast".into());
    if args.next().is_some() {
        return Err("Usage: cloudflare_direct_llm [MODEL]".into());
    }

    // `from_env` reads the variables the driver declares — the API token, the
    // account id, and the optional gateway name. It is the standalone/CLI/dev
    // path; a server resolves credentials from storage instead. The endpoint is
    // derived from the account id, so there is no base URL to assemble here.
    let provider = cloudflare::from_env("cloudflare")?;

    // A model id bound to the provider that serves it.
    let model = Model::new(model_id.clone(), provider);
    println!("model: {model_id}\n");

    // One prompt, one answer.
    let prompt = "Name the three primary colors.";
    println!("> {prompt}");
    println!("{}\n", model.complete(prompt).await?);

    // The builder adds a system message and per-call controls, and returns the
    // full response rather than just its text.
    let response = model
        .completion()
        .system("Answer in one short sentence.")
        .user("Why is the sky blue?")
        .max_tokens(128)
        .send()
        .await?;
    println!("with a system message: {}", response.text);
    if let Some(total) = response.metadata.total_tokens {
        println!("tokens: {total}");
    }

    // Streaming yields semantic events, not raw chunks, so text deltas are
    // distinguishable from the rest of the stream.
    println!("\nstreaming:");
    let mut stream = model
        .completion()
        .user("Count from one to five.")
        .stream()
        .await?;
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

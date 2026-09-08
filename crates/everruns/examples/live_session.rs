//! Send a correction to a live Framework session, then inspect its transcript.
//!
//! Offline (no API key):
//! ```text
//! cargo run -p everruns --example live_session
//! ```
//! OpenAI (requires OPENAI_API_KEY and API credits):
//! ```text
//! cargo run -p everruns --features openai --example live_session -- --live
//! ```
//! Optional positional arguments replace the initial request and correction.
//! Framework steering queues input for the next iteration boundary; it does not
//! send OpenAI's WebSocket response.steer event.

use std::time::Duration;

use everruns::{Agent, Engine, LlmSimConfig, MessageRole, Model, SendDisposition};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1).peekable();
    let live = args.peek().is_some_and(|arg| arg == "--live");
    if live {
        args.next();
    }
    let prompt = args
        .next()
        .unwrap_or_else(|| "Plan a three-day trip from Paris to Amsterdam.".into());
    let update = args.next().unwrap_or_else(|| {
        "Prefer trains, keep the budget under EUR 500, and include a museum.".into()
    });
    if args.next().is_some() || prompt.trim().is_empty() || update.trim().is_empty() {
        return Err("Usage: live_session [--live] [INITIAL_REQUEST [CORRECTION]]".into());
    }

    let builder = Agent::builder()
        .name("steering-example")
        .instructions("Incorporate every user message. Keep the final answer within 150 words.");
    let agent = if live {
        #[cfg(feature = "openai")]
        {
            println!("OpenAI gpt-6-astra over HTTP; corrections apply at iteration boundaries.");
            builder
                .provider(everruns::OpenAI::from_env()?)
                .model("gpt-6-astra")
                .build()?
        }
        #[cfg(not(feature = "openai"))]
        {
            return Err("Live mode requires: cargo run -p everruns --features openai --example live_session -- --live".into());
        }
    } else {
        println!("Offline simulator: echoes the latest user input; no model inference.");
        builder
            .model(Model::simulated_with_config(
                LlmSimConfig::echo().with_response_delay(Duration::from_millis(100)),
            ))
            .build()?
    };
    let engine = Engine::new();
    let session = engine.create(agent);

    println!("\nInitial request: {prompt}");
    let initial = session.send(prompt.as_str()).await?;
    println!("Correction: {update}");
    // In a UI, call this same method when the user submits another message.
    // A receipt confirms acceptance, not that the model has applied the update.
    let latest = session.send(update.as_str()).await?;
    match latest.disposition {
        SendDisposition::Steered => println!("Accepted into the active turn."),
        SendDisposition::Started => {
            println!("The first turn finished; accepted as a follow-up turn.")
        }
        _ => {}
    }

    // Waiting on the latest receipt works even if the first turn finished just
    // before the correction arrived. Both receipts remain independently waitable.
    let result = latest.wait().await?;
    let first = initial.wait().await?;
    if !result.success || !first.success {
        return Err(format!(
            "Session failed: {}",
            result
                .error
                .or(first.error)
                .unwrap_or_else(|| format!("{:?}", result.stop_reason))
        )
        .into());
    }
    println!("\nAnswer: {}", result.response);
    println!("Iterations in the resulting turn: {}", result.iterations);

    let history = session.history().page().await?;
    println!("\nUser messages retained by the Framework:");
    for message in history
        .messages
        .iter()
        .filter(|message| message.role == MessageRole::User)
    {
        println!("- {}", message.text());
    }
    Ok(())
}

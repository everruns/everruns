//! Send a correction to a live Framework session, then inspect its transcript.
//!
//! Requires OPENAI_API_KEY:
//! ```text
//! cargo run -p everruns --features openai --example live_session
//! ```
//! Optional positional arguments replace the initial request and correction.
//! Framework steering queues input for the next iteration boundary; it does not
//! send OpenAI's WebSocket response.steer event.

use everruns::{Agent, Engine, MessageRole, OpenAI, SendDisposition};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let prompt = args
        .next()
        .unwrap_or_else(|| "Plan a three-day trip from Paris to Amsterdam.".into());
    let update = args.next().unwrap_or_else(|| {
        "Prefer trains, keep the budget under EUR 500, and include a museum.".into()
    });
    if args.next().is_some() || prompt.trim().is_empty() || update.trim().is_empty() {
        return Err("Usage: live_session [INITIAL_REQUEST [CORRECTION]]".into());
    }

    println!("OpenAI gpt-6.1-sol over HTTP; corrections apply at iteration boundaries.");
    let agent = Agent::builder()
        .name("steering-example")
        .instructions("Incorporate every user message. Keep the final answer within 150 words.")
        .provider(OpenAI::from_env()?)
        .model("gpt-6.1-sol")
        .build()?;
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

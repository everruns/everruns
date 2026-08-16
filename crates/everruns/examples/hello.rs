//! Smallest useful Everruns program: compose an agent, run a typed tool, inspect
//! the turn, and observe its event stream with `gpt-5.6-terra`.
//!
//! ```text
//! cargo run -p everruns --features openai --example hello
//! ```

use std::time::{SystemTime, UNIX_EPOCH};

use everruns::{Agent, Engine, OpenAI};

/// Return the current Unix time in seconds.
#[everruns::tool]
async fn current_time() -> Result<String, String> {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_secs();
    Ok(format!("{seconds} seconds since the Unix epoch"))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let agent = Agent::builder()
        .name("hello")
        .instructions("Use current_time to answer time questions. Be terse.")
        .provider(OpenAI::from_env()?)
        .model("gpt-5.6-terra")
        .tool(current_time())
        .build()?;

    let session = Engine::new().create(agent.clone());
    let mut events = session.events();
    let pending = session.send("what time is it?").await?;
    let mut event_types = Vec::new();
    while let Some(event) = events.recv().await? {
        event_types.push(event.event_type().to_string());
        if event.turn_id.as_deref() == Some(&pending.turn_id) && event.kind.is_terminal() {
            break;
        }
    }

    let turn = pending.wait().await?;
    println!("response: {}", turn.response);
    println!(
        "iterations: {}, tool calls: {}",
        turn.iterations, turn.tool_calls
    );

    println!("\nevents:");
    for (index, event_type) in event_types.into_iter().enumerate() {
        println!("  {:>2}. {event_type}", index + 1);
    }

    Ok(())
}

//! Concrete Engine ownership, isolated sessions, and engine-scoped resume.
//!
//! ```text
//! cargo run -p everruns --features openai --example engine_sessions
//! ```

use everruns::{Agent, Engine, OpenAI};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let engine = Engine::new();
    let agent = Agent::builder()
        .instructions("Be concise.")
        .provider(OpenAI::from_env()?)
        .model("gpt-6.1-sol")
        .build()?;

    let session = engine.create(agent);
    let session_id = session.session_id();
    println!("{}", session.send_and_wait("hello").await?.response);

    drop(session);
    let resumed = engine.resume(session_id).await?;
    println!(
        "{} messages",
        resumed.history().page().await?.messages.len()
    );
    Ok(())
}

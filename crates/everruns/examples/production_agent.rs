//! A production-shaped agent with a typed read-only tool, a defensive tool
//! boundary, multiple turns, JSONL persistence, and replay-based resume.
//! The agent uses `gpt-5.6-terra`; set `OPENAI_API_KEY` before running it.
//!
//! ```text
//! cargo run -p everruns --features openai,jsonl --example production_agent
//! ```

use std::sync::Arc;

use everruns::{Agent, JsonlSessionStore, OpenAI};

/// Look up an order in the read-only public order namespace.
#[everruns::tool]
async fn lookup_order(order_id: String) -> Result<String, String> {
    let Some(number) = order_id.strip_prefix("A-") else {
        return Err("order_id must use the public A-NNN format".into());
    };
    if number.is_empty() || !number.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("order_id must use the public A-NNN format".into());
    }

    Ok(format!("{order_id}: ready_to_ship"))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let agent = Agent::builder()
        .name("support-agent")
        .instructions("Answer from tool results; do not invent order status.")
        .model(OpenAI::from_env("gpt-5.6-terra")?)
        .tool(lookup_order())
        .build()?;

    let temp_dir = tempfile::tempdir()?;
    let log_path = temp_dir.path().join("support-session.jsonl");
    let store = Arc::new(JsonlSessionStore::open(&log_path).await?);
    let mut session = agent.session_with_store(store.clone());

    let first = session.run("What is the status of order A-42?").await?;
    println!("assistant: {}", first.response);

    let second = session.run("What can you do next?").await?;
    println!("assistant: {}", second.response);

    let session_id = session.id();
    drop(session);
    drop(store);

    let persisted_records = std::fs::read_to_string(&log_path)?.lines().count();
    println!("persisted {persisted_records} message records");

    // Opening a fresh store simulates a new host process. The next turn is
    // composed from the messages replayed out of the JSONL file.
    let reopened = Arc::new(JsonlSessionStore::open(&log_path).await?);
    let mut resumed = agent.resume_session(reopened, &session_id)?;
    let after_restart = resumed.run("Do you remember this order?").await?;
    println!("assistant after restart: {}", after_restart.response);

    Ok(())
}

//! A production-shaped agent with a typed read-only tool, a defensive tool
//! boundary, and multiple turns. The agent uses `gpt-5.6-terra`; set
//! `OPENAI_API_KEY` before running it.
//!
//! ```text
//! cargo run -p everruns --features openai --example production_agent
//! ```

use everruns::{Agent, InMemoryEngine, OpenAI};

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
        .provider(OpenAI::from_env()?)
        .model("gpt-5.6-terra")
        .tool(lookup_order())
        .build()?;

    let session = InMemoryEngine::new().create(agent.clone());
    let first = session
        .send_and_wait("What is the status of order A-42?")
        .await?;
    println!("assistant: {}", first.response);
    let second = session.send_and_wait("What can you do next?").await?;
    println!("assistant: {}", second.response);
    Ok(())
}

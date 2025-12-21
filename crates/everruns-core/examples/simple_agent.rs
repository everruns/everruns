//! Simple Agent Example - Minimal agent loop without tools
//!
//! This is the simplest possible example of using everruns-core with OpenAI.
//! No tools, just a basic conversation.
//!
//! Prerequisites:
//! - Set OPENAI_API_KEY environment variable
//!
//! Run with: cargo run -p everruns-core --example simple_agent --features openai

use everruns_core::{
    config::AgentConfig,
    memory::{InMemoryEventEmitter, InMemoryMessageStore},
    message::Message,
    openai::OpenAIProtocolLlmProvider,
    tools::ToolRegistry,
    AgentLoop,
};
use uuid::Uuid;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Check API key
    if std::env::var("OPENAI_API_KEY").is_err() {
        eprintln!("Error: OPENAI_API_KEY not set");
        eprintln!("  export OPENAI_API_KEY=your-key");
        std::process::exit(1);
    }

    println!("=== Simple Agent (everruns-core) ===\n");

    // 1. Create LLM provider from environment
    let llm = OpenAIProtocolLlmProvider::from_env()?;

    // 2. Create agent config (no tools)
    let config = AgentConfig::new("You are a helpful assistant. Be concise.", "gpt-4o-mini");

    // 3. Create in-memory stores
    let events = InMemoryEventEmitter::new();
    let messages = InMemoryMessageStore::new();
    let tools = ToolRegistry::new(); // No tools

    // 4. Seed with user message
    let session_id = Uuid::now_v7();
    let user_input = "What is Rust programming language in one sentence?";

    messages
        .seed(session_id, vec![Message::user(user_input)])
        .await;

    println!("User: {}\n", user_input);

    // 5. Create and run agent loop
    let agent = AgentLoop::new(config, events, messages, llm, tools);
    let result = agent.run(session_id).await?;

    // 6. Print result
    println!("Assistant: {}", result.final_response.unwrap_or_default());
    println!("\n(iterations: {})", result.iterations);

    Ok(())
}

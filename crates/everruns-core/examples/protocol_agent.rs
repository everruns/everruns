//! Simple Protocol Agent Example
//!
//! This example demonstrates using AgentProtocol for a simple agent
//! that answers questions without tool calls.
//!
//! Prerequisites:
//! - Set OPENAI_API_KEY environment variable
//!
//! Run with: cargo run -p everruns-core --example protocol_agent --features openai

use everruns_core::{
    config::AgentConfig, memory::InMemoryMessageStore, openai::OpenAIProtocolLlmProvider,
    protocol::AgentProtocol, tools::ToolRegistry,
};
use uuid::Uuid;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .init();

    if std::env::var("OPENAI_API_KEY").is_err() {
        eprintln!("Error: OPENAI_API_KEY environment variable is not set");
        eprintln!("  export OPENAI_API_KEY=your-api-key");
        std::process::exit(1);
    }

    println!("=== Simple Agent Demo ===\n");

    // Create protocol components
    let message_store = InMemoryMessageStore::new();
    let llm_provider = OpenAIProtocolLlmProvider::from_env()?;
    let tools = ToolRegistry::new();

    // Create protocol
    let protocol = AgentProtocol::new(message_store, llm_provider, tools);

    // Config
    let config = AgentConfig::new("You are a helpful assistant. Be concise.", "gpt-4o-mini");

    // Run a conversation turn
    let session_id = Uuid::now_v7();
    let question = "What is the capital of France?";

    println!("User: {}", question);

    let response = protocol.run_turn(session_id, question, &config, 5).await?;

    println!("Assistant: {}\n", response);

    println!("=== Demo completed! ===");
    Ok(())
}

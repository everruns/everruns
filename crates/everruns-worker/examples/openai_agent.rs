//! OpenAI Agent Example - Agent Loop with Real OpenAI API
//!
//! This example demonstrates the agent loop using the actual OpenAI API,
//! showing real integration with production components.
//!
//! Prerequisites:
//! - Set OPENAI_API_KEY environment variable
//!
//! Run with: cargo run --example openai_agent -p everruns-worker

use everruns_core::{
    config::AgentConfig,
    memory::{InMemoryMessageStore, NoOpEventEmitter},
    traits::ToolExecutor,
    AgentLoop, Message, Result, ToolCall, ToolDefinition, ToolResult,
};
use everruns_worker::OpenAiProvider;
use uuid::Uuid;

/// Simple no-op tool executor for this demo
struct DemoToolExecutor;

#[async_trait::async_trait]
impl ToolExecutor for DemoToolExecutor {
    async fn execute(
        &self,
        tool_call: &ToolCall,
        _tool_def: &ToolDefinition,
    ) -> Result<ToolResult> {
        // For demo, return a mock result
        println!(
            "  [TOOL] Executing: {} with args: {}",
            tool_call.name, tool_call.arguments
        );

        Ok(ToolResult {
            tool_call_id: tool_call.id.clone(),
            result: Some(serde_json::json!({
                "status": "success",
                "data": "Demo tool result"
            })),
            error: None,
        })
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Set up logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    // Check for API key
    if std::env::var("OPENAI_API_KEY").is_err() {
        eprintln!("Error: OPENAI_API_KEY environment variable is not set");
        eprintln!("Please set it before running this example:");
        eprintln!("  export OPENAI_API_KEY=your-api-key");
        std::process::exit(1);
    }

    println!("=== Agent Loop OpenAI Example ===");
    println!("(Using real OpenAI API)\n");

    // Create agent configuration
    let config = AgentConfig::new(
        "You are a helpful assistant. Keep your responses concise and friendly.",
        "gpt-5.2",
    )
    .with_max_iterations(3);

    // Create components
    let event_emitter = NoOpEventEmitter;
    let message_store = InMemoryMessageStore::new();
    let llm_provider = OpenAiProvider::new()?;
    let tool_executor = DemoToolExecutor;

    // Seed with a user message
    let session_id = Uuid::now_v7();
    message_store
        .seed(
            session_id,
            vec![Message::user(
                "Hello! Can you tell me a short joke about programming?",
            )],
        )
        .await;

    println!("Session ID: {}", session_id);
    println!("User: Hello! Can you tell me a short joke about programming?\n");
    println!("Waiting for response...\n");

    // Build and run the agent loop
    let agent_loop = AgentLoop::new(
        config,
        event_emitter,
        message_store,
        llm_provider,
        tool_executor,
    );

    let result = agent_loop.run(session_id).await?;

    // Print results
    println!("--- Response ---");
    println!("{}", result.final_response.unwrap_or_default());
    println!("\n--- Stats ---");
    println!("Iterations: {}", result.iterations);
    println!("Total messages: {}", result.messages.len());

    println!("\n=== Example completed! ===");
    Ok(())
}

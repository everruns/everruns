//! Simple Agent Loop Example
//!
//! This example demonstrates how to use the everruns-agent-loop crate
//! with in-memory implementations for a standalone agent execution.
//!
//! Run with: cargo run --example simple_loop -p everruns-agent-loop

use everruns_agent_loop::{
    config::AgentConfig,
    memory::{InMemoryAgentLoopBuilder, InMemoryMessageStore, MockLlmProvider, MockLlmResponse},
    message::ConversationMessage,
    ToolCall,
};
use uuid::Uuid;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Set up logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    println!("=== Simple Agent Loop Example ===\n");

    // Example 1: Basic conversation
    basic_conversation().await?;

    // Example 2: Conversation with tool calls
    conversation_with_tools().await?;

    println!("\n=== All examples completed! ===");
    Ok(())
}

/// Example 1: Basic conversation without tools
async fn basic_conversation() -> anyhow::Result<()> {
    println!("--- Example 1: Basic Conversation ---\n");

    // Create agent configuration
    let config = AgentConfig::new(
        "You are a helpful assistant that provides concise answers.",
        "gpt-5.2",
    );

    // Create mock LLM provider with predefined response
    let llm_provider = MockLlmProvider::new();
    llm_provider
        .add_response(MockLlmResponse::text(
            "Hello! I'm your AI assistant. How can I help you today?",
        ))
        .await;

    // Create message store and seed with user message
    let message_store = InMemoryMessageStore::new();
    let session_id = Uuid::now_v7();
    message_store
        .seed(
            session_id,
            vec![ConversationMessage::user("Hello, who are you?")],
        )
        .await;

    // Build the agent loop
    let (agent_loop, event_emitter, _, _, _) = InMemoryAgentLoopBuilder::new(config)
        .llm_provider(llm_provider)
        .message_store(message_store)
        .build_with_refs();

    // Run the loop
    let result = agent_loop.run(session_id).await?;

    // Print results
    println!("Session ID: {}", result.session_id);
    println!("Iterations: {}", result.iterations);
    println!(
        "Final response: {}",
        result.final_response.unwrap_or_default()
    );
    println!("Total messages: {}", result.messages.len());
    println!("Events emitted: {}", event_emitter.count().await);

    println!();
    Ok(())
}

/// Example 2: Conversation with tool calls
async fn conversation_with_tools() -> anyhow::Result<()> {
    println!("--- Example 2: Conversation with Tools ---\n");

    // Create agent configuration with a tool
    let config = AgentConfig::new(
        "You are a helpful assistant with access to a weather tool.",
        "gpt-5.2",
    )
    .with_tools(vec![everruns_agent_loop::ToolDefinition::Builtin(
        everruns_contracts::tools::BuiltinTool {
            name: "get_weather".to_string(),
            description: "Get the current weather for a city".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "city": {
                        "type": "string",
                        "description": "The city name"
                    }
                },
                "required": ["city"]
            }),
            kind: everruns_contracts::tools::BuiltinToolKind::HttpGet,
            policy: everruns_contracts::tools::ToolPolicy::Auto,
        },
    )])
    .with_max_iterations(5);

    // Create mock LLM provider with tool call response
    let llm_provider = MockLlmProvider::new();

    // First response: LLM requests tool call
    llm_provider
        .add_response(MockLlmResponse::with_tools(
            "Let me check the weather for you.",
            vec![ToolCall {
                id: "call_123".to_string(),
                name: "get_weather".to_string(),
                arguments: serde_json::json!({"city": "New York"}),
            }],
        ))
        .await;

    // Second response: LLM uses tool result
    llm_provider
        .add_response(MockLlmResponse::text(
            "The current weather in New York is 72°F and sunny. Perfect day to go outside!",
        ))
        .await;

    // Create message store and seed with user message
    let message_store = InMemoryMessageStore::new();
    let session_id = Uuid::now_v7();
    message_store
        .seed(
            session_id,
            vec![ConversationMessage::user("What's the weather in New York?")],
        )
        .await;

    // Build the agent loop with mock tool executor
    let (agent_loop, event_emitter, _, _, tool_executor) = InMemoryAgentLoopBuilder::new(config)
        .llm_provider(llm_provider)
        .message_store(message_store)
        .build_with_refs();

    // Set up mock tool result
    tool_executor
        .set_result(
            "get_weather",
            serde_json::json!({
                "city": "New York",
                "temperature": 72,
                "condition": "sunny"
            }),
        )
        .await;

    // Run the loop
    let result = agent_loop.run(session_id).await?;

    // Print results
    println!("Session ID: {}", result.session_id);
    println!("Iterations: {}", result.iterations);
    println!(
        "Final response: {}",
        result.final_response.unwrap_or_default()
    );
    println!("Total messages: {}", result.messages.len());
    println!("Events emitted: {}", event_emitter.count().await);

    // Print tool calls made
    let calls = tool_executor.calls().await;
    println!("Tool calls made: {}", calls.len());
    for call in &calls {
        println!("  - {} with args: {}", call.name, call.arguments);
    }

    // Print conversation
    println!("\nConversation:");
    for msg in &result.messages {
        println!("  [{}] {}", msg.role, msg.content.to_llm_string());
    }

    println!();
    Ok(())
}

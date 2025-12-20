//! Protocol Agent Example - Stateless Atomic Operations
//!
//! This example demonstrates using AgentProtocol for stateless agent execution.
//! AgentProtocol provides atomic operations (Atoms) that are self-contained:
//! - Each atom loads its own state, executes, and stores results
//! - No internal state in the protocol - all state in the message store
//! - Perfect for Temporal workflows, custom orchestration, pause/resume
//!
//! Key Atoms:
//! - add_user_message: Add a user message
//! - call_model: Call the LLM (loads messages, calls, stores response)
//! - execute_tools: Execute tool calls (stores results)
//! - determine_next_action: Decide what to do next
//! - run_turn: High-level convenience method
//!
//! Prerequisites:
//! - Set OPENAI_API_KEY environment variable
//!
//! Run with: cargo run -p everruns-core --example protocol_agent --features openai

use async_trait::async_trait;
use everruns_core::{
    config::AgentConfig,
    memory::InMemoryMessageStore,
    openai::OpenAIProtocolLlmProvider,
    protocol::{AgentProtocol, NextAction},
    tools::{Tool, ToolExecutionResult, ToolRegistry},
};
use serde_json::{json, Value};
use uuid::Uuid;

// ============================================================================
// Custom Tool: Get Current Time
// ============================================================================

struct GetCurrentTime;

#[async_trait]
impl Tool for GetCurrentTime {
    fn name(&self) -> &str {
        "get_current_time"
    }

    fn description(&self) -> &str {
        "Get the current date and time."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        let now = chrono::Utc::now();
        ToolExecutionResult::success(json!({
            "datetime": now.format("%A, %B %d, %Y at %H:%M:%S UTC").to_string()
        }))
    }
}

// ============================================================================
// Main
// ============================================================================

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

    println!("=== Protocol Agent Demo (everruns-core) ===\n");

    // Example 1: Simple query using run_turn (high-level)
    example_run_turn().await?;

    // Example 2: Manual orchestration with atoms
    example_manual_atoms().await?;

    // Example 3: Tool calling with atoms
    example_tool_calling().await?;

    println!("=== Demo completed! ===");
    Ok(())
}

/// Example 1: Using run_turn for simple execution
async fn example_run_turn() -> anyhow::Result<()> {
    println!("--- Example 1: run_turn (High-Level) ---\n");

    // Create protocol components
    let message_store = InMemoryMessageStore::new();
    let llm_provider = OpenAIProtocolLlmProvider::from_env()?;
    let tools = ToolRegistry::new();

    // Create protocol
    let protocol = AgentProtocol::new(message_store, llm_provider, tools);

    // Config
    let config = AgentConfig::new("You are helpful. Be concise.", "gpt-4o-mini");

    // Run a turn
    let session_id = Uuid::now_v7();
    let response = protocol
        .run_turn(session_id, "What is 2 + 2?", &config, 5)
        .await?;

    println!("  User: What is 2 + 2?");
    println!("  Assistant: {}\n", response);

    Ok(())
}

/// Example 2: Manual orchestration using individual atoms
async fn example_manual_atoms() -> anyhow::Result<()> {
    println!("--- Example 2: Manual Atom Orchestration ---\n");

    // Create protocol
    let message_store = InMemoryMessageStore::new();
    let llm_provider = OpenAIProtocolLlmProvider::from_env()?;
    let tools = ToolRegistry::new();
    let protocol = AgentProtocol::new(message_store, llm_provider, tools);

    let config = AgentConfig::new("You are helpful. Be concise.", "gpt-4o-mini");
    let session_id = Uuid::now_v7();

    // Step 1: Add user message
    println!("  Step 1: Adding user message...");
    let user_msg = protocol
        .add_user_message(session_id, "What is the capital of France?")
        .await?;
    println!("    Added: {:?}", user_msg.content);

    // Step 2: Check what to do next
    println!("  Step 2: Determining next action...");
    let action = protocol.determine_next_action(session_id).await?;
    println!("    Next action: {:?}", action);

    // Step 3: Call model (since action is CallModel)
    println!("  Step 3: Calling model...");
    let result = protocol.call_model(session_id, &config).await?;
    println!("    Response: {}", result.text);
    println!("    Needs tools: {}", result.needs_tool_execution);

    // Step 4: Check next action (should be Complete)
    println!("  Step 4: Determining next action...");
    let action = protocol.determine_next_action(session_id).await?;
    println!("    Next action: {:?}", action);

    // Step 5: Load all messages to verify
    println!("  Step 5: Loading all messages...");
    let loaded = protocol.load_messages(session_id).await?;
    println!("    Total messages: {}\n", loaded.count);

    Ok(())
}

/// Example 3: Tool calling with atoms
async fn example_tool_calling() -> anyhow::Result<()> {
    println!("--- Example 3: Tool Calling with Atoms ---\n");

    // Create protocol with tools
    let message_store = InMemoryMessageStore::new();
    let llm_provider = OpenAIProtocolLlmProvider::from_env()?;
    let tools = ToolRegistry::builder().tool(GetCurrentTime).build();

    // Get tool definitions before moving tools into protocol
    let tool_definitions = tools.tool_definitions();

    let protocol = AgentProtocol::new(message_store, llm_provider, tools);

    let config = AgentConfig::new(
        "You are helpful. Use the get_current_time tool when asked about time.",
        "gpt-4o-mini",
    )
    .with_tools(tool_definitions)
    .with_max_iterations(5);

    let session_id = Uuid::now_v7();

    // Add user message
    println!("  Adding user message: \"What time is it?\"");
    protocol
        .add_user_message(session_id, "What time is it?")
        .await?;

    // Manual loop using atoms
    let mut iteration = 0;
    let max_iterations = 5;

    loop {
        iteration += 1;
        if iteration > max_iterations {
            println!("  Max iterations reached!");
            break;
        }

        println!("\n  --- Iteration {} ---", iteration);

        // Determine next action
        let action = protocol.determine_next_action(session_id).await?;
        println!("  Next action: {:?}", format_action(&action));

        match action {
            NextAction::CallModel => {
                let result = protocol.call_model(session_id, &config).await?;
                if !result.text.is_empty() {
                    println!("  Model response: {}", truncate(&result.text, 60));
                }
                if result.needs_tool_execution {
                    println!(
                        "  Tool calls: {:?}",
                        result
                            .tool_calls
                            .iter()
                            .map(|tc| &tc.name)
                            .collect::<Vec<_>>()
                    );
                }
            }
            NextAction::ExecuteTools { tool_calls } => {
                println!("  Executing {} tool(s)...", tool_calls.len());
                let result = protocol
                    .execute_tools(session_id, &tool_calls, &config.tools)
                    .await?;
                for (tc, res) in tool_calls.iter().zip(result.results.iter()) {
                    if let Some(ref val) = res.result {
                        println!("    {} -> {}", tc.name, val);
                    }
                }
            }
            NextAction::Complete { final_response } => {
                if let Some(resp) = final_response {
                    println!("\n  Final response: {}", resp);
                }
                break;
            }
            NextAction::Error { message } => {
                println!("  Error: {}", message);
                break;
            }
        }
    }

    println!();
    Ok(())
}

fn format_action(action: &NextAction) -> String {
    match action {
        NextAction::CallModel => "CallModel".to_string(),
        NextAction::ExecuteTools { tool_calls } => {
            format!("ExecuteTools({} calls)", tool_calls.len())
        }
        NextAction::Complete { .. } => "Complete".to_string(),
        NextAction::Error { message } => format!("Error({})", message),
    }
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len])
    }
}

//! Protocol Agent Example - Stateless Atomic Operations
//!
//! This example demonstrates using AgentProtocol for stateless agent execution.
//! AgentProtocol provides atomic operations (Atoms) that are self-contained:
//! - Each atom loads its own state, executes, and stores results
//! - No internal state in the protocol - all state in the message store
//! - Perfect for Temporal workflows, custom orchestration, pause/resume
//!
//! Key concepts:
//! - Atom trait: Defines atomic operations with Input → Output
//! - CallModelAtom: Atom for calling the LLM
//! - ExecuteToolsAtom: Atom for executing tools
//! - AgentProtocol: Convenience wrapper that uses atoms internally
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
    protocol::{AgentProtocol, Atom, CallModelAtom, CallModelInput, NextAction},
    tools::{Tool, ToolExecutionResult, ToolRegistry},
    MessageStore,
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

    // Example 2: Using Atom trait directly
    example_atom_trait().await?;

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

/// Example 2: Using the Atom trait directly
async fn example_atom_trait() -> anyhow::Result<()> {
    println!("--- Example 2: Using Atom Trait Directly ---\n");

    // Create components
    let message_store = InMemoryMessageStore::new();
    let llm_provider = OpenAIProtocolLlmProvider::from_env()?;

    let config = AgentConfig::new("You are helpful. Be concise.", "gpt-4o-mini");
    let session_id = Uuid::now_v7();

    // Add user message first
    let user_msg = everruns_core::ConversationMessage::user("What is the capital of France?");
    message_store.seed(session_id, vec![user_msg.clone()]).await;
    println!("  Added user message: {:?}", user_msg.content);

    // Create CallModelAtom directly
    let call_model_atom = CallModelAtom::new(message_store.clone(), llm_provider);

    // Execute the atom using the Atom trait
    println!("  Executing CallModelAtom...");
    println!("    Atom name: {}", call_model_atom.name());

    let result = call_model_atom
        .execute(CallModelInput {
            session_id,
            config: config.clone(),
        })
        .await?;

    println!("    Response: {}", result.text);
    println!("    Tool calls: {}", result.tool_calls.len());
    println!("    Needs tool execution: {}", result.needs_tool_execution);

    // Verify message was stored
    let messages = message_store.load(session_id).await?;
    println!("    Messages in store: {}\n", messages.len());

    Ok(())
}

/// Example 3: Tool calling with atoms via AgentProtocol
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

    // Manual loop using atoms (via protocol convenience methods)
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
                // Uses CallModelAtom internally
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
                // Uses ExecuteToolsAtom internally
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

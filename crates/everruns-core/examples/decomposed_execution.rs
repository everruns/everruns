//! Decomposed Execution Example - Step-by-Step Agent Loop
//!
//! This example demonstrates decomposed (step-by-step) execution of the agent loop.
//! Instead of running the entire loop at once, each step is executed independently.
//!
//! This pattern is useful for:
//! - Temporal workflow activities (each step can be a separate activity)
//! - Fine-grained control over execution flow
//! - Custom state persistence between steps
//! - Implementing pause/resume functionality
//!
//! The execution flow:
//! 1. Create StepInput with initial messages
//! 2. Execute step with execute_step()
//! 3. Check if loop should continue
//! 4. If tool calls are pending, execute them
//! 5. Repeat until complete
//!
//! Prerequisites:
//! - Set OPENAI_API_KEY environment variable
//!
//! Run with: cargo run -p everruns-core --example decomposed_execution --features openai

use async_trait::async_trait;
use everruns_core::{
    config::AgentConfig,
    memory::{InMemoryEventEmitter, InMemoryMessageStore},
    message::{ConversationMessage, MessageContent, MessageRole},
    openai::OpenAIProtocolLlmProvider,
    step::{StepInput, StepKind, StepResult},
    tools::{Tool, ToolExecutionResult, ToolRegistry},
    AgentLoop,
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
        "Get the current date and time. Use when asked about the current time."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "format": {
                    "type": "string",
                    "description": "Output format: 'human' for readable, 'iso8601' for ISO format",
                    "enum": ["human", "iso8601"]
                }
            },
            "additionalProperties": false
        })
    }

    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        let format = arguments
            .get("format")
            .and_then(|v| v.as_str())
            .unwrap_or("human");

        let now = chrono::Utc::now();

        let result = match format {
            "iso8601" => json!({
                "datetime": now.to_rfc3339(),
                "format": "iso8601"
            }),
            _ => json!({
                "datetime": now.format("%A, %B %d, %Y at %H:%M:%S UTC").to_string(),
                "format": "human"
            }),
        };

        ToolExecutionResult::success(result)
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

fn print_step_info(step_num: usize, kind: &StepKind, result: &StepResult) {
    print!("  Step {}: {:?} -> ", step_num, kind);
    match result {
        StepResult::LlmCallComplete {
            response_text,
            tool_calls,
            continue_loop,
        } => {
            if tool_calls.is_empty() {
                println!("Response: \"{}\"", truncate(response_text, 60));
            } else {
                println!(
                    "{} tool call(s), continue={}",
                    tool_calls.len(),
                    continue_loop
                );
                for tc in tool_calls {
                    println!("       -> {}({})", tc.name, tc.arguments);
                }
            }
        }
        StepResult::ToolExecutionComplete { results } => {
            println!("{} result(s)", results.len());
            for result in results {
                if let Some(ref val) = result.result {
                    println!("       <- {}", val);
                }
            }
        }
        StepResult::SetupComplete { message_count } => {
            println!("{} messages loaded", message_count);
        }
        StepResult::FinalizeComplete { final_response } => {
            println!(
                "Final: \"{}\"",
                truncate(final_response.as_deref().unwrap_or("(none)"), 60)
            );
        }
    }
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len])
    }
}

fn print_messages(messages: &[ConversationMessage]) {
    println!("\n  Final conversation:");
    for (i, msg) in messages.iter().enumerate() {
        match msg.role {
            MessageRole::User => {
                println!("    {}. [User] {}", i + 1, msg.content.to_llm_string());
            }
            MessageRole::Assistant => {
                let text = msg.content.to_llm_string();
                if let Some(ref tool_calls) = msg.tool_calls {
                    if !tool_calls.is_empty() {
                        println!(
                            "    {}. [Assistant] Calling {} tool(s)",
                            i + 1,
                            tool_calls.len()
                        );
                    } else if !text.is_empty() {
                        println!("    {}. [Assistant] {}", i + 1, truncate(&text, 50));
                    }
                } else if !text.is_empty() {
                    println!("    {}. [Assistant] {}", i + 1, truncate(&text, 50));
                }
            }
            MessageRole::ToolResult => {
                if let MessageContent::ToolResult { result, error } = &msg.content {
                    if let Some(err) = error {
                        println!("    {}. [Tool] Error: {}", i + 1, err);
                    } else if let Some(res) = result {
                        println!("    {}. [Tool] {}", i + 1, truncate(&res.to_string(), 50));
                    }
                }
            }
            _ => {}
        }
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

    println!("=== Decomposed Execution Demo (everruns-core) ===\n");

    // Example 1: Simple query (no tools)
    example_simple_query().await?;

    // Example 2: Query with tool calls
    example_with_tool_calls().await?;

    println!("=== Demo completed! ===");
    Ok(())
}

/// Example 1: Simple query without tools - single step execution
async fn example_simple_query() -> anyhow::Result<()> {
    println!("--- Example 1: Simple Query (No Tools) ---\n");
    println!("  This demonstrates a single LLM call without tool usage.\n");

    // Create components
    let event_emitter = InMemoryEventEmitter::new();
    let message_store = InMemoryMessageStore::new();
    let llm_provider = OpenAIProtocolLlmProvider::from_env()?;
    let tools = ToolRegistry::new();

    let config = AgentConfig::new("You are a helpful assistant. Be concise.", "gpt-4o-mini");

    let agent = AgentLoop::new(config, event_emitter, message_store, llm_provider, tools);

    // Create initial input
    let session_id = Uuid::now_v7();
    let user_message = "What is the capital of France? One word answer.";
    println!("  User: {}\n", user_message);

    let initial_messages = vec![ConversationMessage::user(user_message)];

    // Create step input
    let mut input = StepInput::new(session_id, initial_messages);
    input.iteration = 1;

    // Execute the step
    println!("  Executing steps:");
    let mut step_count = 0;

    loop {
        step_count += 1;
        let output = agent.execute_step(input.clone()).await?;

        // Print step info
        if let Some(ref result) = output.step.result {
            print_step_info(step_count, &output.step.kind, result);
        }

        if !output.continue_loop {
            // Loop complete
            print_messages(&output.messages);
            break;
        }

        // Prepare next iteration
        input = StepInput {
            session_id,
            iteration: input.iteration + 1,
            messages: output.messages,
            pending_tool_calls: output.pending_tool_calls,
        };
    }

    println!("\n  Total steps: {}\n", step_count);
    Ok(())
}

/// Example 2: Query with tool calls - multiple step execution
async fn example_with_tool_calls() -> anyhow::Result<()> {
    println!("--- Example 2: Query with Tool Calls ---\n");
    println!("  This demonstrates LLM call -> tool execution -> LLM response flow.\n");

    // Create tool registry
    let registry = ToolRegistry::builder().tool(GetCurrentTime).build();

    // Create components
    let event_emitter = InMemoryEventEmitter::new();
    let message_store = InMemoryMessageStore::new();
    let llm_provider = OpenAIProtocolLlmProvider::from_env()?;

    let config = AgentConfig::new(
        "You are a helpful assistant. When asked about time, use the get_current_time tool.",
        "gpt-4o-mini",
    )
    .with_tools(registry.tool_definitions())
    .with_max_iterations(5);

    let agent = AgentLoop::new(config, event_emitter, message_store, llm_provider, registry);

    // Create initial input
    let session_id = Uuid::now_v7();
    let user_message = "What time is it right now?";
    println!("  User: {}\n", user_message);

    let initial_messages = vec![ConversationMessage::user(user_message)];

    // Create step input
    let mut input = StepInput::new(session_id, initial_messages);
    input.iteration = 1;

    // Execute steps until complete
    println!("  Executing steps:");
    let mut step_count = 0;
    let max_steps = 10;

    loop {
        if step_count >= max_steps {
            println!("  Warning: Max steps reached!");
            break;
        }

        step_count += 1;
        let output = agent.execute_step(input.clone()).await?;

        // Print step info
        if let Some(ref result) = output.step.result {
            print_step_info(step_count, &output.step.kind, result);
        }

        if !output.continue_loop {
            // Loop complete
            print_messages(&output.messages);
            break;
        }

        // Prepare next step
        // If we have pending tool calls, execute them in the next step
        // Otherwise, continue with LLM call
        input = StepInput {
            session_id,
            iteration: if output.pending_tool_calls.is_empty() {
                input.iteration + 1
            } else {
                input.iteration
            },
            messages: output.messages,
            pending_tool_calls: output.pending_tool_calls,
        };
    }

    println!("\n  Total steps: {}", step_count);
    println!("  (Step 1: LLM decides to call tool)");
    println!("  (Step 2: Tool execution)");
    println!("  (Step 3: LLM generates final response)\n");

    Ok(())
}

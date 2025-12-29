//! Capabilities Example - Agent Loop with Capability System
//!
//! This example demonstrates how to use the capabilities system to compose
//! agent functionality through modular units. Capabilities can contribute:
//! - System prompt additions
//! - Tools for the agent
//!
//! The example shows:
//! 1. Using built-in capabilities (CurrentTime)
//! 2. Creating custom capabilities
//! 3. Applying capabilities to build an AgentConfig
//! 4. Running the agent loop with capabilities
//!
//! Prerequisites:
//! - Set OPENAI_API_KEY environment variable
//!
//! Run with: cargo run -p everruns-core --example capabilities --features openai

use async_trait::async_trait;
use everruns_core::{
    apply_capabilities,
    capabilities::{Capability, CapabilityId, CapabilityRegistry, CapabilityStatus},
    config::AgentConfig,
    memory::{InMemoryEventEmitter, InMemoryMessageStore},
    message::{Message, MessageRole},
    openai::OpenAIProtocolLlmProvider,
    tools::{Tool, ToolExecutionResult},
    AgentLoop,
};
use serde_json::{json, Value};
use uuid::Uuid;

// ============================================================================
// Custom Capability: Calculator
// ============================================================================

/// A custom capability that provides a calculator tool
struct CalculatorCapability;

impl Capability for CalculatorCapability {
    fn id(&self) -> &str {
        // Custom capability with a custom ID
        "calculator"
    }

    fn name(&self) -> &str {
        "Calculator"
    }

    fn description(&self) -> &str {
        "Provides a calculator tool for basic arithmetic operations."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("calculator")
    }

    fn category(&self) -> Option<&str> {
        Some("Utilities")
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(CalculatorTool)]
    }
}

/// Calculator tool implementation
struct CalculatorTool;

#[async_trait]
impl Tool for CalculatorTool {
    fn name(&self) -> &str {
        "calculate"
    }

    fn description(&self) -> &str {
        "Perform basic arithmetic calculations. Supports add, subtract, multiply, and divide operations."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "description": "The operation to perform",
                    "enum": ["add", "subtract", "multiply", "divide"]
                },
                "a": {
                    "type": "number",
                    "description": "First operand"
                },
                "b": {
                    "type": "number",
                    "description": "Second operand"
                }
            },
            "required": ["operation", "a", "b"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        let operation = arguments.get("operation").and_then(|v| v.as_str());
        let a = arguments.get("a").and_then(|v| v.as_f64());
        let b = arguments.get("b").and_then(|v| v.as_f64());

        match (operation, a, b) {
            (Some(op), Some(a), Some(b)) => {
                let result = match op {
                    "add" => a + b,
                    "subtract" => a - b,
                    "multiply" => a * b,
                    "divide" => {
                        if b == 0.0 {
                            return ToolExecutionResult::tool_error(
                                "Division by zero is not allowed",
                            );
                        }
                        a / b
                    }
                    _ => {
                        return ToolExecutionResult::tool_error(format!(
                            "Unknown operation: {}",
                            op
                        ))
                    }
                };

                ToolExecutionResult::success(json!({
                    "expression": format!("{} {} {}", a, op, b),
                    "result": result
                }))
            }
            _ => ToolExecutionResult::tool_error(
                "Missing required parameters: operation, a, and b are required",
            ),
        }
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

fn print_conversation_steps(messages: &[Message]) {
    println!("\n  Steps:");
    for (i, msg) in messages.iter().enumerate() {
        match msg.role {
            MessageRole::User => {
                println!("    {}. [User] {}", i + 1, msg.content_to_llm_string());
            }
            MessageRole::Assistant => {
                let text = msg.content_to_llm_string();
                let tool_calls = msg.tool_calls();
                if !tool_calls.is_empty() {
                    println!("    {}. [Assistant] Calling tool(s):", i + 1);
                    for tc in tool_calls {
                        println!("       -> {}({})", tc.name, tc.arguments);
                    }
                    if !text.is_empty() {
                        println!("       Text: {}", text);
                    }
                } else if !text.is_empty() {
                    println!("    {}. [Assistant] {}", i + 1, text);
                }
            }
            MessageRole::ToolResult => {
                if let Some(tr) = msg.tool_result_content() {
                    if let Some(ref err) = tr.error {
                        println!("    {}. [Tool Result] Error: {}", i + 1, err);
                    } else if let Some(ref res) = tr.result {
                        println!("    {}. [Tool Result] {}", i + 1, res);
                    }
                }
            }
            MessageRole::System => {
                // Skip system messages
            }
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
        eprintln!("Please set it before running this example:");
        eprintln!("  export OPENAI_API_KEY=your-api-key");
        std::process::exit(1);
    }

    println!("=== Capabilities Demo (everruns-core) ===\n");

    // Example 1: Using built-in CurrentTime capability
    example_builtin_capability().await?;

    // Example 2: Using custom capability
    example_custom_capability().await?;

    // Example 3: Multiple capabilities
    example_multiple_capabilities().await?;

    println!("=== Demo completed! ===");
    Ok(())
}

/// Example 1: Using the built-in CurrentTime capability
async fn example_builtin_capability() -> anyhow::Result<()> {
    println!("--- Example 1: Built-in CurrentTime Capability ---\n");

    // Create capability registry with built-in capabilities
    let registry = CapabilityRegistry::with_builtins();

    // Base agent config
    let base_config = AgentConfig::new("You are a helpful assistant.", "gpt-5.2");

    // Apply the CurrentTime capability
    let capability_ids = vec![CapabilityId::CURRENT_TIME.to_string()];
    let applied = apply_capabilities(base_config, &capability_ids, &registry);

    println!("Applied capabilities: {:?}", applied.applied_ids);
    println!("Tools available: {:?}", applied.tool_registry.tool_names());
    println!();

    // Create components
    let event_emitter = InMemoryEventEmitter::new();
    let message_store = InMemoryMessageStore::new();
    let llm_provider = OpenAIProtocolLlmProvider::from_env()?;

    // Seed with user message
    let session_id = Uuid::now_v7();
    let user_message = "What's the current time?";
    message_store
        .seed(session_id, vec![Message::user(user_message)])
        .await;

    println!("User: {}\n", user_message);

    // Create and run agent loop with capability tools
    let agent_loop = AgentLoop::new(
        applied.config,
        event_emitter,
        message_store,
        llm_provider,
        applied.tool_registry,
    );

    let result = agent_loop.run(session_id).await?;

    print_conversation_steps(&result.messages);
    println!("\n  Final: {}", result.final_response.unwrap_or_default());
    println!("  (Iterations: {})\n", result.iterations);

    Ok(())
}

/// Example 2: Using a custom Calculator capability
async fn example_custom_capability() -> anyhow::Result<()> {
    println!("--- Example 2: Custom Calculator Capability ---\n");

    // Create custom registry with our calculator capability
    let mut registry = CapabilityRegistry::new();
    registry.register(CalculatorCapability);

    // Base agent config
    let base_config = AgentConfig::new("You are a helpful math assistant.", "gpt-5.2");

    // Apply the custom capability
    let capability_ids = vec!["calculator".to_string()];
    let applied = apply_capabilities(base_config, &capability_ids, &registry);

    println!("Applied capabilities: {:?}", applied.applied_ids);
    println!("Tools available: {:?}", applied.tool_registry.tool_names());
    println!();

    // Create components
    let event_emitter = InMemoryEventEmitter::new();
    let message_store = InMemoryMessageStore::new();
    let llm_provider = OpenAIProtocolLlmProvider::from_env()?;

    // Seed with user message
    let session_id = Uuid::now_v7();
    let user_message = "What is 123 multiplied by 456?";
    message_store
        .seed(session_id, vec![Message::user(user_message)])
        .await;

    println!("User: {}\n", user_message);

    // Create and run agent loop
    let agent_loop = AgentLoop::new(
        applied.config,
        event_emitter,
        message_store,
        llm_provider,
        applied.tool_registry,
    );

    let result = agent_loop.run(session_id).await?;

    print_conversation_steps(&result.messages);
    println!("\n  Final: {}", result.final_response.unwrap_or_default());
    println!("  (Iterations: {})\n", result.iterations);

    Ok(())
}

/// Example 3: Multiple capabilities combined
async fn example_multiple_capabilities() -> anyhow::Result<()> {
    println!("--- Example 3: Multiple Capabilities Combined ---\n");

    // Create registry with built-in capabilities
    let registry = CapabilityRegistry::with_builtins();

    // Base agent config
    let base_config = AgentConfig::new(
        "You are a helpful assistant with access to time and other utilities.",
        "gpt-5.2",
    );

    // Apply multiple capabilities
    let capability_ids = vec![
        CapabilityId::CURRENT_TIME.to_string(),
        CapabilityId::NOOP.to_string(),
    ];
    let applied = apply_capabilities(base_config, &capability_ids, &registry);

    println!("Applied capabilities: {:?}", applied.applied_ids);
    println!("Tools available: {:?}", applied.tool_registry.tool_names());
    println!();

    // Create components
    let event_emitter = InMemoryEventEmitter::new();
    let message_store = InMemoryMessageStore::new();
    let llm_provider = OpenAIProtocolLlmProvider::from_env()?;

    // Seed with user message
    let session_id = Uuid::now_v7();
    let user_message = "What time is it in human-readable format?";
    message_store
        .seed(session_id, vec![Message::user(user_message)])
        .await;

    println!("User: {}\n", user_message);

    // Create and run agent loop
    let agent_loop = AgentLoop::new(
        applied.config,
        event_emitter,
        message_store,
        llm_provider,
        applied.tool_registry,
    );

    let result = agent_loop.run(session_id).await?;

    print_conversation_steps(&result.messages);
    println!("\n  Final: {}", result.final_response.unwrap_or_default());
    println!("  (Iterations: {})\n", result.iterations);

    Ok(())
}

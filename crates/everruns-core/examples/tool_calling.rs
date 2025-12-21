//! Tool Calling Example - Agent Loop with Tool Trait
//!
//! This example demonstrates tool calling using the Tool trait abstraction
//! and ToolRegistry for tool management. Uses OpenAI as the LLM provider.
//!
//! Prerequisites:
//! - Set OPENAI_API_KEY environment variable
//!
//! Run with: cargo run -p everruns-core --example tool_calling --features openai

use async_trait::async_trait;
use everruns_core::{
    config::AgentConfig,
    memory::{InMemoryEventEmitter, InMemoryMessageStore},
    message::{Message, MessageContent, MessageRole},
    openai::OpenAIProtocolLlmProvider,
    tools::{Tool, ToolExecutionResult, ToolRegistry},
    AgentLoop,
};
use serde_json::{json, Value};
use uuid::Uuid;

// ============================================================================
// Custom Tools
// ============================================================================

/// Tool that returns the current date and time
struct GetCurrentTime;

#[async_trait]
impl Tool for GetCurrentTime {
    fn name(&self) -> &str {
        "get_current_time"
    }

    fn description(&self) -> &str {
        "Get the current date and time in various formats. Use this when asked about the current time or date."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "format": {
                    "type": "string",
                    "description": "Output format: 'iso8601' for ISO format, 'unix' for Unix timestamp, 'human' for readable format",
                    "enum": ["iso8601", "unix", "human"]
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
            "unix" => json!({
                "timestamp": now.timestamp(),
                "format": "unix"
            }),
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

/// Tool that performs basic arithmetic calculations
struct Calculator;

#[async_trait]
impl Tool for Calculator {
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

/// Tool that provides random facts
struct RandomFact;

#[async_trait]
impl Tool for RandomFact {
    fn name(&self) -> &str {
        "get_random_fact"
    }

    fn description(&self) -> &str {
        "Get a random interesting fact about a given topic."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "topic": {
                    "type": "string",
                    "description": "The topic to get a fact about (e.g., 'science', 'history', 'nature')"
                }
            },
            "required": ["topic"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        let topic = arguments
            .get("topic")
            .and_then(|v| v.as_str())
            .unwrap_or("general");

        let fact = match topic.to_lowercase().as_str() {
            "science" => "The human brain uses approximately 20% of the body's total energy.",
            "history" => "The Great Wall of China is not visible from space with the naked eye.",
            "nature" => "Honey never spoils. Archaeologists have found 3000-year-old honey in Egyptian tombs that was still edible.",
            "space" => "A day on Venus is longer than a year on Venus.",
            "animals" => "Octopuses have three hearts and blue blood.",
            _ => "The average person walks about 100,000 miles in their lifetime.",
        };

        ToolExecutionResult::success(json!({
            "topic": topic,
            "fact": fact
        }))
    }
}

// ============================================================================
// Helper to print conversation steps
// ============================================================================

fn print_conversation_steps(messages: &[Message]) {
    println!("\n  Steps:");
    for (i, msg) in messages.iter().enumerate() {
        match msg.role {
            MessageRole::User => {
                println!("    {}. [User] {}", i + 1, msg.content.to_llm_string());
            }
            MessageRole::Assistant => {
                let text = msg.content.to_llm_string();
                if let Some(ref tool_calls) = msg.tool_calls {
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
                } else if !text.is_empty() {
                    println!("    {}. [Assistant] {}", i + 1, text);
                }
            }
            MessageRole::ToolCall => {
                // Skip - already shown in assistant message
            }
            MessageRole::ToolResult => {
                if let MessageContent::ToolResult { result, error } = &msg.content {
                    if let Some(err) = error {
                        println!("    {}. [Tool Result] Error: {}", i + 1, err);
                    } else if let Some(res) = result {
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
    // Set up logging (WARN level to reduce noise)
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .init();

    // Check for API key
    if std::env::var("OPENAI_API_KEY").is_err() {
        eprintln!("Error: OPENAI_API_KEY environment variable is not set");
        eprintln!("Please set it before running this example:");
        eprintln!("  export OPENAI_API_KEY=your-api-key");
        std::process::exit(1);
    }

    println!("=== Tool Calling Demo (everruns-core) ===");
    println!("(Using OpenAI API with Tool trait abstraction)\n");

    // Run examples
    example_time_query().await?;
    example_calculation().await?;
    example_multi_tool().await?;

    println!("=== Demo completed! ===");
    Ok(())
}

/// Example 1: Ask about the current time
async fn example_time_query() -> anyhow::Result<()> {
    println!("--- Example 1: Time Query ---\n");

    // Create tool registry
    let registry = ToolRegistry::builder().tool(GetCurrentTime).build();

    // Create agent config with tools
    let config = AgentConfig::new(
        "You are a helpful assistant with access to a time tool. When asked about time, use the get_current_time tool.",
        "gpt-4o-mini",
    )
    .with_tools(registry.tool_definitions())
    .with_max_iterations(5);

    // Create components
    let event_emitter = InMemoryEventEmitter::new();
    let message_store = InMemoryMessageStore::new();
    let llm_provider = OpenAIProtocolLlmProvider::from_env()?;

    // Seed with user message
    let session_id = Uuid::now_v7();
    let user_message = "What time is it right now?";
    message_store
        .seed(session_id, vec![Message::user(user_message)])
        .await;

    println!("User: {}\n", user_message);

    // Create and run agent loop
    let agent_loop = AgentLoop::new(config, event_emitter, message_store, llm_provider, registry);

    let result = agent_loop.run(session_id).await?;

    print_conversation_steps(&result.messages);
    println!("\n  Final: {}", result.final_response.unwrap_or_default());
    println!("  (Iterations: {})\n", result.iterations);

    Ok(())
}

/// Example 2: Perform a calculation
async fn example_calculation() -> anyhow::Result<()> {
    println!("--- Example 2: Calculation ---\n");

    let registry = ToolRegistry::builder().tool(Calculator).build();

    let config = AgentConfig::new(
        "You are a helpful calculator assistant. Use the calculate tool for math operations.",
        "gpt-4o-mini",
    )
    .with_tools(registry.tool_definitions())
    .with_max_iterations(5);

    let event_emitter = InMemoryEventEmitter::new();
    let message_store = InMemoryMessageStore::new();
    let llm_provider = OpenAIProtocolLlmProvider::from_env()?;

    let session_id = Uuid::now_v7();
    let user_message = "What is 42 multiplied by 17?";
    message_store
        .seed(session_id, vec![Message::user(user_message)])
        .await;

    println!("User: {}\n", user_message);

    let agent_loop = AgentLoop::new(config, event_emitter, message_store, llm_provider, registry);

    let result = agent_loop.run(session_id).await?;

    print_conversation_steps(&result.messages);
    println!("\n  Final: {}", result.final_response.unwrap_or_default());
    println!("  (Iterations: {})\n", result.iterations);

    Ok(())
}

/// Example 3: Multiple tools available
async fn example_multi_tool() -> anyhow::Result<()> {
    println!("--- Example 3: Multi-Tool Query ---\n");

    // Register multiple tools
    let registry = ToolRegistry::builder()
        .tool(GetCurrentTime)
        .tool(Calculator)
        .tool(RandomFact)
        .build();

    println!("Available tools: {:?}\n", registry.tool_names());

    let config = AgentConfig::new(
        "You are a helpful assistant with access to multiple tools: get_current_time for time queries, calculate for math, and get_random_fact for interesting facts. Use the appropriate tool based on the user's request.",
        "gpt-4o-mini",
    )
    .with_tools(registry.tool_definitions())
    .with_max_iterations(5);

    let event_emitter = InMemoryEventEmitter::new();
    let message_store = InMemoryMessageStore::new();
    let llm_provider = OpenAIProtocolLlmProvider::from_env()?;

    let session_id = Uuid::now_v7();
    let user_message = "Tell me a random fact about nature.";
    message_store
        .seed(session_id, vec![Message::user(user_message)])
        .await;

    println!("User: {}\n", user_message);

    let agent_loop = AgentLoop::new(config, event_emitter, message_store, llm_provider, registry);

    let result = agent_loop.run(session_id).await?;

    print_conversation_steps(&result.messages);
    println!("\n  Final: {}", result.final_response.unwrap_or_default());
    println!("  (Iterations: {})\n", result.iterations);

    Ok(())
}

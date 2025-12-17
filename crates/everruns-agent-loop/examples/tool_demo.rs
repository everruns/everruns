//! Tool Demo - Agent Loop with Tool Registry
//!
//! This example demonstrates how to use the Tool trait and ToolRegistry
//! to add tools to an agent loop.
//!
//! Run with: cargo run --example tool_demo -p everruns-agent-loop

use async_trait::async_trait;
use everruns_agent_loop::{
    config::AgentConfig,
    memory::{InMemoryAgentLoopBuilder, InMemoryMessageStore, MockLlmProvider, MockLlmResponse},
    message::ConversationMessage,
    tools::{Tool, ToolExecutionResult, ToolRegistry},
    ToolCall,
};
use serde_json::{json, Value};
use uuid::Uuid;

// ============================================================================
// Custom Tools
// ============================================================================

/// A tool that calculates the sum of numbers
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
            "required": ["operation", "a", "b"]
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
                        "Unknown operation: {}. Valid operations: add, subtract, multiply, divide",
                        op
                    ))
                    }
                };

                ToolExecutionResult::success(json!({
                    "operation": op,
                    "a": a,
                    "b": b,
                    "result": result
                }))
            }
            _ => ToolExecutionResult::tool_error(
                "Missing required parameters: operation, a, and b are required",
            ),
        }
    }
}

/// A tool that generates random numbers (demonstrates async operations)
struct RandomGenerator;

#[async_trait]
impl Tool for RandomGenerator {
    fn name(&self) -> &str {
        "random_number"
    }

    fn description(&self) -> &str {
        "Generate a random number within a specified range."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "min": {
                    "type": "integer",
                    "description": "Minimum value (inclusive)",
                    "default": 1
                },
                "max": {
                    "type": "integer",
                    "description": "Maximum value (inclusive)",
                    "default": 100
                }
            }
        })
    }

    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        let min = arguments.get("min").and_then(|v| v.as_i64()).unwrap_or(1);
        let max = arguments.get("max").and_then(|v| v.as_i64()).unwrap_or(100);

        if min > max {
            return ToolExecutionResult::tool_error("min must be less than or equal to max");
        }

        // Simple pseudo-random (for demo purposes)
        // In real code, you'd use a proper RNG
        let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0) as u64;
        let random = (now % ((max - min + 1) as u64)) as i64 + min;

        ToolExecutionResult::success(json!({
            "min": min,
            "max": max,
            "value": random
        }))
    }
}

/// A tool that demonstrates internal error handling
struct UnreliableTool {
    fail_rate: f64,
}

impl UnreliableTool {
    fn new(fail_rate: f64) -> Self {
        Self {
            fail_rate: fail_rate.clamp(0.0, 1.0),
        }
    }
}

#[async_trait]
impl Tool for UnreliableTool {
    fn name(&self) -> &str {
        "unreliable_service"
    }

    fn description(&self) -> &str {
        "A service that sometimes fails (for testing error handling)"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "data": {
                    "type": "string",
                    "description": "Data to process"
                }
            },
            "required": ["data"]
        })
    }

    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        // Simulate random failures based on current time
        let now = chrono::Utc::now().timestamp_millis() as f64;
        let should_fail = (now % 100.0) / 100.0 < self.fail_rate;

        if should_fail {
            // This is an internal error - the real details won't be shown to the LLM
            return ToolExecutionResult::internal_error_msg(
                "Database connection failed: connection refused to postgres://internal:5432/db",
            );
        }

        let data = arguments.get("data").and_then(|v| v.as_str()).unwrap_or("");

        ToolExecutionResult::success(json!({
            "processed": data.to_uppercase(),
            "status": "success"
        }))
    }
}

// ============================================================================
// Main Demo
// ============================================================================

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Set up logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    println!("=== Agent Loop Tool Demo ===\n");

    // Example 1: Basic tool usage with GetCurrentTime
    demo_get_current_time().await?;

    // Example 2: Custom calculator tool
    demo_calculator().await?;

    // Example 3: Multiple tool calls
    demo_multiple_tools().await?;

    // Example 4: Tool error handling
    demo_error_handling().await?;

    println!("=== Demo completed! ===");
    Ok(())
}

/// Demo 1: Using the built-in GetCurrentTime tool
async fn demo_get_current_time() -> anyhow::Result<()> {
    println!("--- Demo 1: GetCurrentTime Tool ---\n");

    // Create tool registry with GetCurrentTime
    let registry = ToolRegistry::builder()
        .tool(everruns_agent_loop::GetCurrentTime)
        .build();

    // Create agent config with tool definitions from registry
    let config = AgentConfig::new(
        "You are a helpful assistant. When asked about time, use the get_current_time tool.",
        "gpt-5.2",
    )
    .with_tools(registry.tool_definitions())
    .with_max_iterations(3);

    // Create mock LLM that will request the tool
    let llm_provider = MockLlmProvider::new();

    // LLM decides to call the tool
    llm_provider
        .add_response(MockLlmResponse::with_tools(
            "Let me check the current time for you.",
            vec![ToolCall {
                id: "call_time_1".to_string(),
                name: "get_current_time".to_string(),
                arguments: json!({"format": "human"}),
            }],
        ))
        .await;

    // LLM responds with the tool result
    llm_provider
        .add_response(MockLlmResponse::text(
            "The current time has been retrieved successfully!",
        ))
        .await;

    // Create message store and seed user message
    let message_store = InMemoryMessageStore::new();
    let session_id = Uuid::now_v7();
    message_store
        .seed(
            session_id,
            vec![ConversationMessage::user("What time is it?")],
        )
        .await;

    // Note: We need to use the registry as the tool executor
    // For this demo, we'll use the mock tool executor and set results manually
    let (agent_loop, event_emitter, _, _, tool_executor) = InMemoryAgentLoopBuilder::new(config)
        .llm_provider(llm_provider)
        .message_store(message_store)
        .build_with_refs();

    // Set up mock tool result to simulate what GetCurrentTime would return
    let now = chrono::Utc::now();
    tool_executor
        .set_result(
            "get_current_time",
            json!({
                "datetime": now.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
                "format": "human"
            }),
        )
        .await;

    // Run the loop
    let result = agent_loop.run(session_id).await?;

    println!("Iterations: {}", result.iterations);
    println!("Events: {}", event_emitter.count().await);
    println!(
        "Final response: {}",
        result.final_response.unwrap_or_default()
    );

    let calls = tool_executor.calls().await;
    println!(
        "Tool calls: {:?}",
        calls.iter().map(|c| &c.name).collect::<Vec<_>>()
    );

    println!();
    Ok(())
}

/// Demo 2: Custom calculator tool
async fn demo_calculator() -> anyhow::Result<()> {
    println!("--- Demo 2: Calculator Tool ---\n");

    // Create registry with calculator
    let registry = ToolRegistry::builder().tool(Calculator).build();

    let config = AgentConfig::new("You are a helpful calculator assistant.", "gpt-5.2")
        .with_tools(registry.tool_definitions())
        .with_max_iterations(3);

    let llm_provider = MockLlmProvider::new();

    // LLM calls the calculator
    llm_provider
        .add_response(MockLlmResponse::with_tools(
            "I'll calculate that for you.",
            vec![ToolCall {
                id: "call_calc_1".to_string(),
                name: "calculate".to_string(),
                arguments: json!({
                    "operation": "multiply",
                    "a": 15,
                    "b": 7
                }),
            }],
        ))
        .await;

    llm_provider
        .add_response(MockLlmResponse::text("15 multiplied by 7 equals 105."))
        .await;

    let message_store = InMemoryMessageStore::new();
    let session_id = Uuid::now_v7();
    message_store
        .seed(
            session_id,
            vec![ConversationMessage::user("What is 15 times 7?")],
        )
        .await;

    let (agent_loop, _, _, _, tool_executor) = InMemoryAgentLoopBuilder::new(config)
        .llm_provider(llm_provider)
        .message_store(message_store)
        .build_with_refs();

    // Set expected result
    tool_executor
        .set_result(
            "calculate",
            json!({
                "operation": "multiply",
                "a": 15,
                "b": 7,
                "result": 105
            }),
        )
        .await;

    let result = agent_loop.run(session_id).await?;

    println!(
        "Final response: {}",
        result.final_response.unwrap_or_default()
    );

    // Show actual tool execution using the Calculator directly
    println!("\nDirect tool execution test:");
    let calc = Calculator;
    let calc_result = calc
        .execute(json!({"operation": "add", "a": 10, "b": 20}))
        .await;
    println!("  10 + 20 = {:?}", calc_result);

    let div_result = calc
        .execute(json!({"operation": "divide", "a": 10, "b": 0}))
        .await;
    println!("  10 / 0 = {:?}", div_result);

    println!();
    Ok(())
}

/// Demo 3: Multiple tool calls
async fn demo_multiple_tools() -> anyhow::Result<()> {
    println!("--- Demo 3: Multiple Tools ---\n");

    // Create registry with multiple tools
    let registry = ToolRegistry::builder()
        .tool(everruns_agent_loop::GetCurrentTime)
        .tool(Calculator)
        .tool(RandomGenerator)
        .build();

    println!("Registered tools: {:?}", registry.tool_names());
    println!(
        "Tool definitions count: {}",
        registry.tool_definitions().len()
    );

    // Show each tool's schema
    for name in registry.tool_names() {
        if let Some(tool) = registry.get(name) {
            println!("\nTool: {}", tool.name());
            println!("  Description: {}", tool.description());
            println!("  Parameters: {}", tool.parameters_schema());
        }
    }

    println!();
    Ok(())
}

/// Demo 4: Error handling
async fn demo_error_handling() -> anyhow::Result<()> {
    println!("--- Demo 4: Error Handling ---\n");

    // Test tool-level error
    println!("Testing tool-level error (shown to LLM):");
    let calc = Calculator;
    let result = calc
        .execute(json!({"operation": "divide", "a": 10, "b": 0}))
        .await;
    let tool_result = match result {
        everruns_agent_loop::ToolExecutionResult::ToolError(msg) => {
            println!("  Tool error message: {}", msg);
            everruns_agent_loop::ToolResult {
                tool_call_id: "test".to_string(),
                result: None,
                error: Some(msg),
            }
        }
        _ => panic!("Expected tool error"),
    };
    println!("  Error returned to LLM: {:?}", tool_result.error);

    // Test internal error (hidden from LLM)
    println!("\nTesting internal error (hidden from LLM):");
    let unreliable = UnreliableTool::new(1.0); // 100% failure rate
    let result = unreliable.execute(json!({"data": "test"})).await;
    let tool_result = match result {
        everruns_agent_loop::ToolExecutionResult::InternalError(err) => {
            println!("  Internal error (logged): {}", err.message);
            everruns_agent_loop::ToolResult {
                tool_call_id: "test".to_string(),
                result: None,
                error: Some("An internal error occurred while executing the tool".to_string()),
            }
        }
        _ => {
            println!("  (Tool succeeded this time)");
            return Ok(());
        }
    };
    println!(
        "  Error returned to LLM: {:?}",
        tool_result.error.as_ref().unwrap()
    );
    println!("  Note: Sensitive database details are NOT exposed!");

    // Test with FailingTool
    println!("\nUsing FailingTool for guaranteed errors:");
    let failing = everruns_agent_loop::FailingTool::with_internal_error(
        "Secret API key: sk-abc123 failed to authenticate",
    );
    let result = failing.execute(json!({})).await;
    if let everruns_agent_loop::ToolExecutionResult::InternalError(err) = result {
        let tool_result = everruns_agent_loop::ToolExecutionResult::InternalError(err)
            .into_tool_result("call_1", "failing_tool");
        println!("  Generic error to LLM: {:?}", tool_result.error);
    }

    println!();
    Ok(())
}

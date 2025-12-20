//! Decomposed Execution Example - ReACT Loop
//!
//! This example demonstrates using atoms directly to implement a ReACT
//! (Reason-Act) loop. Each iteration:
//!
//! 1. **Reason**: Call the model to decide what to do next
//! 2. **Act**: Execute any requested tool calls
//! 3. **Repeat**: Loop until the model provides a final response
//!
//! This pattern is suitable for:
//! - Temporal workflow activities
//! - Custom orchestration logic
//! - Debugging and testing individual steps
//!
//! Prerequisites:
//! - Set OPENAI_API_KEY environment variable
//!
//! Run with: cargo run -p everruns-core --example decomposed_execution --features openai

use everruns_core::{
    atoms::{
        AddUserMessageAtom, AddUserMessageInput, Atom, CallModelAtom, CallModelInput,
        ExecuteToolAtom, ExecuteToolInput,
    },
    config::AgentConfigBuilder,
    memory::InMemoryMessageStore,
    openai::OpenAIProtocolLlmProvider,
    tools::{Tool, ToolExecutionResult, ToolRegistry, ToolRegistryBuilder},
    MessageStore,
};
use serde_json::{json, Value};
use uuid::Uuid;

/// A simple weather tool for demonstration
struct GetWeatherTool;

#[async_trait::async_trait]
impl Tool for GetWeatherTool {
    fn name(&self) -> &str {
        "get_weather"
    }

    fn description(&self) -> &str {
        "Get the current weather for a location"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "location": {
                    "type": "string",
                    "description": "The city name"
                }
            },
            "required": ["location"]
        })
    }

    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        let location = arguments
            .get("location")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown");

        // Simulated weather data
        ToolExecutionResult::Success(json!({
            "location": location,
            "temperature": "22°C",
            "conditions": "Sunny",
            "humidity": "45%"
        }))
    }
}

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

    println!("=== ReACT Loop with Atoms ===\n");

    // Create shared dependencies
    let message_store = InMemoryMessageStore::new();
    let llm_provider = OpenAIProtocolLlmProvider::from_env()?;
    let tools: ToolRegistry = ToolRegistryBuilder::new().tool(GetWeatherTool).build();

    // Create atoms
    let add_user_message = AddUserMessageAtom::new(message_store.clone());
    let call_model = CallModelAtom::new(message_store.clone(), llm_provider.clone());
    let execute_tool = ExecuteToolAtom::new(message_store.clone(), tools.clone());

    // Create config with tools
    let config = AgentConfigBuilder::new()
        .system_prompt(
            "You are a helpful weather assistant. Use the get_weather tool to answer weather questions.",
        )
        .model("gpt-4o-mini")
        .tools(tools.tool_definitions())
        .build();

    let session_id = Uuid::now_v7();
    let user_question = "What's the weather like in Paris?";
    let max_iterations = 5;

    println!("Session: {}", session_id);
    println!("User: {}\n", user_question);

    // =========================================================================
    // Add user message
    // =========================================================================
    add_user_message
        .execute(AddUserMessageInput {
            session_id,
            content: user_question.to_string(),
        })
        .await?;

    // =========================================================================
    // ReACT Loop
    // =========================================================================
    let mut final_response = String::new();

    for iteration in 1..=max_iterations {
        println!("━━━ Iteration {} ━━━", iteration);

        // =====================================================================
        // REASON: Call the model
        // =====================================================================
        println!("  [Reason] Calling model...");

        let model_result = call_model
            .execute(CallModelInput {
                session_id,
                config: config.clone(),
            })
            .await?;

        // Capture response
        if !model_result.text.is_empty() {
            final_response = model_result.text.clone();
            println!("    Response: {}", truncate(&model_result.text, 60));
        }

        // Check if we're done (no tool calls = final response)
        if !model_result.needs_tool_execution {
            println!("  [Done] No tool calls, returning final response\n");
            break;
        }

        println!(
            "    Tool calls requested: {}",
            model_result.tool_calls.len()
        );

        // =====================================================================
        // ACT: Execute tool calls
        // =====================================================================
        println!("  [Act] Executing tools...");

        for tool_call in &model_result.tool_calls {
            println!("    Tool: {}", tool_call.name);
            println!("    Args: {}", tool_call.arguments);

            let tool_result = execute_tool
                .execute(ExecuteToolInput {
                    session_id,
                    tool_call: tool_call.clone(),
                    tool_definitions: config.tools.clone(),
                })
                .await?;

            let result_str = tool_result
                .result
                .result
                .as_ref()
                .map(|v: &Value| truncate(&v.to_string(), 50))
                .unwrap_or_else(|| "None".to_string());

            println!("    Result: {}", result_str);
        }

        println!();

        // Safety check
        if iteration == max_iterations {
            println!("  [Warning] Max iterations reached!");
        }
    }

    // =========================================================================
    // Final Output
    // =========================================================================
    println!("━━━ Final Response ━━━");
    println!("Assistant: {}", final_response);

    // =========================================================================
    // Conversation History
    // =========================================================================
    println!("\n━━━ Conversation History ━━━");
    let messages = message_store.load(session_id).await?;
    for (i, msg) in messages.iter().enumerate() {
        println!(
            "  {}. [{:?}] {}",
            i + 1,
            msg.role,
            truncate(&msg.content.to_llm_string(), 60)
        );
    }

    println!("\n=== Demo completed! ===");
    Ok(())
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len])
    }
}

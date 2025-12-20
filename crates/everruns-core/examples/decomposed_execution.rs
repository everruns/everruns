//! Decomposed Execution Example
//!
//! This example demonstrates using atoms directly for fine-grained control
//! over the agent execution flow. Each atom is a self-contained operation
//! that can be executed independently, making it suitable for:
//!
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

    println!("=== Decomposed Execution with Atoms ===\n");

    // Create shared dependencies
    let message_store = InMemoryMessageStore::new();
    let llm_provider = OpenAIProtocolLlmProvider::from_env()?;
    let tools: ToolRegistry = ToolRegistryBuilder::new().tool(GetWeatherTool).build();

    // Create config with tools
    let config = AgentConfigBuilder::new()
        .system_prompt("You are a helpful weather assistant. Use the get_weather tool to answer weather questions.")
        .model("gpt-4o-mini")
        .tools(tools.tool_definitions())
        .build();

    let session_id = Uuid::now_v7();
    let user_question = "What's the weather like in Paris?";

    println!("Session: {}", session_id);
    println!("User: {}\n", user_question);

    // =========================================================================
    // Step 1: Add user message using AddUserMessageAtom
    // =========================================================================
    println!("--- Step 1: AddUserMessageAtom ---");

    let add_message_atom = AddUserMessageAtom::new(message_store.clone());
    let add_result = add_message_atom
        .execute(AddUserMessageInput {
            session_id,
            content: user_question.to_string(),
        })
        .await?;

    println!("  Stored user message: {:?}", add_result.message.id);

    // =========================================================================
    // Step 2: Call model using CallModelAtom
    // =========================================================================
    println!("\n--- Step 2: CallModelAtom ---");

    let call_model_atom = CallModelAtom::new(message_store.clone(), llm_provider.clone());
    let model_result = call_model_atom
        .execute(CallModelInput {
            session_id,
            config: config.clone(),
        })
        .await?;

    println!("  Response text: {}", truncate(&model_result.text, 50));
    println!("  Tool calls: {}", model_result.tool_calls.len());
    println!(
        "  Needs tool execution: {}",
        model_result.needs_tool_execution
    );

    // =========================================================================
    // Step 3: Execute tools if needed using ExecuteToolAtom
    // =========================================================================
    if model_result.needs_tool_execution {
        println!("\n--- Step 3: ExecuteToolAtom (for each tool call) ---");

        let execute_tool_atom = ExecuteToolAtom::new(message_store.clone(), tools.clone());

        for tool_call in &model_result.tool_calls {
            println!("  Executing tool: {}", tool_call.name);
            println!("    Arguments: {}", tool_call.arguments);

            let tool_result = execute_tool_atom
                .execute(ExecuteToolInput {
                    session_id,
                    tool_call: tool_call.clone(),
                    tool_definitions: config.tools.clone(),
                })
                .await?;

            println!(
                "    Result: {}",
                tool_result
                    .result
                    .result
                    .as_ref()
                    .map(|v: &Value| truncate(&v.to_string(), 60))
                    .unwrap_or_else(|| "None".to_string())
            );
        }

        // =====================================================================
        // Step 4: Call model again to get final response
        // =====================================================================
        println!("\n--- Step 4: CallModelAtom (final response) ---");

        let final_result = call_model_atom
            .execute(CallModelInput {
                session_id,
                config: config.clone(),
            })
            .await?;

        println!("\nAssistant: {}", final_result.text);
    } else {
        println!("\nAssistant: {}", model_result.text);
    }

    // =========================================================================
    // Print conversation history
    // =========================================================================
    println!("\n--- Conversation History ---");
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

//! In-Memory Agentic Loop Example
//!
//! This example demonstrates the simplified API for running agentic loops
//! entirely in memory. Perfect for testing, prototyping, and experimentation.
//!
//! Run with: cargo run -p everruns-core --example in_memory_agentic_loop

use everruns_core::in_memory_loop::InMemoryAgenticLoop;
use everruns_core::llmsim_driver::LlmSimConfig;
use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_core::ToolCall;
use serde_json::{Value, json};

/// Simple weather tool for demonstration
struct GetWeatherTool;

#[async_trait::async_trait]
impl Tool for GetWeatherTool {
    fn name(&self) -> &str {
        "get_weather"
    }

    fn description(&self) -> &str {
        "Get the current weather for a city"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "city": {
                    "type": "string",
                    "description": "The city name"
                }
            },
            "required": ["city"]
        })
    }

    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        let city = arguments
            .get("city")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown");

        ToolExecutionResult::Success(json!({
            "city": city,
            "temperature": "22°C",
            "condition": "Sunny"
        }))
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("=== In-Memory Agentic Loop Examples ===\n");

    // =========================================================================
    // Example 1: Simple fixed response
    // =========================================================================
    println!("--- Example 1: Fixed Response ---");

    let runner = InMemoryAgenticLoop::with_fixed_response("Hello! I'm your assistant.")
        .await?;

    let result = runner.run_turn("Hi there!").await?;
    println!("User: Hi there!");
    println!("Agent: {}", result.response);
    println!("Success: {}, Iterations: {}\n", result.success, result.iterations);

    // =========================================================================
    // Example 2: Echo mode
    // =========================================================================
    println!("--- Example 2: Echo Mode ---");

    let runner = InMemoryAgenticLoop::with_echo().await?;

    let result = runner.run_turn("Can you repeat this?").await?;
    println!("User: Can you repeat this?");
    println!("Agent: {}\n", result.response);

    // =========================================================================
    // Example 3: Multi-turn conversation
    // =========================================================================
    println!("--- Example 3: Multi-Turn Conversation ---");

    let runner = InMemoryAgenticLoop::with_sequence(vec![
        "Hello! I'm here to help.",
        "Of course! What would you like to know?",
        "The capital of France is Paris.",
    ])
    .await?;

    let messages = &["Hello!", "I have a question", "What's the capital of France?"];
    let results = runner.run_conversation(messages).await?;

    for (msg, result) in messages.iter().zip(results.iter()) {
        println!("User: {}", msg);
        println!("Agent: {}", result.response);
    }

    // Show conversation history
    println!("\nConversation history:");
    println!("{}", runner.conversation_string().await?);

    // =========================================================================
    // Example 4: With tool calls
    // =========================================================================
    println!("--- Example 4: With Tool Calls ---");

    // Create tool calls that the simulated LLM will return
    let weather_call = ToolCall {
        id: "call_weather_1".to_string(),
        name: "get_weather".to_string(),
        arguments: json!({"city": "Tokyo"}),
    };

    // First response returns tool call, second response is final
    let runner = InMemoryAgenticLoop::builder()
        .system_prompt("You are a helpful weather assistant.")
        .with_llm_sim(
            LlmSimConfig::sequence(vec![
                "Let me check the weather for you.".to_string(),
                "The weather in Tokyo is 22°C and sunny!".to_string(),
            ])
            .with_tool_call_sequence(vec![
                vec![weather_call], // First call: return tool call
                vec![],             // Second call: no more tools
            ]),
        )
        .tool(GetWeatherTool)
        .build()
        .await?;

    let result = runner.run_turn("What's the weather in Tokyo?").await?;
    println!("User: What's the weather in Tokyo?");
    println!("Agent: {}", result.response);
    println!(
        "Tool calls made: {}, Iterations: {}\n",
        result.tool_calls_count, result.iterations
    );

    // =========================================================================
    // Example 5: Builder with full customization
    // =========================================================================
    println!("--- Example 5: Custom Builder ---");

    let runner = InMemoryAgenticLoop::builder()
        .agent_name("Research Assistant")
        .system_prompt("You are a research assistant specialized in science topics.")
        .with_simulated_response("That's a great question about quantum physics!")
        .max_iterations(3)
        .build()
        .await?;

    let result = runner.run_turn("Explain quantum entanglement").await?;
    println!("User: Explain quantum entanglement");
    println!("Agent: {}", result.response);

    // =========================================================================
    // Example 6: Inspecting events
    // =========================================================================
    println!("\n--- Example 6: Event Inspection ---");

    let runner = InMemoryAgenticLoop::with_fixed_response("Events are tracked!")
        .await?;

    runner.run_turn("Test message").await?;

    println!("Events emitted:");
    for event in runner.events().await {
        println!("  - {}", event.event_type);
    }
    println!("Total events: {}", runner.event_count().await);

    // =========================================================================
    // Example 7: Using with real LLM (requires API key)
    // =========================================================================
    println!("\n--- Example 7: Real LLM (if API key available) ---");

    // To use real LLMs, you need to:
    // 1. Import the provider crates (everruns-openai, everruns-anthropic)
    // 2. Register the drivers with a DriverRegistry
    // 3. Pass the registry to the builder
    //
    // Example (uncomment if you have the dependencies):
    // ```
    // use everruns_core::llm_driver_registry::DriverRegistry;
    // use everruns_core::traits::ModelWithProvider;
    // use everruns_core::llm_models::LlmProviderType;
    //
    // let mut registry = DriverRegistry::new();
    // everruns_openai::register_driver(&mut registry);
    //
    // let model = ModelWithProvider {
    //     model: "gpt-4o".to_string(),
    //     provider_type: LlmProviderType::Openai,
    //     api_key: Some(std::env::var("OPENAI_API_KEY")?),
    //     base_url: None,
    // };
    //
    // let runner = InMemoryAgenticLoop::builder()
    //     .with_driver_registry(model, registry)
    //     .build()
    //     .await?;
    // ```
    println!("See source code for example of using real LLMs");

    println!("\n=== Examples completed! ===");
    Ok(())
}

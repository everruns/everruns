//! Turn-Based Execution Example
//!
//! This example demonstrates the new turn-based atoms:
//! - InputAtom: Records user input and starts a turn
//! - ReasonAtom: LLM call with context preparation
//! - ActAtom: Parallel tool execution
//!
//! The turn loop:
//! 1. **Input**: Record user message (using InputAtom)
//! 2. **Reason**: Call the model (using ReasonAtom)
//! 3. **Act**: Execute tools if needed (using ActAtom)
//! 4. **Repeat**: Loop until no more tool calls
//!
//! This demonstrates the new AtomContext pattern which tracks:
//! - session_id: The session
//! - turn_id: Unique identifier for this turn
//! - input_message_id: The message that triggered this turn
//! - exec_id: Unique identifier for each atom execution
//!
//! Prerequisites:
//! - Set OPENAI_API_KEY or ANTHROPIC_API_KEY environment variable
//!
//! Run with: cargo run -p everruns-core --example turn_based_execution

use chrono::Utc;
use everruns_core::{
    agent::{Agent, AgentStatus},
    atoms::{
        ActAtom, ActInput, Atom, AtomContext, InputAtom, InputAtomInput, ReasonAtom, ReasonInput,
    },
    capabilities::CapabilityRegistry,
    llm_driver_registry::DriverRegistry,
    memory::{
        InMemoryAgentStore, InMemoryEventEmitter, InMemoryLlmProviderStore, InMemoryMessageStore,
        InMemorySessionStore,
    },
    session::{Session, SessionStatus},
    tools::{Tool, ToolExecutionResult, ToolRegistry, ToolRegistryBuilder},
    InputMessage, MessageStore,
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

    if std::env::var("OPENAI_API_KEY").is_err() && std::env::var("ANTHROPIC_API_KEY").is_err() {
        eprintln!("Error: No API key environment variable is set");
        eprintln!("  export OPENAI_API_KEY=your-api-key");
        eprintln!("  or");
        eprintln!("  export ANTHROPIC_API_KEY=your-api-key");
        std::process::exit(1);
    }

    println!("=== Turn-Based Execution with New Atoms ===\n");

    // Create shared dependencies
    let agent_store = InMemoryAgentStore::new();
    let session_store = InMemorySessionStore::new();
    let message_store = InMemoryMessageStore::new();
    let provider_store = InMemoryLlmProviderStore::from_env().await;
    let tools: ToolRegistry = ToolRegistryBuilder::new().tool(GetWeatherTool).build();

    // Create an agent in the store
    let agent_id = Uuid::now_v7();
    let session_id = Uuid::now_v7();
    let now = Utc::now();
    let agent = Agent {
        id: agent_id,
        name: "Weather Assistant".to_string(),
        description: Some("A helpful weather assistant".to_string()),
        system_prompt: "You are a helpful weather assistant. Use the get_weather tool to answer weather questions.".to_string(),
        default_model_id: None,
        tags: vec![],
        capabilities: vec![],
        status: AgentStatus::Active,
        created_at: now,
        updated_at: now,
    };
    agent_store.add_agent(agent).await;

    // Create a session in the store
    let session = Session {
        id: session_id,
        agent_id,
        title: Some("Weather Query".to_string()),
        tags: vec![],
        model_id: None,
        status: SessionStatus::Pending,
        created_at: now,
        started_at: None,
        finished_at: None,
    };
    session_store.add_session(session).await;

    // Create capability and driver registries
    let capability_registry = CapabilityRegistry::new();
    let driver_registry = {
        let mut registry = DriverRegistry::new();
        everruns_openai::register_driver(&mut registry);
        everruns_anthropic::register_driver(&mut registry);
        registry
    };

    // =========================================================================
    // Setup: Add user message to store (simulating API layer)
    // =========================================================================
    let user_question = "What's the weather like in Paris?";
    println!("Session: {}", session_id);
    println!("User: {}\n", user_question);

    // Add user message to store (this would be done by the API layer)
    let user_message = message_store
        .add(session_id, InputMessage::user(user_question))
        .await?;

    // =========================================================================
    // Create Turn Context
    // =========================================================================
    let turn_id = Uuid::now_v7();
    let base_context = AtomContext::new(session_id, turn_id, user_message.id);

    println!("Turn ID: {}", turn_id);
    println!("Input Message ID: {}\n", user_message.id);

    // =========================================================================
    // Create Atoms
    // =========================================================================
    // Use InMemoryEventEmitter to track events emitted by atoms
    let event_emitter = InMemoryEventEmitter::new();

    let input_atom = InputAtom::new(message_store.clone(), event_emitter.clone());
    let reason_atom = ReasonAtom::new(
        agent_store.clone(),
        session_store,
        message_store.clone(),
        provider_store,
        capability_registry,
        driver_registry,
        event_emitter.clone(),
    );
    let act_atom = ActAtom::new(tools.clone(), event_emitter.clone());

    // =========================================================================
    // Step 1: Input Atom - Record user message
    // =========================================================================
    println!("━━━ Step 1: InputAtom ━━━");
    let context = base_context.clone();
    println!("  Exec ID: {}", context.exec_id);

    let input_result = input_atom
        .execute(InputAtomInput {
            context: context.clone(),
        })
        .await?;

    println!(
        "  Message retrieved: {:?} - {}",
        input_result.message.role,
        truncate(input_result.message.text().unwrap_or(""), 40)
    );
    println!();

    // =========================================================================
    // Turn Loop: Reason → Act → Repeat
    // =========================================================================
    let max_iterations = 5;
    let mut final_response = String::new();

    for iteration in 1..=max_iterations {
        // =====================================================================
        // Reason Atom - Call the model
        // =====================================================================
        println!("━━━ Iteration {} - ReasonAtom ━━━", iteration);
        let reason_context = base_context.next_exec();
        println!("  Exec ID: {}", reason_context.exec_id);

        let reason_result = reason_atom
            .execute(ReasonInput {
                context: reason_context,
                agent_id,
            })
            .await?;

        println!("  Success: {}", reason_result.success);
        if !reason_result.text.is_empty() {
            final_response = reason_result.text.clone();
            println!("  Response: {}", truncate(&reason_result.text, 50));
        }
        println!(
            "  Has tool calls: {} (count: {})",
            reason_result.has_tool_calls,
            reason_result.tool_calls.len()
        );

        // Check if we're done
        if !reason_result.has_tool_calls || reason_result.tool_calls.is_empty() {
            println!("\n  [Done] No tool calls, turn complete.");
            break;
        }

        // =====================================================================
        // Act Atom - Execute tools in parallel
        // =====================================================================
        println!("\n━━━ Iteration {} - ActAtom ━━━", iteration);
        let act_context = base_context.next_exec();
        println!("  Exec ID: {}", act_context.exec_id);
        println!(
            "  Tool calls: {:?}",
            reason_result
                .tool_calls
                .iter()
                .map(|tc| &tc.name)
                .collect::<Vec<_>>()
        );

        let act_result = act_atom
            .execute(ActInput {
                context: act_context,
                tool_calls: reason_result.tool_calls.clone(),
                tool_definitions: reason_result.tool_definitions.clone(),
            })
            .await?;

        println!("  Completed: {}", act_result.completed);
        println!(
            "  Results: {} success, {} errors",
            act_result.success_count, act_result.error_count
        );

        for result in &act_result.results {
            let result_preview = result
                .result
                .result
                .as_ref()
                .map(|v| truncate(&v.to_string(), 40))
                .unwrap_or_else(|| "None".to_string());
            println!(
                "    {} [{}]: {}",
                result.tool_call.name, result.status, result_preview
            );
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
    println!("\n━━━ Final Response ━━━");
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
            truncate(&msg.content_to_llm_string(), 60)
        );
    }

    // =========================================================================
    // Emitted Events
    // =========================================================================
    println!("\n━━━ Emitted Events ━━━");
    let events = event_emitter.events().await;
    for (i, event) in events.iter().enumerate() {
        println!("  {}. {}", i + 1, event.event_type);
    }
    println!("  Total events: {}", events.len());

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

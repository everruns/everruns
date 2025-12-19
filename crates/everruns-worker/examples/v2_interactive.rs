// V2 Workflow Example - Interactive Session with Events
//
// This example demonstrates the event-driven interactive session API.
// Run with: cargo run --example v2_interactive

use everruns_worker::v2::*;
use std::sync::Arc;

#[tokio::main]
async fn main() {
    // Initialize tracing for logs
    tracing_subscriber::fmt().with_env_filter("info").init();

    println!("=== V2 Workflow Example: Interactive Session ===\n");

    // Set up mock activities
    let agent_config = AgentConfig::test("interactive-agent")
        .with_system_prompt("You are a helpful calculator assistant.")
        .with_tool(calculator_tool_def());

    let agent_loader = Arc::new(MockAgentLoader::new());
    agent_loader.register(agent_config.clone());

    let llm_caller = Arc::new(MockLlmCaller::new());
    // Response 1: use calculator
    llm_caller.add_response(LlmResponse::with_tools(
        "I'll calculate that for you.",
        vec![ToolCall::new(
            "calculator",
            serde_json::json!({
                "operation": "add",
                "a": 5,
                "b": 3
            }),
        )],
    ));
    // Response 2: final answer
    llm_caller.add_response(LlmResponse::text("5 + 3 = 8"));

    // Use builtin tool executor for calculator
    let tool_executor = Arc::new(BuiltinToolExecutorAdapter);

    let context = Arc::new(ActivityContext::new(
        agent_loader,
        llm_caller,
        tool_executor,
    ));

    let input = SessionInput::new(agent_config.agent_id);
    let (mut session, mut events) = InteractiveSession::new(input, context);

    // Spawn event handler
    let event_handle = tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            match event {
                SessionEvent::Ready => {
                    println!("[Event] Session ready for input");
                }
                SessionEvent::Response { text } => {
                    println!("[Event] Response: {}", text);
                }
                SessionEvent::ToolCall { name, arguments } => {
                    println!("[Event] Tool call: {}({})", name, arguments);
                }
                SessionEvent::ToolResult { name, result } => {
                    println!("[Event] Tool result: {} -> {}", name, result);
                }
                SessionEvent::Error { message } => {
                    println!("[Event] Error: {}", message);
                }
                SessionEvent::Completed { turns } => {
                    println!("[Event] Session completed after {} turns", turns);
                    break;
                }
            }
        }
    });

    // Initialize session
    println!("Initializing session...");
    session.init().await.unwrap();

    // Small delay to let event be processed
    tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

    // Chat
    println!("\nUser: What is 5 + 3?");
    session.chat("What is 5 + 3?").await.unwrap();

    // Small delay to let events be processed
    tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

    // Show final state
    println!("\n--- Final State ---");
    if let SessionState::Waiting {
        messages,
        turn_count,
        ..
    } = session.state()
    {
        println!("Turn count: {}", turn_count);
        println!("Messages: {}", messages.len());
        for msg in messages {
            let role = format!("{:?}", msg.role);
            let content = match &msg.content {
                MessageContent::Text(t) => t.clone(),
                MessageContent::ToolResult(r) => format!("Result: {}", r),
                MessageContent::ToolError(e) => format!("Error: {}", e),
            };
            println!("  {}: {}", role, content);
        }
    }

    // Shutdown
    println!("\nShutting down...");
    session.shutdown().await.unwrap();

    // Wait for event handler to finish
    let _ = event_handle.await;

    println!("\nDone!");
}

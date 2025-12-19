// V2 Workflow Example - Basic Conversation
//
// This example demonstrates the v2 session workflow with a simple conversation.
// Run with: cargo run --example v2_basic

use everruns_worker::v2::*;

#[tokio::main]
async fn main() {
    // Initialize tracing for logs
    tracing_subscriber::fmt().with_env_filter("info").init();

    println!("=== V2 Workflow Example: Basic Conversation ===\n");

    // Create a simple agent configuration
    let agent_config =
        AgentConfig::test("assistant").with_system_prompt("You are a helpful assistant.");

    // Build the executor with scripted LLM responses
    let mut executor = SessionBuilder::new()
        .with_agent(agent_config)
        .with_llm_response(LlmResponse::text(
            "Hello! I'm your assistant. How can I help you today?",
        ))
        .with_llm_response(LlmResponse::text(
            "I'm doing great, thank you for asking! I'm here to help with any questions you might have.",
        ))
        .with_llm_response(LlmResponse::text(
            "Goodbye! Feel free to come back anytime you need help.",
        ))
        .build();

    // Start the session
    println!("Starting session...");
    executor.start().await.unwrap();
    println!("Session started, workflow is now waiting for input.\n");

    // First turn
    println!("User: Hello!");
    executor
        .send_message(Message::user("Hello!"))
        .await
        .unwrap();
    print_last_response(&executor);

    // Second turn
    println!("User: How are you doing?");
    executor
        .send_message(Message::user("How are you doing?"))
        .await
        .unwrap();
    print_last_response(&executor);

    // Third turn
    println!("User: Bye!");
    executor.send_message(Message::user("Bye!")).await.unwrap();
    print_last_response(&executor);

    // Shutdown
    println!("\nShutting down session...");
    let output = executor.shutdown().await.unwrap();
    println!(
        "Session completed: {} turns, status: {:?}",
        output.total_turns, output.status
    );

    // Print full conversation history
    println!("\n=== Full Conversation History ===");
    if let SessionState::Completed { messages, .. } = executor.state() {
        for msg in messages {
            let role = match msg.role {
                MessageRole::User => "User",
                MessageRole::Assistant => "Assistant",
                MessageRole::System => "System",
                MessageRole::Tool => "Tool",
            };
            let content = msg.content.as_text().unwrap_or("(non-text content)");
            println!("{}: {}", role, content);
        }
    }
}

fn print_last_response(executor: &SessionExecutor) {
    if let SessionState::Waiting { messages, .. } = executor.state() {
        if let Some(last_msg) = messages.last() {
            if last_msg.role == MessageRole::Assistant {
                if let Some(text) = last_msg.content.as_text() {
                    println!("Assistant: {}\n", text);
                }
            }
        }
    }
}

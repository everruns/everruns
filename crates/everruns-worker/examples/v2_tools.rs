// V2 Workflow Example - Tool Calls
//
// This example demonstrates the v2 session workflow with tool calling.
// Run with: cargo run --example v2_tools

use everruns_worker::v2::*;

#[tokio::main]
async fn main() {
    // Initialize tracing for logs
    tracing_subscriber::fmt().with_env_filter("info").init();

    println!("=== V2 Workflow Example: Tool Calls ===\n");

    // Create an agent with tools
    let agent_config = AgentConfig::test("assistant-with-tools")
        .with_system_prompt("You are a helpful assistant with access to tools.")
        .with_tool(ToolDefinition::new("get_time", "Gets the current time"))
        .with_tool(ToolDefinition::new(
            "get_weather",
            "Gets the current weather",
        ));

    // Build the executor with scripted responses
    let mut executor = SessionBuilder::new()
        .with_agent(agent_config)
        // First response: ask for time -> tool call
        .with_llm_response(LlmResponse::with_tools(
            "Let me check the current time for you.",
            vec![ToolCall::new("get_time", serde_json::json!({}))],
        ))
        // After tool result: final response
        .with_llm_response(LlmResponse::text("The current time is 12:00 PM UTC."))
        // Second turn: multiple tools in parallel
        .with_llm_response(LlmResponse::with_tools(
            "I'll check both the time and weather for you.",
            vec![
                ToolCall {
                    id: "call_time".to_string(),
                    name: "get_time".to_string(),
                    arguments: serde_json::json!({}),
                },
                ToolCall {
                    id: "call_weather".to_string(),
                    name: "get_weather".to_string(),
                    arguments: serde_json::json!({"location": "New York"}),
                },
            ],
        ))
        // After both tools complete
        .with_llm_response(LlmResponse::text(
            "It's currently 12:00 PM and the weather in New York is sunny with 72°F.",
        ))
        // Tool results
        .with_tool_result(
            "get_time",
            serde_json::json!({
                "time": "12:00 PM",
                "timezone": "UTC"
            }),
        )
        .with_tool_result(
            "get_weather",
            serde_json::json!({
                "temperature": 72,
                "unit": "F",
                "condition": "sunny",
                "location": "New York"
            }),
        )
        .build();

    // Start the session
    println!("Starting session...");
    executor.start().await.unwrap();
    println!("Session started.\n");

    // First turn: single tool call
    println!("User: What time is it?");
    executor
        .send_message(Message::user("What time is it?"))
        .await
        .unwrap();
    print_conversation(&executor);

    // Second turn: multiple parallel tool calls
    println!("\nUser: What's the time and weather in New York?");
    executor
        .send_message(Message::user("What's the time and weather in New York?"))
        .await
        .unwrap();
    print_conversation(&executor);

    // Shutdown
    println!("\n=== Session Complete ===");
    let output = executor.shutdown().await.unwrap();
    println!(
        "Total turns: {}, Status: {:?}",
        output.total_turns, output.status
    );
}

fn print_conversation(executor: &SessionExecutor) {
    if let SessionState::Waiting {
        messages,
        turn_count,
        ..
    } = executor.state()
    {
        println!("\n--- Turn {} ---", turn_count);
        for msg in messages.iter().skip(messages.len().saturating_sub(5)) {
            match msg.role {
                MessageRole::User => {
                    println!("User: {}", msg.content.as_text().unwrap_or(""));
                }
                MessageRole::Assistant => {
                    let text = msg.content.as_text().unwrap_or("");
                    if let Some(tools) = &msg.tool_calls {
                        println!("Assistant: {} [calling {} tool(s)]", text, tools.len());
                        for tool in tools {
                            println!("  -> {}({})", tool.name, tool.arguments);
                        }
                    } else {
                        println!("Assistant: {}", text);
                    }
                }
                MessageRole::Tool => {
                    let tool_id = msg.tool_call_id.as_deref().unwrap_or("unknown");
                    match &msg.content {
                        MessageContent::ToolResult(result) => {
                            println!("Tool [{}]: {}", tool_id, result);
                        }
                        MessageContent::ToolError(err) => {
                            println!("Tool [{}]: ERROR - {}", tool_id, err);
                        }
                        _ => {}
                    }
                }
                MessageRole::System => {}
            }
        }
    }
}

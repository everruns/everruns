//! Task List Example - Agent Loop with Task Management Capability
//!
//! This example demonstrates how to use the TaskList capability to enable
//! agents to create and manage task lists for tracking multi-step work.
//!
//! The TaskList capability provides:
//! - `write_todos` tool for creating/updating task lists
//! - System prompt guidance on when and how to use task lists
//! - Validation of task structure (content, activeForm, status)
//!
//! Prerequisites:
//! - Set OPENAI_API_KEY environment variable
//!
//! Run with: cargo run -p everruns-core --example task_list

use everruns_core::{
    apply_capabilities,
    capabilities::{CapabilityId, CapabilityRegistry},
    config::AgentConfig,
    memory::{InMemoryEventEmitter, InMemoryMessageStore},
    message::{Message, MessageRole},
    openai::OpenAIProtocolLlmProvider,
    AgentLoop,
};
use uuid::Uuid;

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
                        if tc.name == "write_todos" {
                            // Pretty print write_todos arguments
                            if let Ok(todos) =
                                serde_json::from_value::<serde_json::Value>(tc.arguments.clone())
                            {
                                if let Some(todo_list) =
                                    todos.get("todos").and_then(|t| t.as_array())
                                {
                                    println!("       -> write_todos({} tasks):", todo_list.len());
                                    for (idx, todo) in todo_list.iter().enumerate() {
                                        let content = todo
                                            .get("content")
                                            .and_then(|c| c.as_str())
                                            .unwrap_or("?");
                                        let status = todo
                                            .get("status")
                                            .and_then(|s| s.as_str())
                                            .unwrap_or("?");
                                        let icon = match status {
                                            "completed" => "[x]",
                                            "in_progress" => "[>]",
                                            _ => "[ ]",
                                        };
                                        println!("          {} {} {}", icon, idx + 1, content);
                                    }
                                }
                            }
                        } else {
                            println!("       -> {}({})", tc.name, tc.arguments);
                        }
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
                        // Pretty print write_todos result
                        if let Some(obj) = res.as_object() {
                            if obj.contains_key("todos") {
                                let pending =
                                    obj.get("pending").and_then(|v| v.as_u64()).unwrap_or(0);
                                let in_progress =
                                    obj.get("in_progress").and_then(|v| v.as_u64()).unwrap_or(0);
                                let completed =
                                    obj.get("completed").and_then(|v| v.as_u64()).unwrap_or(0);
                                println!(
                                    "    {}. [Tool Result] Tasks: {} pending, {} in progress, {} completed",
                                    i + 1, pending, in_progress, completed
                                );
                                continue;
                            }
                        }
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

    println!("=== Task List Capability Demo (everruns-core) ===\n");

    // Example 1: Basic task list creation
    example_basic_task_list().await?;

    // Example 2: Multi-step project planning
    example_project_planning().await?;

    println!("=== Demo completed! ===");
    Ok(())
}

/// Example 1: Basic task list creation
async fn example_basic_task_list() -> anyhow::Result<()> {
    println!("--- Example 1: Basic Task List Creation ---\n");

    // Create capability registry with built-in capabilities
    let registry = CapabilityRegistry::with_builtins();

    // Base agent config with task management instructions
    let base_config = AgentConfig::new(
        "You are a helpful assistant that tracks tasks using the write_todos tool.",
        "gpt-4.1-mini",
    );

    // Apply the StatelessTodoList capability
    let capability_ids = vec![CapabilityId::STATELESS_TODO_LIST.to_string()];
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
    let user_message = "Create a task list with these items: 1) Write code, 2) Test code, 3) Deploy. Start working on the first task.";
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

/// Example 2: Multi-step project planning
async fn example_project_planning() -> anyhow::Result<()> {
    println!("--- Example 2: Multi-Step Project Planning ---\n");

    // Create capability registry
    let registry = CapabilityRegistry::with_builtins();

    // Base agent config
    let base_config = AgentConfig::new(
        "You are a project manager that breaks down complex tasks into actionable steps. Use write_todos to track all tasks.",
        "gpt-4.1-mini",
    );

    // Apply StatelessTodoList and CurrentTime capabilities
    let capability_ids = vec![
        CapabilityId::STATELESS_TODO_LIST.to_string(),
        CapabilityId::CURRENT_TIME.to_string(),
    ];
    let applied = apply_capabilities(base_config, &capability_ids, &registry);

    println!("Applied capabilities: {:?}", applied.applied_ids);
    println!("Tools available: {:?}", applied.tool_registry.tool_names());
    println!();

    // Create components
    let event_emitter = InMemoryEventEmitter::new();
    let message_store = InMemoryMessageStore::new();
    let llm_provider = OpenAIProtocolLlmProvider::from_env()?;

    // Seed with user message - a complex project request
    let session_id = Uuid::now_v7();
    let user_message = "I need to launch a new product. Help me create a task list for: \
        market research, product design, development, testing, and launch. \
        Start with market research.";
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

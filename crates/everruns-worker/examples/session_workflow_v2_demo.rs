//! Session Workflow V2 Demo - Fake LLM and Tool Execution
//!
//! This example demonstrates the v2 workflow using fake model_call and tool_calls.
//! No external services required - runs entirely in-memory.
//!
//! Run with: cargo run --example session_workflow_v2_demo -p everruns-worker

use serde_json::json;
use uuid::Uuid;

use everruns_worker::types::WorkflowAction;
use everruns_worker::v2::{SessionWorkflowV2, SessionWorkflowV2Input};
use everruns_worker::workflow_traits::Workflow;

fn main() {
    println!("=== Session Workflow V2 Demo ===\n");

    // Scenario 1: Simple text response (no tools)
    demo_simple_response();

    println!("\n{}\n", "=".repeat(50));

    // Scenario 2: Tool call flow
    demo_tool_call_flow();

    println!("\n{}\n", "=".repeat(50));

    // Scenario 3: Multiple tool iterations
    demo_multi_iteration();
}

/// Helper to find activity ID by type prefix
fn find_activity_id(actions: &[WorkflowAction], activity_type_prefix: &str) -> Option<String> {
    actions.iter().find_map(|a| {
        if let WorkflowAction::ScheduleActivity {
            activity_id,
            activity_type,
            ..
        } = a
        {
            if activity_type.starts_with(activity_type_prefix) {
                return Some(activity_id.clone());
            }
        }
        None
    })
}

fn demo_simple_response() {
    println!("--- Scenario 1: Simple Text Response ---\n");

    let input = SessionWorkflowV2Input {
        session_id: Uuid::now_v7(),
        agent_id: Uuid::now_v7(),
    };

    let mut workflow = SessionWorkflowV2::new(input);

    // Start workflow
    println!("Step 1: on_start()");
    let actions = workflow.on_start();
    print_actions(&actions);
    let load_agent_id = find_activity_id(&actions, "load-agent").unwrap();

    // Agent loaded
    println!("\nStep 2: Agent loaded");
    let actions = workflow.on_activity_completed(
        &load_agent_id,
        json!({
            "model": "gpt-4",
            "system_prompt": "You are a helpful assistant.",
            "tools": [],
            "max_iterations": 5
        }),
    );
    print_actions(&actions);
    let load_messages_id = find_activity_id(&actions, "load-messages").unwrap();

    // Messages loaded
    println!("\nStep 3: Messages loaded");
    let actions = workflow.on_activity_completed(
        &load_messages_id,
        json!([{
            "role": "user",
            "content": "What is 2+2?"
        }]),
    );
    print_actions(&actions);
    let call_model_id = find_activity_id(&actions, "call-model").unwrap();

    // LLM response (no tools)
    println!("\nStep 4: LLM responded (no tools)");
    let actions = workflow.on_activity_completed(
        &call_model_id,
        json!({
            "text": "2 + 2 equals 4.",
            "tool_calls": null
        }),
    );
    print_actions(&actions);
    let save_msg_id = find_activity_id(&actions, "save-message").unwrap();

    // Message saved
    println!("\nStep 5: Message saved");
    let actions = workflow.on_activity_completed(&save_msg_id, json!({}));
    print_actions(&actions);

    println!("\nWorkflow completed: {}", workflow.is_completed());
}

fn demo_tool_call_flow() {
    println!("--- Scenario 2: Tool Call Flow ---\n");

    let input = SessionWorkflowV2Input {
        session_id: Uuid::now_v7(),
        agent_id: Uuid::now_v7(),
    };

    let mut workflow = SessionWorkflowV2::new(input);

    // Start
    println!("Step 1: on_start()");
    let actions = workflow.on_start();
    print_actions(&actions);
    let load_agent_id = find_activity_id(&actions, "load-agent").unwrap();

    // Agent loaded with tools
    println!("\nStep 2: Agent loaded (with tools)");
    let actions = workflow.on_activity_completed(
        &load_agent_id,
        json!({
            "model": "gpt-4",
            "tools": [{
                "name": "get_current_time",
                "description": "Get current time in a timezone",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "timezone": {"type": "string"}
                    }
                }
            }],
            "max_iterations": 5
        }),
    );
    print_actions(&actions);
    let load_messages_id = find_activity_id(&actions, "load-messages").unwrap();

    // Messages loaded
    println!("\nStep 3: Messages loaded");
    let actions = workflow.on_activity_completed(
        &load_messages_id,
        json!([{
            "role": "user",
            "content": "What time is it in Tokyo?"
        }]),
    );
    print_actions(&actions);
    let call_model_id = find_activity_id(&actions, "call-model").unwrap();

    // LLM returns tool call
    println!("\nStep 4: LLM requested tool call");
    let actions = workflow.on_activity_completed(
        &call_model_id,
        json!({
            "text": "Let me check the current time in Tokyo.",
            "tool_calls": [{
                "id": "call_abc123",
                "name": "get_current_time",
                "arguments": {"timezone": "Asia/Tokyo"}
            }]
        }),
    );
    print_actions(&actions);
    let exec_tools_id = find_activity_id(&actions, "execute-tools").unwrap();

    // Tool executed
    println!("\nStep 5: Tool executed");
    let actions = workflow.on_activity_completed(
        &exec_tools_id,
        json!([{
            "tool_call_id": "call_abc123",
            "result": {"time": "2024-01-15T14:30:00+09:00", "timezone": "Asia/Tokyo"}
        }]),
    );
    print_actions(&actions);
    let call_model_id2 = find_activity_id(&actions, "call-model").unwrap();

    // Second LLM call (with tool result)
    println!("\nStep 6: LLM final response");
    let actions = workflow.on_activity_completed(
        &call_model_id2,
        json!({
            "text": "The current time in Tokyo is 2:30 PM (14:30).",
            "tool_calls": null
        }),
    );
    print_actions(&actions);
    let save_msg_id = find_activity_id(&actions, "save-message").unwrap();

    // Save and complete
    println!("\nStep 7: Message saved");
    let actions = workflow.on_activity_completed(&save_msg_id, json!({}));
    print_actions(&actions);

    println!("\nWorkflow completed: {}", workflow.is_completed());
}

fn demo_multi_iteration() {
    println!("--- Scenario 3: Multiple Tool Iterations ---\n");

    let input = SessionWorkflowV2Input {
        session_id: Uuid::now_v7(),
        agent_id: Uuid::now_v7(),
    };

    let mut workflow = SessionWorkflowV2::new(input);

    // Start
    println!("Step 1: on_start()");
    let actions = workflow.on_start();
    print_actions(&actions);
    let load_agent_id = find_activity_id(&actions, "load-agent").unwrap();

    // Agent loaded
    println!("\nStep 2: Agent loaded");
    let actions = workflow.on_activity_completed(
        &load_agent_id,
        json!({
            "model": "gpt-4",
            "tools": [
                {"name": "get_weather", "description": "Get weather", "parameters": {}},
                {"name": "calculate", "description": "Calculate", "parameters": {}}
            ],
            "max_iterations": 10
        }),
    );
    print_actions(&actions);
    let load_messages_id = find_activity_id(&actions, "load-messages").unwrap();

    // Messages loaded
    println!("\nStep 3: Messages loaded");
    let actions = workflow.on_activity_completed(
        &load_messages_id,
        json!([{
            "role": "user",
            "content": "Search for weather in NYC and then calculate something"
        }]),
    );
    print_actions(&actions);
    let call_model_id = find_activity_id(&actions, "call-model").unwrap();

    // First LLM call - requests weather tool
    println!("\nStep 4: LLM requests first tool (weather)");
    let actions = workflow.on_activity_completed(
        &call_model_id,
        json!({
            "text": "Getting weather...",
            "tool_calls": [{
                "id": "call_1",
                "name": "get_weather",
                "arguments": {"city": "NYC"}
            }]
        }),
    );
    print_actions(&actions);
    let exec_tools_id = find_activity_id(&actions, "execute-tools").unwrap();

    // First tool executed
    println!("\nStep 5: Weather tool executed");
    let actions = workflow.on_activity_completed(
        &exec_tools_id,
        json!([{
            "tool_call_id": "call_1",
            "result": {"temp": 45, "conditions": "cloudy"}
        }]),
    );
    print_actions(&actions);
    let call_model_id2 = find_activity_id(&actions, "call-model").unwrap();

    // Second LLM call - requests calculate tool
    println!("\nStep 6: LLM requests second tool (calculate)");
    let actions = workflow.on_activity_completed(
        &call_model_id2,
        json!({
            "text": "Now calculating...",
            "tool_calls": [{
                "id": "call_2",
                "name": "calculate",
                "arguments": {"expression": "45 * 1.8 + 32"}
            }]
        }),
    );
    print_actions(&actions);
    let exec_tools_id2 = find_activity_id(&actions, "execute-tools").unwrap();

    // Second tool executed
    println!("\nStep 7: Calculate tool executed");
    let actions = workflow.on_activity_completed(
        &exec_tools_id2,
        json!([{
            "tool_call_id": "call_2",
            "result": {"value": 113}
        }]),
    );
    print_actions(&actions);
    let call_model_id3 = find_activity_id(&actions, "call-model").unwrap();

    // Final LLM response
    println!("\nStep 8: LLM final response");
    let actions = workflow.on_activity_completed(
        &call_model_id3,
        json!({
            "text": "The weather in NYC is 45°F (about 7°C). It's cloudy.",
            "tool_calls": null
        }),
    );
    print_actions(&actions);
    let save_msg_id = find_activity_id(&actions, "save-message").unwrap();

    // Save
    println!("\nStep 9: Message saved");
    let actions = workflow.on_activity_completed(&save_msg_id, json!({}));
    print_actions(&actions);

    println!("\nWorkflow completed: {}", workflow.is_completed());
}

fn print_actions(actions: &[WorkflowAction]) {
    if actions.is_empty() {
        println!("  (no actions)");
        return;
    }
    for action in actions {
        match action {
            WorkflowAction::ScheduleActivity {
                activity_id,
                activity_type,
                ..
            } => {
                println!("  → Schedule: {} ({})", activity_type, activity_id);
            }
            WorkflowAction::CompleteWorkflow { result } => {
                println!(
                    "  ✓ Complete: {}",
                    result.as_ref().map(|r| r.to_string()).unwrap_or_default()
                );
            }
            WorkflowAction::FailWorkflow { reason } => {
                println!("  ✗ Fail: {}", reason);
            }
            WorkflowAction::None => {}
        }
    }
}

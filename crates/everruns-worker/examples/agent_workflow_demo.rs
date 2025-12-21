//! Agent Workflow Demo - State Machine Simulation
//!
//! This example demonstrates the agent workflow state machine logic.
//! It simulates workflow execution without requiring Temporal or database.
//!
//! For real Temporal execution, use the smoke tests or API endpoints.
//!
//! Run with: cargo run --example agent_workflow_demo -p everruns-worker

use serde_json::json;
use uuid::Uuid;

use everruns_worker::types::WorkflowAction;
use everruns_worker::{AgentWorkflow, AgentWorkflowInput};
use everruns_worker::Workflow;

fn main() {
    println!("=== Agent Workflow Demo ===\n");
    println!("This demo simulates the agent workflow state machine.\n");
    println!("Agent workflow design:");
    println!("  - Atoms handle message storage internally (no messages in state)");
    println!("  - Each tool call is a separate activity for visibility");
    println!("  - Workflow is a lightweight orchestrator\n");

    // Scenario 1: Simple text response (no tools)
    demo_simple_response();

    println!("\n{}\n", "=".repeat(60));

    // Scenario 2: Single tool call
    demo_tool_call();

    println!("\n{}\n", "=".repeat(60));

    // Scenario 3: Multiple parallel tool calls
    demo_parallel_tools();
}

fn demo_simple_response() {
    println!("--- Scenario 1: Simple Response (No Tools) ---\n");

    let input = AgentWorkflowInput {
        session_id: Uuid::now_v7(),
        agent_id: Uuid::now_v7(),
    };

    let mut workflow = AgentWorkflow::new(input);

    // Step 1: Start workflow
    println!("1. on_start()");
    let actions = workflow.on_start();
    print_actions(&actions);
    let load_agent_id = find_activity_id(&actions, "load-agent").unwrap();

    // Step 2: Agent loaded → model call
    println!("\n2. Agent loaded → schedule call-model");
    let actions = workflow.on_activity_completed(
        &load_agent_id,
        json!({
            "model": "gpt-5.2",
            "system_prompt": "You are a helpful assistant.",
            "tools": [],
            "max_iterations": 5
        }),
    );
    print_actions(&actions);
    let call_model_id = find_activity_id(&actions, "call-model").unwrap();

    // Step 3: LLM response (no tools) → complete
    println!("\n3. LLM responded (no tools) → complete");
    let actions = workflow.on_activity_completed(
        &call_model_id,
        json!({
            "text": "2 + 2 = 4",
            "tool_calls": null,
            "needs_tool_execution": false
        }),
    );
    print_actions(&actions);
    println!("\nWorkflow completed: {}", workflow.is_completed());
}

fn demo_tool_call() {
    println!("--- Scenario 2: Tool Call Flow ---\n");

    let input = AgentWorkflowInput {
        session_id: Uuid::now_v7(),
        agent_id: Uuid::now_v7(),
    };

    let mut workflow = AgentWorkflow::new(input);

    // Start
    let actions = workflow.on_start();
    let load_agent_id = find_activity_id(&actions, "load-agent").unwrap();

    // Agent loaded with tool
    println!("1. Agent loaded (with get_time tool)");
    let actions = workflow.on_activity_completed(
        &load_agent_id,
        json!({
            "model": "gpt-5.2",
            "tools": [{
                "name": "get_time",
                "description": "Get current time",
                "parameters": {}
            }],
            "max_iterations": 5
        }),
    );
    print_actions(&actions);
    let call_model_id = find_activity_id(&actions, "call-model").unwrap();

    // LLM requests tool
    println!("\n2. LLM requests tool call");
    let actions = workflow.on_activity_completed(
        &call_model_id,
        json!({
            "text": "Let me check the time.",
            "tool_calls": [{
                "id": "call_123",
                "name": "get_time",
                "arguments": {}
            }],
            "needs_tool_execution": true
        }),
    );
    print_actions(&actions);
    let exec_tool_id = find_activity_id(&actions, "execute-tool").unwrap();

    // Tool executed
    println!("\n3. Tool executed → next model call");
    let actions = workflow.on_activity_completed(
        &exec_tool_id,
        json!({
            "result": {
                "tool_call_id": "call_123",
                "result": {"time": "14:30"},
                "error": null
            }
        }),
    );
    print_actions(&actions);
    let call_model_id2 = find_activity_id(&actions, "call-model").unwrap();

    // Final response
    println!("\n4. LLM final response → complete");
    let actions = workflow.on_activity_completed(
        &call_model_id2,
        json!({
            "text": "The current time is 14:30.",
            "tool_calls": null,
            "needs_tool_execution": false
        }),
    );
    print_actions(&actions);
    println!("\nWorkflow completed: {}", workflow.is_completed());
}

fn demo_parallel_tools() {
    println!("--- Scenario 3: Parallel Tool Calls ---\n");

    let input = AgentWorkflowInput {
        session_id: Uuid::now_v7(),
        agent_id: Uuid::now_v7(),
    };

    let mut workflow = AgentWorkflow::new(input);

    // Start and load agent
    let actions = workflow.on_start();
    let load_agent_id = find_activity_id(&actions, "load-agent").unwrap();

    let actions = workflow.on_activity_completed(
        &load_agent_id,
        json!({
            "model": "gpt-5.2",
            "tools": [
                {"name": "get_weather", "description": "Get weather", "parameters": {}},
                {"name": "get_time", "description": "Get time", "parameters": {}}
            ],
            "max_iterations": 5
        }),
    );
    let call_model_id = find_activity_id(&actions, "call-model").unwrap();

    // LLM requests TWO tool calls at once
    println!("1. LLM requests 2 tools in parallel");
    let actions = workflow.on_activity_completed(
        &call_model_id,
        json!({
            "text": "Let me check both.",
            "tool_calls": [
                {"id": "call_1", "name": "get_weather", "arguments": {"city": "NYC"}},
                {"id": "call_2", "name": "get_time", "arguments": {}}
            ],
            "needs_tool_execution": true
        }),
    );
    print_actions(&actions);
    let tool_ids: Vec<_> = actions
        .iter()
        .filter_map(|a| {
            if let WorkflowAction::ScheduleActivity { activity_id, .. } = a {
                Some(activity_id.clone())
            } else {
                None
            }
        })
        .collect();
    println!("   ({} parallel activities scheduled)\n", tool_ids.len());

    // First tool completes
    println!("2. First tool completes (still waiting)");
    let actions = workflow.on_activity_completed(
        &tool_ids[0],
        json!({
            "result": {
                "tool_call_id": "call_1",
                "result": {"temp": 72},
                "error": null
            }
        }),
    );
    print_actions(&actions);
    println!("   (waiting for remaining tool)\n");

    // Second tool completes
    println!("3. Second tool completes → next model call");
    let actions = workflow.on_activity_completed(
        &tool_ids[1],
        json!({
            "result": {
                "tool_call_id": "call_2",
                "result": {"time": "14:30"},
                "error": null
            }
        }),
    );
    print_actions(&actions);
    let call_model_id2 = find_activity_id(&actions, "call-model").unwrap();

    // Final response
    println!("\n4. LLM final response → complete");
    let actions = workflow.on_activity_completed(
        &call_model_id2,
        json!({
            "text": "It's 72°F in NYC and the time is 14:30.",
            "tool_calls": null,
            "needs_tool_execution": false
        }),
    );
    print_actions(&actions);
    println!("\nWorkflow completed: {}", workflow.is_completed());
}

// Helper: Find activity ID by type prefix
fn find_activity_id(actions: &[WorkflowAction], prefix: &str) -> Option<String> {
    actions.iter().find_map(|a| {
        if let WorkflowAction::ScheduleActivity {
            activity_id,
            activity_type,
            ..
        } = a
        {
            if activity_type.starts_with(prefix) {
                return Some(activity_id.clone());
            }
        }
        None
    })
}

// Helper: Print actions
fn print_actions(actions: &[WorkflowAction]) {
    if actions.is_empty() {
        println!("   (no actions)");
        return;
    }
    for action in actions {
        match action {
            WorkflowAction::ScheduleActivity {
                activity_id,
                activity_type,
                ..
            } => {
                println!("   → Schedule: {} ({})", activity_type, activity_id);
            }
            WorkflowAction::CompleteWorkflow { .. } => {
                println!("   ✓ CompleteWorkflow");
            }
            WorkflowAction::FailWorkflow { reason } => {
                println!("   ✗ FailWorkflow: {}", reason);
            }
            WorkflowAction::None => {}
        }
    }
}

// Workflow step implementations
//
// Steps are the units of work scheduled by the durable workflow orchestrator.
// Each step runs as a task and returns a result.
// Decision: Workers communicate with control-plane via gRPC for all operations.
//
// These implementations use Atoms from everruns-core for the actual work:
// - InputAtom: Retrieves user input message
// - ReasonAtom: LLM call with context preparation
// - ActAtom: Parallel tool execution
//
// Atoms handle message loading/storage internally via MessageStore trait.
// Atoms emit events via EventEmitter for observability.

use anyhow::{Context, Result};
use everruns_core::atoms::{ActAtom, Atom, InputAtom, ReasonAtom};
use everruns_core::capabilities::CapabilityRegistry;
use everruns_core::ToolRegistry;
use std::sync::Arc;

use crate::adapters::create_driver_registry;
use crate::grpc_adapters::{
    GrpcAgentStore, GrpcClient, GrpcEventEmitter, GrpcLlmProviderStore, GrpcMessageStore,
    GrpcSessionFileStore, GrpcSessionStore,
};

// Re-export atom types for activity callers
pub use everruns_core::atoms::{
    ActInput, ActResult, InputAtomInput, InputAtomResult, ReasonInput, ReasonResult, ToolCallResult,
};

// ============================================================================
// Activity Implementations
// ============================================================================

/// Process user input using InputAtom
///
/// This activity:
/// 1. Sets session status to "active" and emits session.activated event
/// 2. Emits turn.started event
/// 3. Retrieves the user message from the message store (emits input.received event)
/// 4. Returns the message for downstream processing
pub async fn input_activity(
    grpc_client: GrpcClient,
    input: InputAtomInput,
) -> Result<InputAtomResult> {
    use everruns_core::events::{
        EventContext, EventRequest, SessionActivatedData, TurnStartedData,
    };
    use everruns_core::traits::EventEmitter;

    tracing::info!(
        session_id = %input.context.session_id,
        turn_id = %input.context.turn_id,
        input_message_id = %input.context.input_message_id,
        "Executing input_activity"
    );

    // Set session status to "active" - turn is starting
    if let Err(e) = grpc_client
        .set_session_status(input.context.session_id, "active")
        .await
    {
        tracing::warn!(error = %e, "Failed to set session status to active");
    }

    // Emit session.activated event
    let event_emitter = GrpcEventEmitter::new(grpc_client.clone());
    let activated_event = EventRequest::new(
        input.context.session_id,
        EventContext::turn(input.context.turn_id, input.context.input_message_id),
        SessionActivatedData {
            turn_id: input.context.turn_id,
            input_message_id: input.context.input_message_id,
        },
    );
    if let Err(e) = event_emitter.emit(activated_event).await {
        tracing::warn!(error = %e, "Failed to emit session.activated event");
    }

    // Emit turn.started event
    let turn_started_event = EventRequest::new(
        input.context.session_id,
        EventContext::turn(input.context.turn_id, input.context.input_message_id),
        TurnStartedData {
            turn_id: input.context.turn_id,
            input_message_id: input.context.input_message_id,
        },
    );
    if let Err(e) = event_emitter.emit(turn_started_event).await {
        tracing::warn!(error = %e, "Failed to emit turn.started event");
    }

    let message_store = GrpcMessageStore::new(grpc_client.clone());
    let atom = InputAtom::new(message_store, event_emitter);

    atom.execute(input)
        .await
        .context("InputAtom execution failed")
}

/// Call the LLM model for reasoning using ReasonAtom
///
/// This activity:
/// 1. Emits reason.started event
/// 2. Retrieves agent and session configuration
/// 3. Loads messages and prepares context
/// 4. Calls the LLM with the messages
/// 5. Stores the assistant response
/// 6. Emits reason.completed event
/// 7. Returns the result with tool calls (if any)
/// 8. If turn completes (no tool calls), emits turn.completed, sets session status to "idle" and emits session.idled
///
/// Note: API key decryption is handled by the control-plane gRPC service.
pub async fn reason_activity(grpc_client: GrpcClient, input: ReasonInput) -> Result<ReasonResult> {
    use everruns_core::events::{
        EventContext, EventRequest, SessionIdledData, TurnCompletedData, TurnFailedData,
    };
    use everruns_core::traits::EventEmitter;

    tracing::info!(
        session_id = %input.context.session_id,
        turn_id = %input.context.turn_id,
        agent_id = %input.agent_id,
        "Executing reason_activity"
    );

    let session_id = input.context.session_id;
    let turn_id = input.context.turn_id;
    let input_message_id = input.context.input_message_id;

    // Create atom dependencies using gRPC adapters
    let agent_store = GrpcAgentStore::new(grpc_client.clone());
    let session_store = GrpcSessionStore::new(grpc_client.clone());
    let message_store = GrpcMessageStore::new(grpc_client.clone());
    let provider_store = GrpcLlmProviderStore::new(grpc_client.clone());
    let capability_registry = CapabilityRegistry::with_builtins();
    let driver_registry = create_driver_registry();
    let event_emitter = GrpcEventEmitter::new(grpc_client.clone());

    let atom = ReasonAtom::new(
        agent_store,
        session_store,
        message_store,
        provider_store,
        capability_registry,
        driver_registry,
        event_emitter,
    );

    let result = atom
        .execute(input)
        .await
        .context("ReasonAtom execution failed")?;

    // If turn is complete (no tool calls, or failure), set session to idle
    let turn_complete = !result.has_tool_calls || !result.success;
    if turn_complete {
        // Set session status to "idle"
        if let Err(e) = grpc_client.set_session_status(session_id, "idle").await {
            tracing::warn!(error = %e, "Failed to set session status to idle");
        }

        let event_emitter = GrpcEventEmitter::new(grpc_client.clone());

        // Emit turn.failed or turn.completed based on success
        if !result.success {
            // Emit turn.failed event with sanitized error message
            let turn_failed_event = EventRequest::new(
                session_id,
                EventContext::turn(turn_id, input_message_id),
                TurnFailedData {
                    turn_id,
                    error: "An error occurred while processing your request.".to_string(),
                    error_code: Some("llm_error".to_string()),
                },
            );
            if let Err(e) = event_emitter.emit(turn_failed_event).await {
                tracing::warn!(error = %e, "Failed to emit turn.failed event");
            }
        } else {
            // Emit turn.completed event
            let turn_completed_event = EventRequest::new(
                session_id,
                EventContext::turn(turn_id, input_message_id),
                TurnCompletedData {
                    turn_id,
                    iterations: 1, // TODO: Track actual iterations when workflow supports it
                    duration_ms: None,
                },
            );
            if let Err(e) = event_emitter.emit(turn_completed_event).await {
                tracing::warn!(error = %e, "Failed to emit turn.completed event");
            }
        }

        // Emit session.idled event
        let idled_event = EventRequest::new(
            session_id,
            EventContext::turn(turn_id, input_message_id),
            SessionIdledData {
                turn_id,
                iterations: None, // We don't track iterations in the activity
            },
        );
        if let Err(e) = event_emitter.emit(idled_event).await {
            tracing::warn!(error = %e, "Failed to emit session.idled event");
        }
    }

    Ok(result)
}

/// Execute tools in parallel using ActAtom
///
/// This activity:
/// 1. Emits act.started event
/// 2. Executes all tool calls in parallel (emitting tool.call_started/completed for each)
/// 3. Handles errors, timeouts, and cancellations gracefully
/// 4. Stores tool result messages
/// 5. Emits act.completed event
/// 6. Returns comprehensive results for all tools
pub async fn act_activity(grpc_client: GrpcClient, input: ActInput) -> Result<ActResult> {
    tracing::info!(
        session_id = %input.context.session_id,
        turn_id = %input.context.turn_id,
        tool_count = %input.tool_calls.len(),
        "Executing act_activity"
    );

    let tool_executor = ToolRegistry::with_defaults();
    let event_emitter = GrpcEventEmitter::new(grpc_client.clone());
    let file_store = Arc::new(GrpcSessionFileStore::new(grpc_client));

    let atom = ActAtom::with_file_store(tool_executor, event_emitter, file_store);

    atom.execute(input)
        .await
        .context("ActAtom execution failed")
}

// ============================================================================
// Activity Type Constants
// ============================================================================

/// Activity type constants for workflow scheduling
pub mod activity_types {
    pub const INPUT: &str = "input";
    pub const REASON: &str = "reason";
    pub const ACT: &str = "act";
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_core::atoms::AtomContext;
    use everruns_core::{BuiltinTool, ToolCall, ToolDefinition, ToolPolicy};
    use serde_json::json;
    use uuid::Uuid;

    #[test]
    fn test_input_atom_input_serialization() {
        let context = AtomContext::new(
            Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap(),
            Uuid::parse_str("660e8400-e29b-41d4-a716-446655440000").unwrap(),
            Uuid::parse_str("770e8400-e29b-41d4-a716-446655440000").unwrap(),
        );

        let input = InputAtomInput { context };

        let json = serde_json::to_string(&input).unwrap();
        let parsed: InputAtomInput = serde_json::from_str(&json).unwrap();
        assert_eq!(
            parsed.context.session_id,
            Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap()
        );
    }

    #[test]
    fn test_reason_input_serialization() {
        let context = AtomContext::new(Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());

        let input = ReasonInput {
            context: context.clone(),
            agent_id: Uuid::parse_str("880e8400-e29b-41d4-a716-446655440000").unwrap(),
        };

        let json = serde_json::to_string(&input).unwrap();
        let parsed: ReasonInput = serde_json::from_str(&json).unwrap();
        assert_eq!(
            parsed.agent_id,
            Uuid::parse_str("880e8400-e29b-41d4-a716-446655440000").unwrap()
        );
    }

    #[test]
    fn test_act_input_serialization() {
        let context = AtomContext::new(Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());

        let input = ActInput {
            context: context.clone(),
            agent_id: Uuid::now_v7(),
            tool_calls: vec![ToolCall {
                id: "call_1".to_string(),
                name: "get_weather".to_string(),
                arguments: json!({"city": "NYC"}),
            }],
            tool_definitions: vec![ToolDefinition::Builtin(BuiltinTool {
                name: "get_weather".to_string(),
                description: "Get weather".to_string(),
                parameters: json!({}),
                policy: ToolPolicy::Auto,
            })],
        };

        let json = serde_json::to_string(&input).unwrap();
        let parsed: ActInput = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.tool_calls.len(), 1);
        assert_eq!(parsed.tool_calls[0].name, "get_weather");
    }

    #[test]
    fn test_reason_result_serialization() {
        let result = ReasonResult {
            success: true,
            text: "Hello!".to_string(),
            tool_calls: vec![],
            has_tool_calls: false,
            tool_definitions: vec![],
            max_iterations: 10,
            error: None,
        };

        let json = serde_json::to_string(&result).unwrap();
        let parsed: ReasonResult = serde_json::from_str(&json).unwrap();
        assert!(parsed.success);
        assert_eq!(parsed.text, "Hello!");
    }

    #[test]
    fn test_act_result_serialization() {
        let result = ActResult {
            results: vec![ToolCallResult {
                tool_call: ToolCall {
                    id: "call_1".to_string(),
                    name: "get_weather".to_string(),
                    arguments: json!({}),
                },
                result: everruns_core::ToolResult {
                    tool_call_id: "call_1".to_string(),
                    result: Some(json!({"temp": 72})),
                    error: None,
                },
                success: true,
                status: "success".to_string(),
            }],
            completed: true,
            success_count: 1,
            error_count: 0,
        };

        let json = serde_json::to_string(&result).unwrap();
        let parsed: ActResult = serde_json::from_str(&json).unwrap();
        assert!(parsed.completed);
        assert_eq!(parsed.success_count, 1);
    }
}

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
// Atoms handle message loading internally via MessageRetriever trait.
// Atoms emit events via EventEmitter for observability.

use anyhow::{Context, Result};
use everruns_host::HostComposition;
use everruns_host::{
    execute_act_activity as runtime_execute_act_activity,
    execute_input_activity as runtime_execute_input_activity,
    execute_reason_activity as runtime_execute_reason_activity,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::grpc_adapters::GrpcClient;
use crate::grpc_worker_adapters::GrpcWorkerAdapters;
use crate::runtime_host::WorkerRuntimeHost;

// Re-export atom types for activity callers
pub use everruns_engine::{
    ActInput, ActResult, InputAtomInput, InputAtomResult, ReasonInput, ReasonResult, ToolCallResult,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledAppChannelInput {
    pub org_id: i64,
    pub app_id: String,
    pub channel_id: String,
}

/// Durable schedule target input for an agent-owned schedule trigger (EVE-757).
/// Mirrors [`ScheduledAppChannelInput`], re-homed on the agent + trigger.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledAgentTriggerInput {
    pub org_id: i64,
    pub agent_id: String,
    pub trigger_id: String,
}

// ============================================================================
// Activity Implementations
// ============================================================================

/// Process user input using InputAtom
///
/// This activity:
/// 1. Sets session status to "active" and emits session.activated event
/// 2. Emits turn.started event
/// 3. Retrieves the user message from the message store
/// 4. Returns the message for downstream processing
pub async fn input_activity(
    grpc_client: GrpcClient,
    org_id: i64,
    input: InputAtomInput,
) -> Result<InputAtomResult> {
    tracing::info!(
        org_id = org_id,
        session_id = %input.context.session_id,
        turn_id = %input.context.turn_id,
        input_message_id = %input.context.input_message_id,
        "Executing input_activity"
    );

    runtime_execute_input_activity(
        &WorkerRuntimeHost::new(GrpcWorkerAdapters::from_client(grpc_client)),
        org_id,
        input,
    )
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
pub async fn reason_activity(
    grpc_client: GrpcClient,
    org_id: i64,
    input: ReasonInput,
    host_composition: &HostComposition,
    stream_heartbeater: Option<Arc<dyn everruns_core::durability::StreamHeartbeater>>,
) -> Result<ReasonResult> {
    tracing::info!(
        org_id = org_id,
        session_id = %input.context.session_id,
        turn_id = %input.context.turn_id,
        agent_id = ?input.agent_id,
        "Executing reason_activity"
    );

    let mut adapters = GrpcWorkerAdapters::from_client_with_host_composition(
        grpc_client,
        host_composition.clone(),
    );
    if let Some(hb) = stream_heartbeater {
        adapters = adapters.with_stream_heartbeater(hb);
    }
    let result = runtime_execute_reason_activity(&WorkerRuntimeHost::new(adapters), org_id, input)
        .await
        .context("ReasonAtom execution failed")?;

    // Turn lifecycle events (turn.completed, turn.failed, session.idled) are NOT
    // emitted here. They are deferred to the workflow scheduler which checks for
    // pending steering signals (mid-turn user messages) before deciding whether
    // the turn is truly done. This ensures turn.completed is emitted exactly once
    // and prevents the idle→active flicker when steering continues the turn.

    Ok(result)
}

/// Execute tools in parallel using ActAtom
///
/// This activity:
/// 1. Emits act.started event
/// 2. Executes all tool calls in parallel (emitting tool.started/completed for each)
/// 3. Handles errors, timeouts, and cancellations gracefully
/// 4. Stores tool result messages
/// 5. Emits act.completed event
/// 6. Returns comprehensive results for all tools
///
/// Supports both built-in tools and MCP tools (via remote MCP servers).
pub async fn act_activity(
    grpc_client: GrpcClient,
    org_id: i64,
    input: ActInput,
    host_composition: &HostComposition,
) -> Result<ActResult> {
    tracing::info!(
        org_id = org_id,
        session_id = %input.context.session_id,
        turn_id = %input.context.turn_id,
        tool_count = %input.tool_calls.len(),
        "Executing act_activity"
    );

    runtime_execute_act_activity(
        &WorkerRuntimeHost::new(GrpcWorkerAdapters::from_client_with_host_composition(
            grpc_client,
            host_composition.clone(),
        )),
        input,
    )
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
    pub const INVOKE_SCHEDULED_APP_CHANNEL: &str = "invoke_scheduled_app_channel";
    pub const INVOKE_AGENT_TRIGGER: &str = "invoke_agent_trigger";
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_core::ExecutionContext;
    use everruns_provider::tool_types::{
        BuiltinTool, DeferrablePolicy, ToolCall, ToolDefinition, ToolPolicy,
    };
    use everruns_provider::typed_id::{AgentId, HarnessId, MessageId, SessionId, TurnId};
    use serde_json::json;
    use uuid::Uuid;

    #[test]
    fn durable_activity_io_is_the_engine_contract() {
        assert_eq!(
            std::any::type_name::<ActInput>(),
            std::any::type_name::<everruns_engine::ActInput>(),
        );
        assert_eq!(
            std::any::type_name::<ReasonResult>(),
            std::any::type_name::<everruns_engine::ReasonResult>(),
        );
    }

    #[test]
    fn test_input_atom_input_serialization() {
        let context = ExecutionContext::new(
            SessionId::from_uuid(Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap()),
            TurnId::from_uuid(Uuid::parse_str("660e8400-e29b-41d4-a716-446655440000").unwrap()),
            MessageId::from_uuid(Uuid::parse_str("770e8400-e29b-41d4-a716-446655440000").unwrap()),
        );

        let input = InputAtomInput { context };

        let json = serde_json::to_string(&input).unwrap();
        let parsed: InputAtomInput = serde_json::from_str(&json).unwrap();
        assert_eq!(
            parsed.context.session_id.uuid(),
            Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap()
        );
    }

    #[test]
    fn test_reason_input_serialization() {
        let context = ExecutionContext::new(SessionId::new(), TurnId::new(), MessageId::new());

        let input = ReasonInput {
            context: context.clone(),
            agent_id: Some(AgentId::from_uuid(
                Uuid::parse_str("880e8400-e29b-41d4-a716-446655440000").unwrap(),
            )),
            harness_id: HarnessId::from_uuid(
                Uuid::parse_str("990e8400-e29b-41d4-a716-446655440000").unwrap(),
            ),
            org_id: 1,
            mcp_tool_definitions: vec![],
            previous_response_id: None,
            iteration: 1,
        };

        let json = serde_json::to_string(&input).unwrap();
        let parsed: ReasonInput = serde_json::from_str(&json).unwrap();
        assert_eq!(
            parsed.agent_id.unwrap().uuid(),
            Uuid::parse_str("880e8400-e29b-41d4-a716-446655440000").unwrap()
        );
    }

    #[test]
    fn test_act_input_serialization() {
        let context = ExecutionContext::new(SessionId::new(), TurnId::new(), MessageId::new());

        let input = ActInput {
            org_id: Some(1),
            context: context.clone(),
            agent_id: Some(AgentId::new()),
            harness_id: HarnessId::new(),
            tool_calls: vec![ToolCall {
                id: "call_1".to_string(),
                name: "get_weather".to_string(),
                arguments: json!({"city": "NYC"}),
            }],
            tool_definitions: vec![ToolDefinition::Builtin(BuiltinTool {
                name: "get_weather".to_string(),
                display_name: None,
                description: "Get weather".to_string(),
                parameters: json!({}),
                policy: ToolPolicy::Auto,
                category: None,
                deferrable: DeferrablePolicy::default(),
                hints: everruns_provider::tool_types::ToolHints::default(),
                full_parameters: None,
            })],
            locale: None,
            blueprint_id: None,
            network_access: None,
            parallel_tool_calls: None,
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
            user_facing_error: None,
            error_disclosure: None,
            usage: None,
            output_message_id: None,
            time_to_first_token_ms: None,
            response_id: None,
            finish_reason: Some("stop".to_string()),
            locale: None,
            network_access: None,
            parallel_tool_calls: None,
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
                result: everruns_provider::tool_types::ToolResult {
                    tool_call_id: "call_1".to_string(),
                    result: Some(json!({"temp": 72})),
                    images: None,
                    error: None,
                    connection_required: None,
                    raw_output: None,
                },
                success: true,
                status: "success".to_string(),
                connection_required: None,
                determinism_fatal: None,
            }],
            completed: true,
            success_count: 1,
            error_count: 0,
            waiting_for_tool_results: false,
            blocked: false,
            client_tool_calls: vec![],
            client_tool_definitions: vec![],
        };

        let json = serde_json::to_string(&result).unwrap();
        let parsed: ActResult = serde_json::from_str(&json).unwrap();
        assert!(parsed.completed);
        assert_eq!(parsed.success_count, 1);
    }
}

// Activity implementations for workflow execution
//
// Activities are the units of work scheduled by the workflow.
// Each activity runs outside the workflow and returns a result.
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
use everruns_core::llm_driver_registry::DriverRegistry;
use everruns_core::ToolRegistry;
use everruns_storage::{repositories::Database, DbEventEmitter, EncryptionService};
use std::sync::Arc;

use crate::adapters::{
    DbAgentStore, DbLlmProviderStore, DbMessageStore, DbSessionFileStore, DbSessionStore,
};

/// Create and configure the driver registry with all supported LLM providers
///
/// This registers drivers for:
/// - OpenAI (and Azure OpenAI)
/// - Anthropic Claude
fn create_driver_registry() -> DriverRegistry {
    let mut registry = DriverRegistry::new();
    everruns_openai::register_driver(&mut registry);
    everruns_anthropic::register_driver(&mut registry);
    registry
}

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
/// 1. Emits input.started event
/// 2. Retrieves the user message from the message store
/// 3. Emits input.completed event
/// 4. Returns the message for downstream processing
pub async fn input_activity(db: Database, input: InputAtomInput) -> Result<InputAtomResult> {
    tracing::info!(
        session_id = %input.context.session_id,
        turn_id = %input.context.turn_id,
        input_message_id = %input.context.input_message_id,
        "Executing input_activity"
    );

    let message_store = DbMessageStore::new(db.clone());
    let event_emitter = DbEventEmitter::new(db);
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
pub async fn reason_activity(
    db: Database,
    encryption: EncryptionService,
    input: ReasonInput,
) -> Result<ReasonResult> {
    tracing::info!(
        session_id = %input.context.session_id,
        turn_id = %input.context.turn_id,
        agent_id = %input.agent_id,
        "Executing reason_activity"
    );

    // Create atom dependencies
    let agent_store = DbAgentStore::new(db.clone());
    let session_store = DbSessionStore::new(db.clone());
    let message_store = DbMessageStore::new(db.clone());
    let provider_store = DbLlmProviderStore::new(db.clone(), encryption);
    let capability_registry = CapabilityRegistry::with_builtins();
    let driver_registry = create_driver_registry();
    let event_emitter = DbEventEmitter::new(db);

    let atom = ReasonAtom::new(
        agent_store,
        session_store,
        message_store,
        provider_store,
        capability_registry,
        driver_registry,
        event_emitter,
    );

    atom.execute(input)
        .await
        .context("ReasonAtom execution failed")
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
pub async fn act_activity(db: Database, input: ActInput) -> Result<ActResult> {
    tracing::info!(
        session_id = %input.context.session_id,
        turn_id = %input.context.turn_id,
        tool_count = %input.tool_calls.len(),
        "Executing act_activity"
    );

    let message_store = DbMessageStore::new(db.clone());
    let tool_executor = ToolRegistry::with_defaults();
    let event_emitter = DbEventEmitter::new(db.clone());
    let file_store = Arc::new(DbSessionFileStore::new(db));

    let atom = ActAtom::with_file_store(message_store, tool_executor, event_emitter, file_store);

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

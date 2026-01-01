// Activity implementations for workflow execution
//
// Activities are the units of work scheduled by the workflow.
// Each activity runs outside the workflow and returns a result.
//
// These implementations use Atoms from everruns-core for the actual work:
// - Legacy atoms: CallModelAtom, ExecuteToolAtom
// - New turn-based atoms: InputAtom, ReasonAtom, ActAtom
//
// Atoms handle message loading/storage internally via MessageStore trait.

use anyhow::{Context, Result};
use everruns_core::atoms::{ActAtom, Atom, CallModelAtom, ExecuteToolAtom, InputAtom, ReasonAtom};
use everruns_core::capabilities::CapabilityRegistry;
use everruns_core::llm_driver_registry::DriverRegistry;
use everruns_core::ToolRegistry;
use everruns_storage::{repositories::Database, EncryptionService};
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

// Re-export legacy atom types for activity callers
pub use everruns_core::atoms::{
    CallModelInput, CallModelResult, ExecuteToolInput, ExecuteToolResult,
};

// Re-export new turn-based atom types for activity callers
pub use everruns_core::atoms::{
    ActInput, ActResult, InputAtomInput, InputAtomResult, ReasonInput, ReasonResult, ToolCallResult,
};

// ============================================================================
// Activity Implementations
// ============================================================================

/// Call the LLM model using CallModelAtom
///
/// This activity:
/// 1. Retrieves agent configuration via AgentStore
/// 2. Resolves model and provider via LlmProviderStore
/// 3. Loads messages from the database via MessageStore
/// 4. Calls the LLM with the agent configuration
/// 5. Stores the assistant response and any tool call messages
/// 6. Returns the text and tool calls
pub async fn call_model_activity(
    db: Database,
    encryption: EncryptionService,
    input: CallModelInput,
) -> Result<CallModelResult> {
    tracing::info!(
        session_id = %input.session_id,
        agent_id = %input.agent_id,
        "Executing call_model_activity"
    );

    // Create atom dependencies
    let agent_store = DbAgentStore::new(db.clone());
    let session_store = DbSessionStore::new(db.clone());
    let message_store = DbMessageStore::new(db.clone());
    let provider_store = DbLlmProviderStore::new(db, encryption);
    let capability_registry = CapabilityRegistry::with_builtins();
    let driver_registry = create_driver_registry();

    // Create and execute CallModelAtom
    // The atom resolves model using chain: controls.model_id > session.model_id > agent.default_model_id
    let atom = CallModelAtom::new(
        agent_store,
        session_store,
        message_store,
        provider_store,
        capability_registry,
        driver_registry,
    );

    atom.execute(input)
        .await
        .context("CallModelAtom execution failed")
}

/// Execute a single tool using ExecuteToolAtom
///
/// This activity:
/// 1. Executes the tool call via ToolExecutor
/// 2. Stores the tool result message
/// 3. Returns the result
pub async fn execute_tool_activity(
    db: Database,
    input: ExecuteToolInput,
) -> Result<ExecuteToolResult> {
    // Create atom dependencies
    let message_store = DbMessageStore::new(db.clone());
    let tool_executor = ToolRegistry::with_defaults();
    let file_store = Arc::new(DbSessionFileStore::new(db));

    // Create and execute ExecuteToolAtom with file store for context-aware tools
    let atom = ExecuteToolAtom::with_file_store(message_store, tool_executor, file_store);

    atom.execute(input)
        .await
        .context("ExecuteToolAtom execution failed")
}

// ============================================================================
// New Turn-Based Activity Implementations
// ============================================================================

/// Process user input using InputAtom
///
/// This activity:
/// 1. Retrieves the user message from the message store
/// 2. Returns the message for downstream processing
pub async fn input_activity(db: Database, input: InputAtomInput) -> Result<InputAtomResult> {
    tracing::info!(
        session_id = %input.context.session_id,
        turn_id = %input.context.turn_id,
        input_message_id = %input.context.input_message_id,
        "Executing input_activity"
    );

    let message_store = DbMessageStore::new(db);
    let atom = InputAtom::new(message_store);

    atom.execute(input)
        .await
        .context("InputAtom execution failed")
}

/// Call the LLM model for reasoning using ReasonAtom
///
/// This activity:
/// 1. Retrieves agent and session configuration
/// 2. Loads messages and prepares context
/// 3. Calls the LLM with the messages
/// 4. Stores the assistant response
/// 5. Returns the result with tool calls (if any)
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
    let provider_store = DbLlmProviderStore::new(db, encryption);
    let capability_registry = CapabilityRegistry::with_builtins();
    let driver_registry = create_driver_registry();

    let atom = ReasonAtom::new(
        agent_store,
        session_store,
        message_store,
        provider_store,
        capability_registry,
        driver_registry,
    );

    atom.execute(input)
        .await
        .context("ReasonAtom execution failed")
}

/// Execute tools in parallel using ActAtom
///
/// This activity:
/// 1. Executes all tool calls in parallel
/// 2. Handles errors, timeouts, and cancellations gracefully
/// 3. Stores tool result messages
/// 4. Returns comprehensive results for all tools
pub async fn act_activity(db: Database, input: ActInput) -> Result<ActResult> {
    tracing::info!(
        session_id = %input.context.session_id,
        turn_id = %input.context.turn_id,
        tool_count = %input.tool_calls.len(),
        "Executing act_activity"
    );

    let message_store = DbMessageStore::new(db.clone());
    let tool_executor = ToolRegistry::with_defaults();
    let file_store = Arc::new(DbSessionFileStore::new(db));

    let atom = ActAtom::with_file_store(message_store, tool_executor, file_store);

    atom.execute(input)
        .await
        .context("ActAtom execution failed")
}

// ============================================================================
// Activity Type Constants
// ============================================================================

/// Activity type constants for workflow scheduling
pub mod activity_types {
    // Legacy activities
    pub const CALL_MODEL: &str = "call-model";
    pub const EXECUTE_TOOL: &str = "execute-tool";

    // New turn-based activities
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
    fn test_call_model_input_serialization() {
        let input = CallModelInput {
            session_id: Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap(),
            agent_id: Uuid::parse_str("660e8400-e29b-41d4-a716-446655440000").unwrap(),
        };

        let json = serde_json::to_string(&input).unwrap();
        let parsed: CallModelInput = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.session_id, input.session_id);
        assert_eq!(parsed.agent_id, input.agent_id);
    }

    #[test]
    fn test_execute_tool_input_serialization() {
        let input = ExecuteToolInput {
            session_id: Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap(),
            tool_call: ToolCall {
                id: "call_1".into(),
                name: "get_time".into(),
                arguments: json!({}),
            },
            tool_definitions: vec![ToolDefinition::Builtin(BuiltinTool {
                name: "get_time".into(),
                description: "Get current time".into(),
                parameters: json!({"type": "object", "properties": {}}),
                policy: ToolPolicy::Auto,
            })],
        };

        let json = serde_json::to_string(&input).unwrap();
        let parsed: ExecuteToolInput = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.tool_call.name, "get_time");
    }

    // ========================================================================
    // New Turn-Based Input/Output Serialization Tests
    // ========================================================================

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

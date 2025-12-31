// Activity implementations for workflow execution
//
// Activities are the units of work scheduled by the workflow.
// Each activity runs outside the workflow and returns a result.
//
// These implementations use Atoms from everruns-core for the actual work:
// - CallModelAtom for LLM calls
// - ExecuteToolAtom for tool execution
//
// Atoms handle message loading/storage internally via MessageStore trait.

use anyhow::{Context, Result};
use everruns_core::atoms::{Atom, CallModelAtom, ExecuteToolAtom};
use everruns_core::capabilities::CapabilityRegistry;
use everruns_core::ToolRegistry;
use everruns_storage::{repositories::Database, EncryptionService};
use std::sync::Arc;

use crate::adapters::{
    DbAgentStore, DbLlmProviderStore, DbMessageStore, DbSessionFileStore, DbSessionStore,
};

// Re-export atom types for activity callers
pub use everruns_core::atoms::{
    CallModelInput, CallModelResult, ExecuteToolInput, ExecuteToolResult,
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

    // Create and execute CallModelAtom
    // The atom resolves model using chain: controls.model_id > session.model_id > agent.default_model_id
    let atom = CallModelAtom::new(
        agent_store,
        session_store,
        message_store,
        provider_store,
        capability_registry,
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
// Activity Type Constants
// ============================================================================

/// Activity type constants for workflow scheduling
pub mod activity_types {
    pub const CALL_MODEL: &str = "call-model";
    pub const EXECUTE_TOOL: &str = "execute-tool";
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
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
}

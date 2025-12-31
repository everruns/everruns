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
use everruns_core::atoms::{
    Atom, CallModelAtom, CallModelInput as AtomCallModelInput, ExecuteToolAtom,
    ExecuteToolInput as AtomExecuteToolInput,
};
use everruns_core::capabilities::CapabilityRegistry;
use everruns_core::traits::ToolContext;
use everruns_core::{BuiltinTool, ToolCall, ToolDefinition, ToolPolicy, ToolRegistry};
use everruns_storage::{repositories::Database, EncryptionService};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::adapters::{
    DbAgentStore, DbLlmProviderStore, DbMessageStore, DbSessionFileStore, DbSessionStore,
};
use crate::agent_workflow::{ToolCallData, ToolDefinitionData, ToolResultData};

// ============================================================================
// Activity Input/Output Types
// ============================================================================

/// Input for call-model activity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallModelInput {
    /// Session ID (UUID string)
    pub session_id: String,
    /// Agent ID (UUID string)
    pub agent_id: String,
}

/// Output from call-model activity
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CallModelOutput {
    /// Text response from the model
    pub text: String,
    /// Tool calls requested by the model (if any)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCallData>>,
    /// Whether tool execution is needed
    pub needs_tool_execution: bool,
    /// Tool definitions from applied capabilities (for tool execution)
    #[serde(default)]
    pub tool_definitions: Vec<ToolDefinitionData>,
    /// Maximum iterations configured for the agent
    #[serde(default = "default_max_iterations")]
    pub max_iterations: u8,
}

fn default_max_iterations() -> u8 {
    10
}

/// Input for execute-tool activity (single tool)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecuteToolInput {
    /// Session ID (UUID string)
    pub session_id: String,
    /// Tool call to execute
    pub tool_call: ToolCallData,
    /// Available tool definitions
    pub tool_definitions: Vec<ToolDefinitionData>,
}

/// Output from execute-tool activity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecuteToolOutput {
    /// Result of the tool execution
    pub result: ToolResultData,
}

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
) -> Result<CallModelOutput> {
    let session_id: Uuid = input
        .session_id
        .parse()
        .context("Invalid session_id UUID")?;

    let agent_id: Uuid = input.agent_id.parse().context("Invalid agent_id UUID")?;

    tracing::info!(
        session_id = %session_id,
        agent_id = %agent_id,
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
    let result = atom
        .execute(AtomCallModelInput {
            session_id,
            agent_id,
        })
        .await
        .context("CallModelAtom execution failed")?;

    // Convert to activity output
    let tool_calls = if result.tool_calls.is_empty() {
        None
    } else {
        Some(
            result
                .tool_calls
                .iter()
                .map(|tc| ToolCallData {
                    id: tc.id.clone(),
                    name: tc.name.clone(),
                    arguments: tc.arguments.clone(),
                })
                .collect(),
        )
    };

    // Convert tool definitions to workflow DTO format
    let tool_definitions: Vec<ToolDefinitionData> = result
        .tool_definitions
        .iter()
        .map(|tool| match tool {
            ToolDefinition::Builtin(b) => ToolDefinitionData {
                name: b.name.clone(),
                description: b.description.clone(),
                parameters: b.parameters.clone(),
            },
        })
        .collect();

    Ok(CallModelOutput {
        text: result.text,
        tool_calls,
        needs_tool_execution: result.needs_tool_execution,
        tool_definitions,
        max_iterations: result.max_iterations.min(255) as u8,
    })
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
) -> Result<ExecuteToolOutput> {
    let session_id: Uuid = input
        .session_id
        .parse()
        .context("Invalid session_id UUID")?;

    // Create atom dependencies
    let message_store = DbMessageStore::new(db.clone());
    let tool_executor = ToolRegistry::with_defaults();

    // Create file store and tool context for context-aware tools (like filesystem tools)
    let file_store = Arc::new(DbSessionFileStore::new(db));
    let tool_context = ToolContext::with_file_store(session_id, file_store);

    // Convert tool call data
    let tool_call = ToolCall {
        id: input.tool_call.id.clone(),
        name: input.tool_call.name.clone(),
        arguments: input.tool_call.arguments.clone(),
    };

    // Convert tool definitions
    let tool_definitions: Vec<ToolDefinition> = input
        .tool_definitions
        .iter()
        .map(convert_tool_definition)
        .collect();

    // Create and execute ExecuteToolAtom
    let atom = ExecuteToolAtom::new(message_store, tool_executor);
    let result = atom
        .execute(AtomExecuteToolInput {
            session_id,
            tool_call: tool_call.clone(),
            tool_definitions,
            tool_context: Some(tool_context),
        })
        .await
        .context("ExecuteToolAtom execution failed")?;

    Ok(ExecuteToolOutput {
        result: ToolResultData {
            tool_call_id: tool_call.id,
            result: result.result.result,
            error: result.result.error,
        },
    })
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Convert workflow's ToolDefinitionData to core's ToolDefinition
fn convert_tool_definition(tool: &ToolDefinitionData) -> ToolDefinition {
    ToolDefinition::Builtin(BuiltinTool {
        name: tool.name.clone(),
        description: tool.description.clone(),
        parameters: tool.parameters.clone(),
        policy: ToolPolicy::Auto,
    })
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
    use serde_json::json;

    #[test]
    fn test_call_model_input_serialization() {
        let input = CallModelInput {
            session_id: "550e8400-e29b-41d4-a716-446655440000".into(),
            agent_id: "660e8400-e29b-41d4-a716-446655440000".into(),
        };

        let json = serde_json::to_string(&input).unwrap();
        let parsed: CallModelInput = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.session_id, input.session_id);
        assert_eq!(parsed.agent_id, input.agent_id);
    }

    #[test]
    fn test_execute_tool_input_serialization() {
        let input = ExecuteToolInput {
            session_id: "550e8400-e29b-41d4-a716-446655440000".into(),
            tool_call: ToolCallData {
                id: "call_1".into(),
                name: "get_time".into(),
                arguments: json!({}),
            },
            tool_definitions: vec![ToolDefinitionData {
                name: "get_time".into(),
                description: "Get current time".into(),
                parameters: json!({"type": "object", "properties": {}}),
            }],
        };

        let json = serde_json::to_string(&input).unwrap();
        let parsed: ExecuteToolInput = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.tool_call.name, "get_time");
    }
}

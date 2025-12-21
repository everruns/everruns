// V2 Activity Implementations
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
use everruns_contracts::tools::{
    BuiltinTool, BuiltinToolKind, ToolCall, ToolDefinition, ToolPolicy,
};
use everruns_core::atoms::{
    Atom, CallModelAtom, CallModelInput as AtomCallModelInput, ExecuteToolAtom,
    ExecuteToolInput as AtomExecuteToolInput,
};
use everruns_core::config::AgentConfigBuilder;
use everruns_core::openai::OpenAIProtocolLlmProvider;
use everruns_storage::repositories::Database;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::session_workflow::{AgentConfigData, ToolCallData, ToolDefinitionData, ToolResultData};
use crate::adapters::DbMessageStore;
use crate::unified_tool_executor::UnifiedToolExecutor;

// ============================================================================
// Activity Input/Output Types
// ============================================================================

/// Input for call-model activity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallModelInput {
    /// Session ID (UUID string)
    pub session_id: String,
    /// Agent configuration (model, tools, system_prompt)
    pub agent_config: AgentConfigData,
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

// Legacy types for backwards compatibility
pub type ExecuteToolsInput = ExecuteToolInput;
pub type ExecuteToolsOutput = ExecuteToolOutput;

// ============================================================================
// Activity Implementations
// ============================================================================

/// Call the LLM model using CallModelAtom
///
/// This activity:
/// 1. Loads messages from the database via MessageStore
/// 2. Calls the LLM with the agent configuration
/// 3. Stores the assistant response and any tool call messages
/// 4. Returns the text and tool calls
pub async fn call_model_activity(db: Database, input: CallModelInput) -> Result<CallModelOutput> {
    let session_id: Uuid = input
        .session_id
        .parse()
        .context("Invalid session_id UUID")?;

    // Create atom dependencies
    let message_store = DbMessageStore::new(db);
    let llm_provider =
        OpenAIProtocolLlmProvider::from_env().context("Failed to create LLM provider")?;

    // Build AgentConfig from the workflow's AgentConfigData
    let agent_config = build_agent_config(&input.agent_config);

    // Create and execute CallModelAtom
    let atom = CallModelAtom::new(message_store, llm_provider);
    let result = atom
        .execute(AtomCallModelInput {
            session_id,
            config: agent_config,
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

    Ok(CallModelOutput {
        text: result.text,
        tool_calls,
        needs_tool_execution: result.needs_tool_execution,
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
    let message_store = DbMessageStore::new(db);
    let tool_executor = UnifiedToolExecutor::with_default_tools();

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

/// Build AgentConfig from workflow's AgentConfigData
fn build_agent_config(data: &AgentConfigData) -> everruns_core::AgentConfig {
    let tools: Vec<ToolDefinition> = data.tools.iter().map(convert_tool_definition).collect();

    AgentConfigBuilder::new()
        .model(&data.model)
        .system_prompt(data.system_prompt.as_deref().unwrap_or(""))
        .tools(tools)
        .max_iterations(data.max_iterations as usize)
        .build()
}

/// Convert workflow's ToolDefinitionData to core's ToolDefinition
fn convert_tool_definition(tool: &ToolDefinitionData) -> ToolDefinition {
    ToolDefinition::Builtin(BuiltinTool {
        name: tool.name.clone(),
        description: tool.description.clone(),
        parameters: tool.parameters.clone(),
        kind: BuiltinToolKind::CurrentTime, // Default kind, actual execution is by name
        policy: ToolPolicy::Auto,
    })
}

// ============================================================================
// Activity Type Constants
// ============================================================================

/// Activity type constants matching workflow's activity_names
pub mod activity_types {
    pub const CALL_MODEL: &str = "call-model";
    pub const EXECUTE_TOOL: &str = "execute-tool";
    pub const LOAD_AGENT: &str = "load-agent";
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
            agent_config: AgentConfigData {
                model: "gpt-5.2".into(),
                system_prompt: Some("You are a helpful assistant.".into()),
                tools: vec![],
                max_iterations: 5,
            },
        };

        let json = serde_json::to_string(&input).unwrap();
        let parsed: CallModelInput = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.session_id, input.session_id);
        assert_eq!(parsed.agent_config.model, "gpt-5.2");
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

    #[test]
    fn test_build_agent_config() {
        let data = AgentConfigData {
            model: "gpt-5.2".into(),
            system_prompt: Some("Test prompt".into()),
            tools: vec![ToolDefinitionData {
                name: "test_tool".into(),
                description: "A test tool".into(),
                parameters: json!({}),
            }],
            max_iterations: 10,
        };

        let config = build_agent_config(&data);
        assert_eq!(config.model, "gpt-5.2");
        assert_eq!(config.system_prompt, "Test prompt");
        assert_eq!(config.tools.len(), 1);
        assert_eq!(config.max_iterations, 10);
    }
}

// Temporal workflow and activity type definitions (M2)
// Decision: Keep Temporal types in a dedicated module for clean separation
// Decision: Use step.rs abstractions (StepInput/StepOutput) for decomposed execution
//
// These types define the inputs/outputs for workflows and activities in the
// Temporal execution model. All types must be serializable for Temporal's
// persistence layer.
//
// M2 model: Agent → Session → Messages (no separate Thread/Run concepts)
// Each LLM call and each tool execution is a separate Temporal activity (node).

use everruns_contracts::tools::{ToolCall, ToolResult};
use everruns_core::message::ConversationMessage;
use everruns_core::step::{LoopStep, StepOutput};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Result from processing a workflow activation
#[derive(Debug)]
pub enum WorkflowAction {
    /// Schedule an activity
    ScheduleActivity {
        activity_id: String,
        activity_type: String,
        input: serde_json::Value,
    },
    /// Complete the workflow successfully
    CompleteWorkflow { result: Option<serde_json::Value> },
    /// Fail the workflow
    FailWorkflow { reason: String },
    /// No action needed (waiting for activity result)
    None,
}

/// Input for the Session workflow (M2)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionWorkflowInput {
    pub session_id: Uuid,
    pub agent_id: Uuid,
}

/// Output for the Session workflow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionWorkflowOutput {
    pub session_id: Uuid,
    pub status: String,
    pub iterations: u32,
}

/// Legacy alias for backwards compatibility
pub type AgentRunWorkflowInput = SessionWorkflowInput;
pub type AgentRunWorkflowOutput = SessionWorkflowOutput;

/// Input for the LoadAgent activity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadAgentInput {
    pub agent_id: Uuid,
}

/// Output from LoadAgent activity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadAgentOutput {
    pub agent_id: Uuid,
    pub name: String,
    pub model_id: String,
    pub system_prompt: Option<String>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    /// Capability IDs enabled for this agent (e.g., ["current_time", "research"])
    #[serde(default)]
    pub capability_ids: Vec<String>,
}

/// Input for the LoadMessages activity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadMessagesInput {
    pub session_id: Uuid,
}

/// A message in the conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageData {
    pub role: String,
    pub content: String,
    /// Tool calls for assistant messages (when the assistant requests tool execution)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCallData>>,
    /// Tool call ID for tool result messages (links result to the original tool call)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

/// Output from LoadMessages activity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadMessagesOutput {
    pub messages: Vec<MessageData>,
}

/// Input for the UpdateStatus activity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateStatusInput {
    pub session_id: Uuid,
    pub status: String,
    pub started_at: Option<chrono::DateTime<chrono::Utc>>,
    pub finished_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Input for the PersistEvent activity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistEventInput {
    pub session_id: Uuid,
    pub event_data: serde_json::Value,
}

/// Input for the CallLLM activity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallLlmInput {
    pub session_id: Uuid,
    pub messages: Vec<MessageData>,
    pub model_id: String,
    pub system_prompt: Option<String>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    /// Capability IDs to apply (resolve to tools)
    #[serde(default)]
    pub capability_ids: Vec<String>,
}

/// A tool call from the LLM
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallData {
    pub id: String,
    pub name: String,
    pub arguments: String,
    /// Optional tool definition JSON (for future use)
    #[serde(default)]
    pub tool_definition_json: Option<String>,
}

/// Output from CallLLM activity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallLlmOutput {
    pub text: String,
    pub tool_calls: Option<Vec<ToolCallData>>,
}

/// Input for the ExecuteTools activity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecuteToolsInput {
    pub session_id: Uuid,
    pub tool_calls: Vec<ToolCallData>,
}

/// Result of a single tool execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResultData {
    pub tool_call_id: String,
    pub result: Option<serde_json::Value>,
    pub error: Option<String>,
}

/// Output from ExecuteTools activity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecuteToolsOutput {
    pub results: Vec<ToolResultData>,
}

/// Input for the SaveMessage activity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveMessageInput {
    pub session_id: Uuid,
    pub role: String,
    pub content: serde_json::Value,
    /// Tool call ID for tool result messages (links result to the original tool call)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

/// Constants for activity names (used for registration and invocation)
pub mod activity_names {
    // Legacy activities (still used for compatibility)
    pub const LOAD_AGENT: &str = "load_agent";
    pub const LOAD_MESSAGES: &str = "load_messages";
    pub const UPDATE_STATUS: &str = "update_status";
    pub const PERSIST_EVENT: &str = "persist_event";
    pub const CALL_LLM: &str = "call_llm";
    pub const EXECUTE_TOOLS: &str = "execute_tools";
    pub const SAVE_MESSAGE: &str = "save_message";

    // Step-based activities (using step.rs abstractions)
    pub const SETUP_STEP: &str = "setup_step";
    pub const EXECUTE_LLM_STEP: &str = "execute_llm_step";
    pub const EXECUTE_SINGLE_TOOL: &str = "execute_single_tool";
    pub const FINALIZE_STEP: &str = "finalize_step";
}

/// Constants for workflow names
pub mod workflow_names {
    pub const SESSION_WORKFLOW: &str = "session_workflow";
    /// Legacy alias
    pub const AGENT_RUN: &str = "session_workflow";
}

/// Task queue name for agent runs
pub const TASK_QUEUE: &str = "everruns-agent-runs";

/// Maximum number of tool calling iterations
pub const MAX_TOOL_ITERATIONS: u32 = 5;

// =============================================================================
// Step-based activity types (using step.rs abstractions)
// =============================================================================

/// Input for the SetupStep activity
/// Loads agent configuration and session messages
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetupStepInput {
    pub session_id: Uuid,
    pub agent_id: Uuid,
}

/// Output from SetupStep activity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetupStepOutput {
    /// Agent configuration
    pub agent_config: LoadAgentOutput,
    /// Initial messages loaded from the session
    pub messages: Vec<ConversationMessage>,
    /// The setup step record
    pub step: LoopStep,
}

/// Input for the ExecuteLlmStep activity
/// Calls LLM with current messages
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecuteLlmStepInput {
    pub session_id: Uuid,
    pub agent_config: LoadAgentOutput,
    pub messages: Vec<ConversationMessage>,
    pub iteration: usize,
}

/// Output from ExecuteLlmStep activity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecuteLlmStepOutput {
    /// The step output from the agent loop
    pub step_output: StepOutput,
    /// Whether there are pending tool calls
    pub has_tool_calls: bool,
    /// Pending tool calls (if any)
    pub pending_tool_calls: Vec<ToolCall>,
}

/// Input for the ExecuteSingleTool activity
/// Executes ONE tool call - each tool is a separate Temporal node
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecuteSingleToolInput {
    pub session_id: Uuid,
    pub tool_call: ToolCall,
    /// Tool definition JSON (for future use)
    pub tool_definition_json: Option<String>,
}

/// Output from ExecuteSingleTool activity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecuteSingleToolOutput {
    /// The tool result
    pub result: ToolResult,
    /// The tool execution step record
    pub step: LoopStep,
}

/// Input for the FinalizeStep activity
/// Saves final message and updates session status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FinalizeStepInput {
    pub session_id: Uuid,
    pub final_messages: Vec<ConversationMessage>,
    pub total_iterations: usize,
    pub final_response: Option<String>,
}

/// Output from FinalizeStep activity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FinalizeStepOutput {
    /// Final status
    pub status: String,
    /// The finalize step record
    pub step: LoopStep,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_data_serialization_with_tool_calls() {
        let tool_calls = vec![ToolCallData {
            id: "call_123".to_string(),
            name: "get_weather".to_string(),
            arguments: r#"{"location": "NYC"}"#.to_string(),
            tool_definition_json: None,
        }];

        let msg = MessageData {
            role: "assistant".to_string(),
            content: "Let me check the weather.".to_string(),
            tool_calls: Some(tool_calls),
            tool_call_id: None,
        };

        let json = serde_json::to_string(&msg).unwrap();
        let parsed: MessageData = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.role, "assistant");
        assert_eq!(parsed.content, "Let me check the weather.");
        assert!(parsed.tool_calls.is_some());
        assert_eq!(parsed.tool_calls.as_ref().unwrap().len(), 1);
        assert_eq!(parsed.tool_calls.as_ref().unwrap()[0].id, "call_123");
        assert_eq!(parsed.tool_calls.as_ref().unwrap()[0].name, "get_weather");
        assert!(parsed.tool_call_id.is_none());
    }

    #[test]
    fn test_message_data_serialization_with_tool_call_id() {
        let msg = MessageData {
            role: "tool".to_string(),
            content: r#"{"temperature": 72, "unit": "F"}"#.to_string(),
            tool_calls: None,
            tool_call_id: Some("call_123".to_string()),
        };

        let json = serde_json::to_string(&msg).unwrap();
        let parsed: MessageData = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.role, "tool");
        assert!(parsed.tool_calls.is_none());
        assert_eq!(parsed.tool_call_id, Some("call_123".to_string()));
    }

    #[test]
    fn test_message_data_serialization_simple_user_message() {
        let msg = MessageData {
            role: "user".to_string(),
            content: "What's the weather?".to_string(),
            tool_calls: None,
            tool_call_id: None,
        };

        let json = serde_json::to_string(&msg).unwrap();
        // Verify that None fields are skipped in serialization
        assert!(!json.contains("tool_calls"));
        assert!(!json.contains("tool_call_id"));

        let parsed: MessageData = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.role, "user");
        assert_eq!(parsed.content, "What's the weather?");
        assert!(parsed.tool_calls.is_none());
        assert!(parsed.tool_call_id.is_none());
    }

    #[test]
    fn test_message_data_deserialization_from_minimal_json() {
        // Test backward compatibility - can deserialize JSON without tool fields
        let json = r#"{"role": "user", "content": "Hello"}"#;
        let parsed: MessageData = serde_json::from_str(json).unwrap();

        assert_eq!(parsed.role, "user");
        assert_eq!(parsed.content, "Hello");
        assert!(parsed.tool_calls.is_none());
        assert!(parsed.tool_call_id.is_none());
    }

    #[test]
    fn test_tool_call_data_arguments_as_json_string() {
        let tc = ToolCallData {
            id: "call_abc".to_string(),
            name: "search".to_string(),
            arguments: r#"{"query": "rust programming", "limit": 10}"#.to_string(),
            tool_definition_json: None,
        };

        // Verify arguments can be parsed as valid JSON
        let args: serde_json::Value = serde_json::from_str(&tc.arguments).unwrap();
        assert_eq!(args["query"], "rust programming");
        assert_eq!(args["limit"], 10);
    }

    #[test]
    fn test_call_llm_output_with_tool_calls() {
        let output = CallLlmOutput {
            text: "I'll help you with that.".to_string(),
            tool_calls: Some(vec![
                ToolCallData {
                    id: "call_1".to_string(),
                    name: "tool_a".to_string(),
                    arguments: "{}".to_string(),
                    tool_definition_json: None,
                },
                ToolCallData {
                    id: "call_2".to_string(),
                    name: "tool_b".to_string(),
                    arguments: r#"{"x": 1}"#.to_string(),
                    tool_definition_json: None,
                },
            ]),
        };

        let json = serde_json::to_string(&output).unwrap();
        let parsed: CallLlmOutput = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.text, "I'll help you with that.");
        assert!(parsed.tool_calls.is_some());
        assert_eq!(parsed.tool_calls.as_ref().unwrap().len(), 2);
    }

    #[test]
    fn test_call_llm_output_without_tool_calls() {
        let output = CallLlmOutput {
            text: "The answer is 42.".to_string(),
            tool_calls: None,
        };

        let json = serde_json::to_string(&output).unwrap();
        let parsed: CallLlmOutput = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.text, "The answer is 42.");
        assert!(parsed.tool_calls.is_none());
    }

    #[test]
    fn test_tool_result_data_with_success() {
        let result = ToolResultData {
            tool_call_id: "call_123".to_string(),
            result: Some(serde_json::json!({"status": "ok", "data": [1, 2, 3]})),
            error: None,
        };

        let json = serde_json::to_string(&result).unwrap();
        let parsed: ToolResultData = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.tool_call_id, "call_123");
        assert!(parsed.result.is_some());
        assert!(parsed.error.is_none());
    }

    #[test]
    fn test_tool_result_data_with_error() {
        let result = ToolResultData {
            tool_call_id: "call_456".to_string(),
            result: None,
            error: Some("Tool execution failed: timeout".to_string()),
        };

        let json = serde_json::to_string(&result).unwrap();
        let parsed: ToolResultData = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.tool_call_id, "call_456");
        assert!(parsed.result.is_none());
        assert_eq!(
            parsed.error,
            Some("Tool execution failed: timeout".to_string())
        );
    }

    #[test]
    fn test_execute_tools_output() {
        let output = ExecuteToolsOutput {
            results: vec![
                ToolResultData {
                    tool_call_id: "call_1".to_string(),
                    result: Some(serde_json::json!("success")),
                    error: None,
                },
                ToolResultData {
                    tool_call_id: "call_2".to_string(),
                    result: None,
                    error: Some("failed".to_string()),
                },
            ],
        };

        let json = serde_json::to_string(&output).unwrap();
        let parsed: ExecuteToolsOutput = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.results.len(), 2);
        assert_eq!(parsed.results[0].tool_call_id, "call_1");
        assert!(parsed.results[0].result.is_some());
        assert_eq!(parsed.results[1].tool_call_id, "call_2");
        assert!(parsed.results[1].error.is_some());
    }
}

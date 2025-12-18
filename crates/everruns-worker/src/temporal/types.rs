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

use everruns_agent_loop::message::ConversationMessage;
use everruns_agent_loop::step::{LoopStep, StepOutput};
use everruns_contracts::tools::{ToolCall, ToolResult};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

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
}

/// A tool call from the LLM
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallData {
    pub id: String,
    pub name: String,
    pub arguments: String,
    /// Optional tool definition JSON for webhook tools
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
    /// Tool definition JSON for webhook tools
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

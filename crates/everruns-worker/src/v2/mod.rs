// V2 Workflow Module
//
// Simpler workflow design leveraging everruns-core primitives.
// See session_workflow.rs for the main implementation.

pub mod activities;
pub mod session_workflow;

pub use activities::{
    activity_types, call_model_activity, execute_tool_activity, CallModelInput, CallModelOutput,
    ExecuteToolInput, ExecuteToolOutput, ExecuteToolsInput, ExecuteToolsOutput,
};
pub use session_workflow::{
    AgentConfigData, MessageData, SessionWorkflowV2, SessionWorkflowV2Input, ToolCallData,
    ToolDefinitionData, ToolResultData,
};

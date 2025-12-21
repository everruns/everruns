// V2 Workflow Module
//
// Simpler workflow design leveraging everruns-core primitives.
// See agent_workflow.rs for the main implementation.

pub mod activities;
pub mod agent_workflow;

pub use activities::{
    activity_types, call_model_activity, execute_tool_activity, load_agent_activity,
    CallModelInput, CallModelOutput, ExecuteToolInput, ExecuteToolOutput, ExecuteToolsInput,
    ExecuteToolsOutput, LoadAgentInput,
};
pub use agent_workflow::{
    AgentConfigData, AgentWorkflow, AgentWorkflowInput, MessageData, ToolCallData,
    ToolDefinitionData, ToolResultData,
};

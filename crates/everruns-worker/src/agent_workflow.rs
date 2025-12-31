// Agent Workflow - Agent Loop Orchestration with Atoms
//
// Design: Lightweight state machine that orchestrates atoms
//
// Key design principles:
// - Atoms handle message loading/storage internally via MessageStore
// - Atoms emit events via EventEmitter (no separate emit-event activities)
// - CallModelAtom loads agent config, resolves model/provider, applies capabilities
// - Workflow only tracks session_id, tool_definitions, and iteration
// - Each tool call is a separate activity for better visibility
// - No message passing between states - atoms load from DB
//
// State machine:
// Init → CallingModel →
//   (tool_calls?) → ExecutingTools → (wait for all) → CallingModel (loop)
//   (no tools)   → Completed

use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::activities::{CallModelOutput, ExecuteToolOutput};
use crate::traits::{Workflow, WorkflowInput};
use crate::types::WorkflowAction;

// ============================================================================
// Input/Output Types
// ============================================================================

/// Workflow input
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentWorkflowInput {
    /// Session ID
    pub session_id: Uuid,
    /// Agent ID for loading configuration
    pub agent_id: Uuid,
}

/// Simple message data for serialization (kept for compatibility)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageData {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCallData>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

/// Tool call data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallData {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

/// Tool result data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResultData {
    pub tool_call_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Agent configuration loaded from storage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfigData {
    pub model: String,
    /// Provider type (openai, anthropic, azure_openai, ollama, custom)
    #[serde(default = "default_provider_type")]
    pub provider_type: String,
    /// Optional API key (only passed when provider-specific key is configured)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// Optional base URL override
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    pub system_prompt: Option<String>,
    pub tools: Vec<ToolDefinitionData>,
    #[serde(default)]
    pub max_iterations: u8,
}

fn default_provider_type() -> String {
    "openai".to_string()
}

impl Default for AgentConfigData {
    fn default() -> Self {
        Self {
            model: "gpt-4o".to_string(),
            provider_type: "openai".to_string(),
            api_key: None,
            base_url: None,
            system_prompt: None,
            tools: Vec::new(),
            max_iterations: 10,
        }
    }
}

/// Tool definition data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinitionData {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

// ============================================================================
// Workflow State (Simplified - no message passing, no emit-event)
// ============================================================================

/// Workflow states - atoms handle message storage and event emission
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WorkflowState {
    /// Initial state
    Init,

    /// Waiting for LLM response (CallModelAtom loads agent config and messages from DB)
    CallingModel {
        pending_activity: String,
        /// Tool definitions from previous call (None on first call)
        tool_definitions: Option<Vec<ToolDefinitionData>>,
        /// Max iterations from previous call (None on first call)
        max_iterations: Option<u8>,
        iteration: u8,
    },

    /// Executing tools (one activity per tool)
    ExecutingTools {
        /// Activity IDs for pending tool executions
        pending_activities: Vec<String>,
        /// Completed tool results (activity_id -> result)
        completed: Vec<(String, ToolResultData)>,
        /// Tool definitions from call-model result
        tool_definitions: Vec<ToolDefinitionData>,
        /// Max iterations from call-model result
        max_iterations: u8,
        tool_calls: Vec<ToolCallData>,
        iteration: u8,
    },

    /// Terminal: workflow completed
    Completed { final_text: Option<String> },

    /// Terminal: workflow failed
    Failed { error: String },
}

// ============================================================================
// Activity Names
// ============================================================================

mod activity_names {
    pub const CALL_MODEL: &str = "call-model";
    pub const EXECUTE_TOOL: &str = "execute-tool";
}

// ============================================================================
// Workflow Implementation
// ============================================================================

/// Agent Workflow with Atoms
#[derive(Debug)]
pub struct AgentWorkflow {
    input: AgentWorkflowInput,
    state: WorkflowState,
    activity_seq: u32,
}

impl AgentWorkflow {
    pub fn new(input: AgentWorkflowInput) -> Self {
        Self {
            input,
            state: WorkflowState::Init,
            activity_seq: 0,
        }
    }

    fn next_activity_id(&mut self, prefix: &str) -> String {
        self.activity_seq += 1;
        format!("{}-{}", prefix, self.activity_seq)
    }

    fn session_id(&self) -> String {
        self.input.session_id.to_string()
    }

    // =========================================================================
    // State Handlers
    // =========================================================================

    fn handle_model_response(
        &mut self,
        iteration: u8,
        result: serde_json::Value,
    ) -> Vec<WorkflowAction> {
        // Deserialize directly to CallModelOutput (same struct the activity returns)
        let output: CallModelOutput = serde_json::from_value(result).unwrap_or_default();

        // Check if we need to execute tools
        if output.needs_tool_execution {
            if let Some(tool_calls) = output.tool_calls {
                if !tool_calls.is_empty() && iteration < output.max_iterations {
                    return self.transition_to_tool_execution(
                        output.tool_definitions,
                        output.max_iterations,
                        tool_calls,
                        iteration,
                    );
                }
            }
        }

        // No tool calls or max iterations - complete
        self.state = WorkflowState::Completed {
            final_text: Some(output.text),
        };

        vec![WorkflowAction::CompleteWorkflow {
            result: Some(json!({
                "status": "completed",
                "session_id": self.session_id(),
            })),
        }]
    }

    fn handle_tool_completed(
        &mut self,
        activity_id: String,
        result: serde_json::Value,
    ) -> Vec<WorkflowAction> {
        // Extract current state
        let (
            mut pending_activities,
            mut completed,
            tool_definitions,
            max_iterations,
            tool_calls,
            iteration,
        ) = match &self.state {
            WorkflowState::ExecutingTools {
                pending_activities,
                completed,
                tool_definitions,
                max_iterations,
                tool_calls,
                iteration,
            } => (
                pending_activities.clone(),
                completed.clone(),
                tool_definitions.clone(),
                *max_iterations,
                tool_calls.clone(),
                *iteration,
            ),
            _ => return vec![],
        };

        // Deserialize directly to ExecuteToolOutput (same struct the activity returns)
        let output: ExecuteToolOutput =
            serde_json::from_value(result).unwrap_or(ExecuteToolOutput {
                result: ToolResultData {
                    tool_call_id: "unknown".to_string(),
                    result: None,
                    error: Some("Failed to parse tool result".to_string()),
                },
            });

        // Remove from pending, add to completed
        pending_activities.retain(|id| id != &activity_id);
        completed.push((activity_id, output.result));

        // Check if all tools are done
        if pending_activities.is_empty() {
            // All tools completed, go to next iteration
            self.transition_to_model_call(
                Some(tool_definitions),
                Some(max_iterations),
                iteration + 1,
            )
        } else {
            // Still waiting for more tools
            self.state = WorkflowState::ExecutingTools {
                pending_activities,
                completed,
                tool_definitions,
                max_iterations,
                tool_calls,
                iteration,
            };
            vec![]
        }
    }
}

impl Workflow for AgentWorkflow {
    fn workflow_type(&self) -> &'static str {
        "agent_workflow"
    }

    fn on_start(&mut self) -> Vec<WorkflowAction> {
        // Go directly to calling the model - CallModelAtom handles agent loading
        self.transition_to_model_call(None, None, 1)
    }

    fn on_activity_completed(
        &mut self,
        activity_id: &str,
        result: serde_json::Value,
    ) -> Vec<WorkflowAction> {
        let state = self.state.clone();

        match state {
            WorkflowState::CallingModel {
                pending_activity,
                iteration,
                ..
            } if pending_activity == activity_id => self.handle_model_response(iteration, result),

            WorkflowState::ExecutingTools {
                ref pending_activities,
                ..
            } if pending_activities.contains(&activity_id.to_string()) => {
                self.handle_tool_completed(activity_id.to_string(), result)
            }

            _ => vec![],
        }
    }

    fn on_activity_failed(&mut self, _activity_id: &str, error: &str) -> Vec<WorkflowAction> {
        self.state = WorkflowState::Failed {
            error: error.to_string(),
        };

        vec![WorkflowAction::FailWorkflow {
            reason: error.to_string(),
        }]
    }

    fn is_completed(&self) -> bool {
        matches!(
            self.state,
            WorkflowState::Completed { .. } | WorkflowState::Failed { .. }
        )
    }
}

// Helper methods for state transitions
impl AgentWorkflow {
    fn transition_to_model_call(
        &mut self,
        tool_definitions: Option<Vec<ToolDefinitionData>>,
        max_iterations: Option<u8>,
        iteration: u8,
    ) -> Vec<WorkflowAction> {
        let activity_id = self.next_activity_id(activity_names::CALL_MODEL);

        self.state = WorkflowState::CallingModel {
            pending_activity: activity_id.clone(),
            tool_definitions: tool_definitions.clone(),
            max_iterations,
            iteration,
        };

        // CallModelInput only needs session_id and agent_id
        // The atom handles loading agent config, model resolution, and capabilities
        vec![WorkflowAction::ScheduleActivity {
            activity_id,
            activity_type: activity_names::CALL_MODEL.to_string(),
            input: json!({
                "session_id": self.session_id(),
                "agent_id": self.input.agent_id.to_string(),
            }),
        }]
    }

    fn transition_to_tool_execution(
        &mut self,
        tool_definitions: Vec<ToolDefinitionData>,
        max_iterations: u8,
        tool_calls: Vec<ToolCallData>,
        iteration: u8,
    ) -> Vec<WorkflowAction> {
        let mut actions = Vec::new();
        let mut pending_activities = Vec::new();

        // Schedule one activity per tool call
        for tool_call in &tool_calls {
            let activity_id = self.next_activity_id(activity_names::EXECUTE_TOOL);
            pending_activities.push(activity_id.clone());

            actions.push(WorkflowAction::ScheduleActivity {
                activity_id,
                activity_type: activity_names::EXECUTE_TOOL.to_string(),
                input: json!({
                    "session_id": self.session_id(),
                    "tool_call": tool_call,
                    "tool_definitions": tool_definitions,
                }),
            });
        }

        self.state = WorkflowState::ExecutingTools {
            pending_activities,
            completed: Vec::new(),
            tool_definitions,
            max_iterations,
            tool_calls,
            iteration,
        };

        actions
    }
}

impl WorkflowInput for AgentWorkflow {
    const WORKFLOW_TYPE: &'static str = "agent_workflow";
    type Input = AgentWorkflowInput;

    fn from_input(input: Self::Input) -> Self {
        AgentWorkflow::new(input)
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn find_activity_id(actions: &[WorkflowAction], activity_type_prefix: &str) -> Option<String> {
        actions.iter().find_map(|a| {
            if let WorkflowAction::ScheduleActivity {
                activity_id,
                activity_type,
                ..
            } = a
            {
                if activity_type.starts_with(activity_type_prefix) {
                    return Some(activity_id.clone());
                }
            }
            None
        })
    }

    #[test]
    fn test_workflow_start() {
        let input = AgentWorkflowInput {
            session_id: Uuid::now_v7(),
            agent_id: Uuid::now_v7(),
        };

        let mut workflow = AgentWorkflow::new(input);
        let actions = workflow.on_start();

        // Should go directly to call-model (CallModelAtom handles agent loading)
        assert_eq!(actions.len(), 1);
        assert!(find_activity_id(&actions, "call-model").is_some());
        assert!(!workflow.is_completed());
    }

    #[test]
    fn test_model_with_tool_calls() {
        let input = AgentWorkflowInput {
            session_id: Uuid::now_v7(),
            agent_id: Uuid::now_v7(),
        };

        let mut workflow = AgentWorkflow::new(input);
        let actions = workflow.on_start();
        let call_model_id = find_activity_id(&actions, "call-model").unwrap();

        // LLM returns tool calls (includes tool_definitions and max_iterations from CallModelOutput)
        let actions = workflow.on_activity_completed(
            &call_model_id,
            json!({
                "text": "Let me check.",
                "tool_calls": [{"id": "call_1", "name": "get_time", "arguments": {}}],
                "needs_tool_execution": true,
                "tool_definitions": [{"name": "get_time", "description": "Get time", "parameters": {}}],
                "max_iterations": 10
            }),
        );

        // Should schedule execute-tool
        assert_eq!(actions.len(), 1);
        assert!(find_activity_id(&actions, "execute-tool").is_some());
    }

    #[test]
    fn test_completion_without_tools() {
        let input = AgentWorkflowInput {
            session_id: Uuid::now_v7(),
            agent_id: Uuid::now_v7(),
        };

        let mut workflow = AgentWorkflow::new(input);
        let actions = workflow.on_start();
        let call_model_id = find_activity_id(&actions, "call-model").unwrap();

        // LLM returns no tool calls
        let actions = workflow.on_activity_completed(
            &call_model_id,
            json!({
                "text": "Hello!",
                "tool_calls": null,
                "needs_tool_execution": false,
                "tool_definitions": [],
                "max_iterations": 10
            }),
        );

        // Should complete
        assert_eq!(actions.len(), 1);
        assert!(workflow.is_completed());
        assert!(actions
            .iter()
            .any(|a| matches!(a, WorkflowAction::CompleteWorkflow { .. })));
    }
}

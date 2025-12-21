// Session Workflow V2 - Agent Loop Orchestration with Atoms
//
// Design: Lightweight state machine that orchestrates atoms
//
// Key design principles:
// - Atoms handle message loading/storage internally via MessageStore
// - Workflow only tracks session_id, agent_config, and iteration
// - Each tool call is a separate activity for better visibility
// - No message passing between states - atoms load from DB
//
// State machine:
// Init → LoadingAgent → PreModelCall → CallingModel →
//   (tool_calls?) → ExecutingTools → (wait for all) → PreModelCall (loop)
//   (no tools)   → Completed
//
// Events emitted:
// - LoopStarted: On workflow start
// - LlmCallStarted/LlmCallCompleted: Before/after model call
// - ToolExecutionStarted/ToolExecutionCompleted: For each tool
// - LoopCompleted/LoopError: On workflow end

use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::types::WorkflowAction;
use crate::workflow_traits::{Workflow, WorkflowInput};

// ============================================================================
// Input/Output Types
// ============================================================================

/// Workflow input
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionWorkflowV2Input {
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
    pub system_prompt: Option<String>,
    pub tools: Vec<ToolDefinitionData>,
    #[serde(default)]
    pub max_iterations: u8,
}

impl Default for AgentConfigData {
    fn default() -> Self {
        Self {
            model: "gpt-5.2".to_string(),
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
// Workflow State (Simplified - no message passing)
// ============================================================================

/// Workflow states - atoms handle message storage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WorkflowState {
    /// Initial: emit start event, load agent config
    Init,

    /// Loading agent configuration
    LoadingAgent { pending_activity: String },

    /// Ready to call LLM (atoms load messages from DB)
    PreModelCall {
        agent_config: AgentConfigData,
        iteration: u8,
    },

    /// Waiting for LLM response
    CallingModel {
        pending_activity: String,
        agent_config: AgentConfigData,
        iteration: u8,
    },

    /// Executing tools (one activity per tool)
    ExecutingTools {
        /// Activity IDs for pending tool executions
        pending_activities: Vec<String>,
        /// Completed tool results (activity_id -> result)
        completed: Vec<(String, ToolResultData)>,
        agent_config: AgentConfigData,
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
    pub const LOAD_AGENT: &str = "load-agent";
    pub const CALL_MODEL: &str = "call-model";
    pub const EXECUTE_TOOL: &str = "execute-tool";
    pub const EMIT_EVENT: &str = "emit-event";
}

// ============================================================================
// Workflow Implementation
// ============================================================================

/// V2 Session Workflow with Atoms
#[derive(Debug)]
pub struct SessionWorkflowV2 {
    input: SessionWorkflowV2Input,
    state: WorkflowState,
    activity_seq: u32,
}

impl SessionWorkflowV2 {
    pub fn new(input: SessionWorkflowV2Input) -> Self {
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

    /// Create activity to emit an event
    fn emit_event_activity(&mut self, event_type: &str, data: serde_json::Value) -> WorkflowAction {
        WorkflowAction::ScheduleActivity {
            activity_id: self.next_activity_id(activity_names::EMIT_EVENT),
            activity_type: activity_names::EMIT_EVENT.to_string(),
            input: json!({
                "session_id": self.session_id(),
                "event_type": event_type,
                "data": data,
            }),
        }
    }

    // =========================================================================
    // State Handlers
    // =========================================================================

    fn handle_agent_loaded(&mut self, result: serde_json::Value) -> Vec<WorkflowAction> {
        let agent_config: AgentConfigData = serde_json::from_value(result).unwrap_or_default();

        // Go directly to PreModelCall - atoms will load messages
        self.state = WorkflowState::PreModelCall {
            agent_config,
            iteration: 1,
        };
        self.transition_to_model_call()
    }

    fn handle_model_response(
        &mut self,
        agent_config: AgentConfigData,
        iteration: u8,
        result: serde_json::Value,
    ) -> Vec<WorkflowAction> {
        let text = result
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let needs_tool_execution = result
            .get("needs_tool_execution")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let tool_calls: Option<Vec<ToolCallData>> = result
            .get("tool_calls")
            .and_then(|v| serde_json::from_value(v.clone()).ok());

        // Event: LlmCallCompleted
        let mut actions = vec![self.emit_event_activity(
            "llm_call_completed",
            json!({
                "session_id": self.session_id(),
                "iteration": iteration,
                "has_tool_calls": needs_tool_execution,
            }),
        )];

        if needs_tool_execution {
            if let Some(tool_calls) = tool_calls {
                if !tool_calls.is_empty() && iteration < agent_config.max_iterations {
                    // Schedule tool execution
                    actions.extend(self.transition_to_tool_execution(
                        agent_config,
                        tool_calls,
                        iteration,
                    ));
                    return actions;
                }
            }
        }

        // No tool calls or max iterations - complete
        self.state = WorkflowState::Completed {
            final_text: Some(text),
        };

        actions.push(self.emit_event_activity(
            "loop_completed",
            json!({
                "session_id": self.session_id(),
            }),
        ));
        actions.push(WorkflowAction::CompleteWorkflow {
            result: Some(json!({
                "status": "completed",
                "session_id": self.session_id(),
            })),
        });

        actions
    }

    fn handle_tool_completed(
        &mut self,
        activity_id: String,
        result: serde_json::Value,
    ) -> Vec<WorkflowAction> {
        // Extract current state
        let (mut pending_activities, mut completed, agent_config, tool_calls, iteration) =
            match &self.state {
                WorkflowState::ExecutingTools {
                    pending_activities,
                    completed,
                    agent_config,
                    tool_calls,
                    iteration,
                } => (
                    pending_activities.clone(),
                    completed.clone(),
                    agent_config.clone(),
                    tool_calls.clone(),
                    *iteration,
                ),
                _ => return vec![],
            };

        // Parse result
        let tool_result: ToolResultData =
            serde_json::from_value(result.get("result").cloned().unwrap_or(result.clone()))
                .unwrap_or(ToolResultData {
                    tool_call_id: "unknown".to_string(),
                    result: None,
                    error: Some("Failed to parse tool result".to_string()),
                });

        // Find the tool call for this result
        let tool_call = tool_calls
            .iter()
            .find(|tc| tc.id == tool_result.tool_call_id);

        // Emit completion event
        let mut actions = vec![self.emit_event_activity(
            "tool_execution_completed",
            json!({
                "session_id": self.session_id(),
                "tool_call_id": tool_result.tool_call_id,
                "tool_name": tool_call.map(|tc| tc.name.as_str()).unwrap_or("unknown"),
                "success": tool_result.error.is_none(),
            }),
        )];

        // Remove from pending, add to completed
        pending_activities.retain(|id| id != &activity_id);
        completed.push((activity_id, tool_result));

        // Check if all tools are done
        if pending_activities.is_empty() {
            // All tools completed, go to next iteration
            self.state = WorkflowState::PreModelCall {
                agent_config,
                iteration: iteration + 1,
            };
            actions.extend(self.transition_to_model_call());
        } else {
            // Still waiting for more tools
            self.state = WorkflowState::ExecutingTools {
                pending_activities,
                completed,
                agent_config,
                tool_calls,
                iteration,
            };
        }

        actions
    }
}

impl Workflow for SessionWorkflowV2 {
    fn workflow_type(&self) -> &'static str {
        "session_workflow_v2"
    }

    fn on_start(&mut self) -> Vec<WorkflowAction> {
        let activity_id = self.next_activity_id(activity_names::LOAD_AGENT);

        self.state = WorkflowState::LoadingAgent {
            pending_activity: activity_id.clone(),
        };

        vec![
            // Event: LoopStarted
            self.emit_event_activity(
                "loop_started",
                json!({
                    "session_id": self.session_id(),
                }),
            ),
            // Load agent config
            WorkflowAction::ScheduleActivity {
                activity_id,
                activity_type: activity_names::LOAD_AGENT.to_string(),
                input: json!({
                    "agent_id": self.input.agent_id.to_string(),
                }),
            },
        ]
    }

    fn on_activity_completed(
        &mut self,
        activity_id: &str,
        result: serde_json::Value,
    ) -> Vec<WorkflowAction> {
        let state = self.state.clone();

        match state {
            WorkflowState::LoadingAgent { pending_activity } if pending_activity == activity_id => {
                self.handle_agent_loaded(result)
            }

            WorkflowState::CallingModel {
                pending_activity,
                agent_config,
                iteration,
            } if pending_activity == activity_id => {
                self.handle_model_response(agent_config, iteration, result)
            }

            WorkflowState::ExecutingTools {
                ref pending_activities,
                ..
            } if pending_activities.contains(&activity_id.to_string()) => {
                self.handle_tool_completed(activity_id.to_string(), result)
            }

            // Ignore unrelated activity completions (e.g., emit_event)
            _ => vec![],
        }
    }

    fn on_activity_failed(&mut self, activity_id: &str, error: &str) -> Vec<WorkflowAction> {
        // Ignore emit_event failures
        if activity_id.starts_with(activity_names::EMIT_EVENT) {
            return vec![];
        }

        self.state = WorkflowState::Failed {
            error: error.to_string(),
        };

        vec![
            self.emit_event_activity(
                "loop_error",
                json!({
                    "session_id": self.session_id(),
                    "error": error,
                }),
            ),
            WorkflowAction::FailWorkflow {
                reason: error.to_string(),
            },
        ]
    }

    fn is_completed(&self) -> bool {
        matches!(
            self.state,
            WorkflowState::Completed { .. } | WorkflowState::Failed { .. }
        )
    }
}

// Helper methods for state transitions
impl SessionWorkflowV2 {
    fn transition_to_model_call(&mut self) -> Vec<WorkflowAction> {
        let (agent_config, iteration) = match &self.state {
            WorkflowState::PreModelCall {
                agent_config,
                iteration,
            } => (agent_config.clone(), *iteration),
            _ => return vec![],
        };

        let activity_id = self.next_activity_id(activity_names::CALL_MODEL);

        self.state = WorkflowState::CallingModel {
            pending_activity: activity_id.clone(),
            agent_config: agent_config.clone(),
            iteration,
        };

        vec![
            // Event: LlmCallStarted
            self.emit_event_activity(
                "llm_call_started",
                json!({
                    "session_id": self.session_id(),
                    "iteration": iteration,
                }),
            ),
            // Call model - atoms load messages from DB
            WorkflowAction::ScheduleActivity {
                activity_id,
                activity_type: activity_names::CALL_MODEL.to_string(),
                input: json!({
                    "session_id": self.session_id(),
                    "agent_config": agent_config,
                }),
            },
        ]
    }

    fn transition_to_tool_execution(
        &mut self,
        agent_config: AgentConfigData,
        tool_calls: Vec<ToolCallData>,
        iteration: u8,
    ) -> Vec<WorkflowAction> {
        let mut actions = Vec::new();
        let mut pending_activities = Vec::new();

        // Schedule one activity per tool call
        for tool_call in &tool_calls {
            let activity_id = self.next_activity_id(activity_names::EXECUTE_TOOL);
            pending_activities.push(activity_id.clone());

            // Event: ToolExecutionStarted
            actions.push(self.emit_event_activity(
                "tool_execution_started",
                json!({
                    "session_id": self.session_id(),
                    "tool_call_id": tool_call.id,
                    "tool_name": tool_call.name,
                }),
            ));

            // Schedule tool execution
            actions.push(WorkflowAction::ScheduleActivity {
                activity_id,
                activity_type: activity_names::EXECUTE_TOOL.to_string(),
                input: json!({
                    "session_id": self.session_id(),
                    "tool_call": tool_call,
                    "tool_definitions": agent_config.tools,
                }),
            });
        }

        self.state = WorkflowState::ExecutingTools {
            pending_activities,
            completed: Vec::new(),
            agent_config,
            tool_calls,
            iteration,
        };

        actions
    }
}

impl WorkflowInput for SessionWorkflowV2 {
    const WORKFLOW_TYPE: &'static str = "session_workflow_v2";
    type Input = SessionWorkflowV2Input;

    fn from_input(input: Self::Input) -> Self {
        SessionWorkflowV2::new(input)
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
        let input = SessionWorkflowV2Input {
            session_id: Uuid::now_v7(),
            agent_id: Uuid::now_v7(),
        };

        let mut workflow = SessionWorkflowV2::new(input);
        let actions = workflow.on_start();

        // Should emit start event and load agent
        assert!(actions.len() >= 2);
        assert!(find_activity_id(&actions, "load-agent").is_some());
        assert!(!workflow.is_completed());
    }

    #[test]
    fn test_agent_loaded_goes_to_model_call() {
        let input = SessionWorkflowV2Input {
            session_id: Uuid::now_v7(),
            agent_id: Uuid::now_v7(),
        };

        let mut workflow = SessionWorkflowV2::new(input);
        let actions = workflow.on_start();
        let load_agent_id = find_activity_id(&actions, "load-agent").unwrap();

        // Agent loaded - should go directly to call-model (no load-messages)
        let actions = workflow.on_activity_completed(
            &load_agent_id,
            json!({
                "model": "gpt-5.2",
                "tools": [],
                "max_iterations": 5
            }),
        );

        // Should schedule call-model (not load-messages)
        assert!(find_activity_id(&actions, "call-model").is_some());
        assert!(find_activity_id(&actions, "load-messages").is_none());
    }

    #[test]
    fn test_model_with_tool_calls() {
        let input = SessionWorkflowV2Input {
            session_id: Uuid::now_v7(),
            agent_id: Uuid::now_v7(),
        };

        let mut workflow = SessionWorkflowV2::new(input);
        let actions = workflow.on_start();
        let load_agent_id = find_activity_id(&actions, "load-agent").unwrap();

        let actions = workflow.on_activity_completed(
            &load_agent_id,
            json!({
                "model": "gpt-5.2",
                "tools": [{"name": "get_time", "description": "Get time", "parameters": {}}],
                "max_iterations": 5
            }),
        );
        let call_model_id = find_activity_id(&actions, "call-model").unwrap();

        // LLM returns tool calls
        let actions = workflow.on_activity_completed(
            &call_model_id,
            json!({
                "text": "Let me check.",
                "tool_calls": [{"id": "call_1", "name": "get_time", "arguments": {}}],
                "needs_tool_execution": true
            }),
        );

        // Should schedule execute-tool (not execute-tools)
        assert!(find_activity_id(&actions, "execute-tool").is_some());
    }

    #[test]
    fn test_completion_without_tools() {
        let input = SessionWorkflowV2Input {
            session_id: Uuid::now_v7(),
            agent_id: Uuid::now_v7(),
        };

        let mut workflow = SessionWorkflowV2::new(input);
        let actions = workflow.on_start();
        let load_agent_id = find_activity_id(&actions, "load-agent").unwrap();

        let actions = workflow.on_activity_completed(
            &load_agent_id,
            json!({
                "model": "gpt-5.2",
                "tools": [],
                "max_iterations": 5
            }),
        );
        let call_model_id = find_activity_id(&actions, "call-model").unwrap();

        // LLM returns no tool calls
        let actions = workflow.on_activity_completed(
            &call_model_id,
            json!({
                "text": "Hello!",
                "tool_calls": null,
                "needs_tool_execution": false
            }),
        );

        // Should complete
        assert!(workflow.is_completed());
        assert!(actions
            .iter()
            .any(|a| matches!(a, WorkflowAction::CompleteWorkflow { .. })));
    }
}

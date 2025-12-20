// Session Workflow V2 - Agent Loop Orchestration
//
// Design: Simpler state machine leveraging everruns-core primitives
//
// Key differences from v1:
// - Uses StepInput/StepOutput from everruns-core for step decomposition
// - Uses LoopEvent from everruns-core for event emission
// - Always loads messages from storage
// - Parallel tool execution within a single activity
// - Cleaner state transitions with explicit event points
//
// Events emitted:
// - LoopStarted: On workflow start
// - LlmCallStarted/LlmCallCompleted: Before/after model call
// - ToolExecutionStarted/ToolExecutionCompleted: Before/after each tool (parallel)
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

/// Simple message data for serialization
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
            model: "gpt-4".to_string(),
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
// Workflow State
// ============================================================================

/// Workflow states - linear progression with loop
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WorkflowState {
    /// Initial: emit start event, load agent config
    Init,

    /// Loading agent configuration
    LoadingAgent { pending_activity: String },

    /// Loading session messages (or using provided ones)
    LoadingMessages {
        pending_activity: String,
        agent_config: AgentConfigData,
    },

    /// Ready to call LLM
    PreModelCall {
        agent_config: AgentConfigData,
        messages: Vec<MessageData>,
        iteration: u8,
    },

    /// Waiting for LLM response
    CallingModel {
        pending_activity: String,
        agent_config: AgentConfigData,
        messages: Vec<MessageData>,
        iteration: u8,
    },

    /// LLM returned tool calls, ready to execute
    PreToolExecution {
        agent_config: AgentConfigData,
        messages: Vec<MessageData>,
        tool_calls: Vec<ToolCallData>,
        iteration: u8,
    },

    /// Executing tools (parallel)
    ExecutingTools {
        pending_activity: String,
        agent_config: AgentConfigData,
        messages: Vec<MessageData>,
        tool_calls: Vec<ToolCallData>,
        iteration: u8,
    },

    /// Post tool execution, decide next step
    PostToolExecution {
        agent_config: AgentConfigData,
        messages: Vec<MessageData>,
        iteration: u8,
    },

    /// Saving final response
    SavingResponse {
        pending_activity: String,
        final_text: String,
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
    pub const LOAD_MESSAGES: &str = "load-messages";
    pub const CALL_MODEL: &str = "call-model";
    pub const EXECUTE_TOOLS: &str = "execute-tools";
    pub const SAVE_MESSAGE: &str = "save-message";
    pub const EMIT_EVENT: &str = "emit-event";
}

// ============================================================================
// Workflow Implementation
// ============================================================================

/// V2 Session Workflow
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
    // State Handlers - Called when activity results arrive
    // =========================================================================

    fn handle_agent_loaded(&mut self, result: serde_json::Value) -> Vec<WorkflowAction> {
        let agent_config: AgentConfigData = serde_json::from_value(result).unwrap_or_default();

        let activity_id = self.next_activity_id(activity_names::LOAD_MESSAGES);
        self.state = WorkflowState::LoadingMessages {
            pending_activity: activity_id.clone(),
            agent_config,
        };

        vec![WorkflowAction::ScheduleActivity {
            activity_id,
            activity_type: activity_names::LOAD_MESSAGES.to_string(),
            input: json!({
                "session_id": self.session_id(),
            }),
        }]
    }

    fn handle_messages_loaded(
        &mut self,
        agent_config: AgentConfigData,
        result: serde_json::Value,
    ) -> Vec<WorkflowAction> {
        let messages: Vec<MessageData> = serde_json::from_value(result).unwrap_or_default();

        self.state = WorkflowState::PreModelCall {
            agent_config,
            messages,
            iteration: 1,
        };
        self.transition_to_model_call()
    }

    fn handle_model_response(
        &mut self,
        agent_config: AgentConfigData,
        mut messages: Vec<MessageData>,
        iteration: u8,
        result: serde_json::Value,
    ) -> Vec<WorkflowAction> {
        let text = result
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let tool_calls: Option<Vec<ToolCallData>> = result
            .get("tool_calls")
            .and_then(|v| serde_json::from_value(v.clone()).ok());

        // Add assistant message to history
        messages.push(MessageData {
            role: "assistant".to_string(),
            content: text.clone(),
            tool_calls: tool_calls.clone(),
            tool_call_id: None,
        });

        // Event: LlmCallCompleted
        let mut actions = vec![self.emit_event_activity(
            "llm_call_completed",
            json!({
                "session_id": self.session_id(),
                "iteration": iteration,
                "has_tool_calls": tool_calls.is_some(),
            }),
        )];

        if let Some(tool_calls) = tool_calls {
            if !tool_calls.is_empty() && iteration < agent_config.max_iterations {
                // Has tool calls, execute them
                self.state = WorkflowState::PreToolExecution {
                    agent_config,
                    messages,
                    tool_calls,
                    iteration,
                };
                actions.extend(self.transition_to_tool_execution());
                return actions;
            }
        }

        // No tool calls or max iterations, save and complete
        self.transition_to_save_and_complete(text, actions)
    }

    fn handle_tools_executed(
        &mut self,
        agent_config: AgentConfigData,
        mut messages: Vec<MessageData>,
        tool_calls: Vec<ToolCallData>,
        iteration: u8,
        result: serde_json::Value,
    ) -> Vec<WorkflowAction> {
        let results: Vec<ToolResultData> = serde_json::from_value(result).unwrap_or_default();

        // Add tool results to messages
        for (tool_call, result) in tool_calls.iter().zip(results.iter()) {
            messages.push(MessageData {
                role: "tool_call".to_string(),
                content: serde_json::to_string(&tool_call.arguments).unwrap_or_default(),
                tool_calls: None,
                tool_call_id: Some(tool_call.id.clone()),
            });

            messages.push(MessageData {
                role: "tool_result".to_string(),
                content: result
                    .result
                    .as_ref()
                    .map(|v| serde_json::to_string(v).unwrap_or_default())
                    .or(result.error.clone())
                    .unwrap_or_default(),
                tool_calls: None,
                tool_call_id: Some(result.tool_call_id.clone()),
            });
        }

        // Events: ToolExecutionCompleted for each tool
        let mut actions: Vec<WorkflowAction> = tool_calls
            .iter()
            .zip(results.iter())
            .map(|(tc, res)| {
                self.emit_event_activity(
                    "tool_execution_completed",
                    json!({
                        "session_id": self.session_id(),
                        "tool_call_id": tc.id,
                        "tool_name": tc.name,
                        "success": res.error.is_none(),
                    }),
                )
            })
            .collect();

        // Move to next iteration
        self.state = WorkflowState::PreModelCall {
            agent_config,
            messages,
            iteration: iteration + 1,
        };
        actions.extend(self.transition_to_model_call());
        actions
    }

    fn handle_response_saved(&mut self, final_text: String) -> Vec<WorkflowAction> {
        self.state = WorkflowState::Completed {
            final_text: Some(final_text),
        };

        vec![
            self.emit_event_activity(
                "loop_completed",
                json!({
                    "session_id": self.session_id(),
                }),
            ),
            WorkflowAction::CompleteWorkflow {
                result: Some(json!({
                    "status": "completed",
                    "session_id": self.session_id(),
                })),
            },
        ]
    }
}

impl Workflow for SessionWorkflowV2 {
    fn workflow_type(&self) -> &'static str {
        "session_workflow_v2"
    }

    fn on_start(&mut self) -> Vec<WorkflowAction> {
        // Emit LoopStarted event and schedule LoadAgent
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

    /// Dispatches activity results to the appropriate state handler.
    /// This is the Temporal callback interface - internally we use clearer handler names.
    fn on_activity_completed(
        &mut self,
        activity_id: &str,
        result: serde_json::Value,
    ) -> Vec<WorkflowAction> {
        // Clone state data needed for handlers (to avoid borrow issues)
        let state = self.state.clone();

        match state {
            WorkflowState::LoadingAgent { pending_activity } if pending_activity == activity_id => {
                self.handle_agent_loaded(result)
            }

            WorkflowState::LoadingMessages {
                pending_activity,
                agent_config,
            } if pending_activity == activity_id => {
                self.handle_messages_loaded(agent_config, result)
            }

            WorkflowState::CallingModel {
                pending_activity,
                agent_config,
                messages,
                iteration,
            } if pending_activity == activity_id => {
                self.handle_model_response(agent_config, messages, iteration, result)
            }

            WorkflowState::ExecutingTools {
                pending_activity,
                agent_config,
                messages,
                tool_calls,
                iteration,
            } if pending_activity == activity_id => {
                self.handle_tools_executed(agent_config, messages, tool_calls, iteration, result)
            }

            WorkflowState::SavingResponse {
                pending_activity,
                final_text,
            } if pending_activity == activity_id => self.handle_response_saved(final_text),

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
            // Event: LoopError
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
        let (agent_config, messages, iteration) = match &self.state {
            WorkflowState::PreModelCall {
                agent_config,
                messages,
                iteration,
            } => (agent_config.clone(), messages.clone(), *iteration),
            _ => return vec![],
        };

        let activity_id = self.next_activity_id(activity_names::CALL_MODEL);

        self.state = WorkflowState::CallingModel {
            pending_activity: activity_id.clone(),
            agent_config: agent_config.clone(),
            messages: messages.clone(),
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
            // Call model
            WorkflowAction::ScheduleActivity {
                activity_id,
                activity_type: activity_names::CALL_MODEL.to_string(),
                input: json!({
                    "session_id": self.session_id(),
                    "model": agent_config.model,
                    "messages": messages,
                    "tools": agent_config.tools,
                }),
            },
        ]
    }

    fn transition_to_tool_execution(&mut self) -> Vec<WorkflowAction> {
        let (agent_config, messages, tool_calls, iteration) = match &self.state {
            WorkflowState::PreToolExecution {
                agent_config,
                messages,
                tool_calls,
                iteration,
            } => (
                agent_config.clone(),
                messages.clone(),
                tool_calls.clone(),
                *iteration,
            ),
            _ => return vec![],
        };

        let activity_id = self.next_activity_id(activity_names::EXECUTE_TOOLS);

        self.state = WorkflowState::ExecutingTools {
            pending_activity: activity_id.clone(),
            agent_config: agent_config.clone(),
            messages: messages.clone(),
            tool_calls: tool_calls.clone(),
            iteration,
        };

        // Events: ToolExecutionStarted for each tool
        let mut actions: Vec<WorkflowAction> = tool_calls
            .iter()
            .map(|tc| {
                self.emit_event_activity(
                    "tool_execution_started",
                    json!({
                        "session_id": self.session_id(),
                        "tool_call_id": tc.id,
                        "tool_name": tc.name,
                    }),
                )
            })
            .collect();

        // Execute all tools in one activity (parallel execution inside activity)
        actions.push(WorkflowAction::ScheduleActivity {
            activity_id,
            activity_type: activity_names::EXECUTE_TOOLS.to_string(),
            input: json!({
                "session_id": self.session_id(),
                "tool_calls": tool_calls,
                "tools": agent_config.tools,
            }),
        });

        actions
    }

    fn transition_to_save_and_complete(
        &mut self,
        final_text: String,
        mut actions: Vec<WorkflowAction>,
    ) -> Vec<WorkflowAction> {
        let activity_id = self.next_activity_id(activity_names::SAVE_MESSAGE);

        self.state = WorkflowState::SavingResponse {
            pending_activity: activity_id.clone(),
            final_text: final_text.clone(),
        };

        actions.push(WorkflowAction::ScheduleActivity {
            activity_id,
            activity_type: activity_names::SAVE_MESSAGE.to_string(),
            input: json!({
                "session_id": self.session_id(),
                "role": "assistant",
                "content": { "text": final_text },
            }),
        });

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
// Example Usage (for documentation)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to find activity ID by type prefix
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

    /// Example: Start a new session
    #[test]
    fn example_new_session() {
        let input = SessionWorkflowV2Input {
            session_id: Uuid::now_v7(),
            agent_id: Uuid::now_v7(),
        };

        let mut workflow = SessionWorkflowV2::new(input);
        let actions = workflow.on_start();

        // Should emit start event and load agent
        assert!(actions.len() >= 2);
        assert!(!workflow.is_completed());
    }

    /// Example: Messages loaded from storage
    #[test]
    fn example_messages_from_storage() {
        let input = SessionWorkflowV2Input {
            session_id: Uuid::now_v7(),
            agent_id: Uuid::now_v7(),
        };

        let mut workflow = SessionWorkflowV2::new(input);
        let actions = workflow.on_start();
        let load_agent_id = find_activity_id(&actions, "load-agent").unwrap();

        // Agent loaded
        let actions = workflow.on_activity_completed(
            &load_agent_id,
            json!({
                "model": "gpt-4",
                "tools": [],
                "max_iterations": 5
            }),
        );

        // Should schedule load-messages
        assert!(actions
            .iter()
            .any(|a| matches!(a, WorkflowAction::ScheduleActivity { activity_type, .. } if activity_type == "load-messages")));
    }

    /// Example: Simulate tool call flow
    #[test]
    fn example_tool_call_flow() {
        let input = SessionWorkflowV2Input {
            session_id: Uuid::now_v7(),
            agent_id: Uuid::now_v7(),
        };

        let mut workflow = SessionWorkflowV2::new(input);
        let actions = workflow.on_start();
        let load_agent_id = find_activity_id(&actions, "load-agent").unwrap();

        // Agent loaded
        let actions = workflow.on_activity_completed(
            &load_agent_id,
            json!({
                "model": "gpt-4",
                "tools": [{"name": "get_time", "description": "Get current time", "parameters": {}}],
                "max_iterations": 5
            }),
        );
        let load_messages_id = find_activity_id(&actions, "load-messages").unwrap();

        // Messages loaded
        let actions = workflow.on_activity_completed(
            &load_messages_id,
            json!([{
                "role": "user",
                "content": "What time is it?"
            }]),
        );
        let call_model_id = find_activity_id(&actions, "call-model").unwrap();

        // LLM returns tool call
        let actions = workflow.on_activity_completed(
            &call_model_id,
            json!({
                "text": "Let me check the time.",
                "tool_calls": [{
                    "id": "call_123",
                    "name": "get_time",
                    "arguments": {}
                }]
            }),
        );

        // Should schedule tool execution
        assert!(actions.iter().any(
            |a| matches!(a, WorkflowAction::ScheduleActivity { activity_type, .. } if activity_type == "execute-tools")
        ));
    }
}

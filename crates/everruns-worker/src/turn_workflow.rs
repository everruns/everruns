// Turn Workflow - Turn-based Agent Loop Orchestration
//
// This workflow implements the turn-based execution model:
// 1. InputAtom: Record user message and start turn
// 2. ReasonAtom: LLM call with context preparation
// 3. ActAtom: Parallel tool execution (if needed)
// 4. Loop: Repeat Reason→Act until no more tool calls
//
// Key design principles:
// - Each turn has a unique turn_id for tracking
// - AtomContext is passed to all atoms for correlation
// - Error handling is "normal" - failures are captured, not propagated
// - Cancellation is supported at any point

use everruns_core::atoms::AtomContext;
use everruns_core::{ToolCall, ToolDefinition};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::activities::{ActResult, ReasonResult};
use crate::traits::{Workflow, WorkflowInput};
use crate::types::WorkflowAction;

// ============================================================================
// Input/Output Types
// ============================================================================

/// Workflow input for starting a turn
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnWorkflowInput {
    /// Session ID
    pub session_id: Uuid,
    /// Agent ID for loading configuration
    pub agent_id: Uuid,
    /// Input message ID (the user message that triggered this turn)
    pub input_message_id: Uuid,
}

// ============================================================================
// Workflow State
// ============================================================================

/// Workflow states for the turn-based execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TurnState {
    /// Initial state
    Init,

    /// Processing user input (InputAtom)
    ProcessingInput { pending_activity: String },

    /// Calling LLM (ReasonAtom)
    Reasoning {
        pending_activity: String,
        iteration: usize,
        tool_definitions: Option<Vec<ToolDefinition>>,
        max_iterations: Option<usize>,
    },

    /// Executing tools (ActAtom)
    Acting {
        pending_activity: String,
        iteration: usize,
        tool_definitions: Vec<ToolDefinition>,
        max_iterations: usize,
    },

    /// Terminal: turn completed successfully
    Completed { final_text: Option<String> },

    /// Terminal: turn failed
    Failed { error: String },
}

// ============================================================================
// Activity Names
// ============================================================================

mod activity_names {
    pub const INPUT: &str = "input";
    pub const REASON: &str = "reason";
    pub const ACT: &str = "act";
}

// ============================================================================
// Workflow Implementation
// ============================================================================

/// Turn Workflow - orchestrates a single turn (user input → final response)
#[derive(Debug)]
pub struct TurnWorkflow {
    input: TurnWorkflowInput,
    turn_id: Uuid,
    state: TurnState,
    activity_seq: u32,
}

impl TurnWorkflow {
    pub fn new(input: TurnWorkflowInput) -> Self {
        Self {
            input,
            turn_id: Uuid::now_v7(),
            state: TurnState::Init,
            activity_seq: 0,
        }
    }

    fn next_activity_id(&mut self, prefix: &str) -> String {
        self.activity_seq += 1;
        format!("{}-{}", prefix, self.activity_seq)
    }

    /// Create an AtomContext for the current execution
    fn create_context(&self) -> AtomContext {
        AtomContext::new(
            self.input.session_id,
            self.turn_id,
            self.input.input_message_id,
        )
    }

    // =========================================================================
    // State Transitions
    // =========================================================================

    fn transition_to_input(&mut self) -> Vec<WorkflowAction> {
        let activity_id = self.next_activity_id(activity_names::INPUT);
        let context = self.create_context();

        self.state = TurnState::ProcessingInput {
            pending_activity: activity_id.clone(),
        };

        vec![WorkflowAction::ScheduleActivity {
            activity_id,
            activity_type: activity_names::INPUT.to_string(),
            input: json!({
                "context": context,
            }),
        }]
    }

    fn transition_to_reason(
        &mut self,
        tool_definitions: Option<Vec<ToolDefinition>>,
        max_iterations: Option<usize>,
        iteration: usize,
    ) -> Vec<WorkflowAction> {
        let activity_id = self.next_activity_id(activity_names::REASON);
        let context = self.create_context();

        self.state = TurnState::Reasoning {
            pending_activity: activity_id.clone(),
            iteration,
            tool_definitions: tool_definitions.clone(),
            max_iterations,
        };

        vec![WorkflowAction::ScheduleActivity {
            activity_id,
            activity_type: activity_names::REASON.to_string(),
            input: json!({
                "context": context,
                "agent_id": self.input.agent_id,
            }),
        }]
    }

    fn transition_to_act(
        &mut self,
        tool_calls: Vec<ToolCall>,
        tool_definitions: Vec<ToolDefinition>,
        max_iterations: usize,
        iteration: usize,
    ) -> Vec<WorkflowAction> {
        let activity_id = self.next_activity_id(activity_names::ACT);
        let context = self.create_context();

        self.state = TurnState::Acting {
            pending_activity: activity_id.clone(),
            iteration,
            tool_definitions: tool_definitions.clone(),
            max_iterations,
        };

        vec![WorkflowAction::ScheduleActivity {
            activity_id,
            activity_type: activity_names::ACT.to_string(),
            input: json!({
                "context": context,
                "tool_calls": tool_calls,
                "tool_definitions": tool_definitions,
            }),
        }]
    }

    // =========================================================================
    // Result Handlers
    // =========================================================================

    fn handle_input_completed(&mut self, _result: serde_json::Value) -> Vec<WorkflowAction> {
        // Input processed, now start reasoning
        self.transition_to_reason(None, None, 1)
    }

    fn handle_reason_completed(
        &mut self,
        result: serde_json::Value,
        iteration: usize,
    ) -> Vec<WorkflowAction> {
        let output: ReasonResult = serde_json::from_value(result).unwrap_or_default();

        // Check if the LLM call failed
        if !output.success {
            // Store the error message as the final response
            self.state = TurnState::Completed {
                final_text: Some(output.text),
            };
            return vec![WorkflowAction::CompleteWorkflow {
                result: Some(json!({
                    "status": "completed",
                    "session_id": self.input.session_id,
                    "turn_id": self.turn_id,
                    "error": output.error,
                })),
            }];
        }

        // Check if we need to execute tools
        if output.has_tool_calls && !output.tool_calls.is_empty() {
            // Check iteration limit
            if iteration >= output.max_iterations {
                self.state = TurnState::Completed {
                    final_text: Some(output.text),
                };
                return vec![WorkflowAction::CompleteWorkflow {
                    result: Some(json!({
                        "status": "completed",
                        "session_id": self.input.session_id,
                        "turn_id": self.turn_id,
                        "reason": "max_iterations_reached",
                    })),
                }];
            }

            // Transition to tool execution
            return self.transition_to_act(
                output.tool_calls,
                output.tool_definitions,
                output.max_iterations,
                iteration,
            );
        }

        // No tool calls - complete the turn
        self.state = TurnState::Completed {
            final_text: Some(output.text),
        };

        vec![WorkflowAction::CompleteWorkflow {
            result: Some(json!({
                "status": "completed",
                "session_id": self.input.session_id,
                "turn_id": self.turn_id,
            })),
        }]
    }

    fn handle_act_completed(
        &mut self,
        result: serde_json::Value,
        iteration: usize,
        tool_definitions: Vec<ToolDefinition>,
        max_iterations: usize,
    ) -> Vec<WorkflowAction> {
        let _output: ActResult = serde_json::from_value(result).unwrap_or(ActResult {
            results: vec![],
            completed: true,
            success_count: 0,
            error_count: 0,
        });

        // After tool execution, call reason again
        self.transition_to_reason(Some(tool_definitions), Some(max_iterations), iteration + 1)
    }
}

impl Workflow for TurnWorkflow {
    fn workflow_type(&self) -> &'static str {
        "turn_workflow"
    }

    fn on_start(&mut self) -> Vec<WorkflowAction> {
        // Start by processing the input message
        self.transition_to_input()
    }

    fn on_activity_completed(
        &mut self,
        activity_id: &str,
        result: serde_json::Value,
    ) -> Vec<WorkflowAction> {
        let state = self.state.clone();

        match state {
            TurnState::ProcessingInput { pending_activity } if pending_activity == activity_id => {
                self.handle_input_completed(result)
            }

            TurnState::Reasoning {
                pending_activity,
                iteration,
                ..
            } if pending_activity == activity_id => self.handle_reason_completed(result, iteration),

            TurnState::Acting {
                pending_activity,
                iteration,
                tool_definitions,
                max_iterations,
            } if pending_activity == activity_id => {
                self.handle_act_completed(result, iteration, tool_definitions, max_iterations)
            }

            _ => vec![],
        }
    }

    fn on_activity_failed(&mut self, _activity_id: &str, error: &str) -> Vec<WorkflowAction> {
        self.state = TurnState::Failed {
            error: error.to_string(),
        };

        vec![WorkflowAction::FailWorkflow {
            reason: error.to_string(),
        }]
    }

    fn is_completed(&self) -> bool {
        matches!(
            self.state,
            TurnState::Completed { .. } | TurnState::Failed { .. }
        )
    }
}

impl WorkflowInput for TurnWorkflow {
    const WORKFLOW_TYPE: &'static str = "turn_workflow";
    type Input = TurnWorkflowInput;

    fn from_input(input: Self::Input) -> Self {
        TurnWorkflow::new(input)
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
        let input = TurnWorkflowInput {
            session_id: Uuid::now_v7(),
            agent_id: Uuid::now_v7(),
            input_message_id: Uuid::now_v7(),
        };

        let mut workflow = TurnWorkflow::new(input);
        let actions = workflow.on_start();

        // Should start with input activity
        assert_eq!(actions.len(), 1);
        assert!(find_activity_id(&actions, "input").is_some());
        assert!(!workflow.is_completed());
    }

    #[test]
    fn test_input_to_reason_transition() {
        let input = TurnWorkflowInput {
            session_id: Uuid::now_v7(),
            agent_id: Uuid::now_v7(),
            input_message_id: Uuid::now_v7(),
        };

        let mut workflow = TurnWorkflow::new(input);
        let actions = workflow.on_start();
        let input_id = find_activity_id(&actions, "input").unwrap();

        // Complete input activity
        let actions = workflow.on_activity_completed(
            &input_id,
            json!({
                "message": {"id": "msg_123", "role": "user", "content": []}
            }),
        );

        // Should transition to reason
        assert_eq!(actions.len(), 1);
        assert!(find_activity_id(&actions, "reason").is_some());
    }

    #[test]
    fn test_reason_without_tools_completes() {
        let input = TurnWorkflowInput {
            session_id: Uuid::now_v7(),
            agent_id: Uuid::now_v7(),
            input_message_id: Uuid::now_v7(),
        };

        let mut workflow = TurnWorkflow::new(input);
        let actions = workflow.on_start();
        let input_id = find_activity_id(&actions, "input").unwrap();

        // Complete input
        let actions = workflow.on_activity_completed(&input_id, json!({"message": {}}));
        let reason_id = find_activity_id(&actions, "reason").unwrap();

        // Complete reason without tool calls
        let actions = workflow.on_activity_completed(
            &reason_id,
            json!({
                "success": true,
                "text": "Hello!",
                "tool_calls": [],
                "has_tool_calls": false,
                "tool_definitions": [],
                "max_iterations": 10
            }),
        );

        // Should complete workflow
        assert_eq!(actions.len(), 1);
        assert!(workflow.is_completed());
        assert!(actions
            .iter()
            .any(|a| matches!(a, WorkflowAction::CompleteWorkflow { .. })));
    }

    #[test]
    fn test_reason_with_tools_transitions_to_act() {
        let input = TurnWorkflowInput {
            session_id: Uuid::now_v7(),
            agent_id: Uuid::now_v7(),
            input_message_id: Uuid::now_v7(),
        };

        let mut workflow = TurnWorkflow::new(input);
        let actions = workflow.on_start();
        let input_id = find_activity_id(&actions, "input").unwrap();

        // Complete input
        let actions = workflow.on_activity_completed(&input_id, json!({"message": {}}));
        let reason_id = find_activity_id(&actions, "reason").unwrap();

        // Complete reason with tool calls
        let actions = workflow.on_activity_completed(
            &reason_id,
            json!({
                "success": true,
                "text": "Let me check.",
                "tool_calls": [{"id": "call_1", "name": "get_time", "arguments": {}}],
                "has_tool_calls": true,
                "tool_definitions": [{"type": "builtin", "name": "get_time", "description": "Get time", "parameters": {}}],
                "max_iterations": 10
            }),
        );

        // Should transition to act
        assert_eq!(actions.len(), 1);
        assert!(find_activity_id(&actions, "act").is_some());
        assert!(!workflow.is_completed());
    }

    #[test]
    fn test_act_transitions_back_to_reason() {
        let input = TurnWorkflowInput {
            session_id: Uuid::now_v7(),
            agent_id: Uuid::now_v7(),
            input_message_id: Uuid::now_v7(),
        };

        let mut workflow = TurnWorkflow::new(input);

        // Fast-forward to Acting state
        let actions = workflow.on_start();
        let input_id = find_activity_id(&actions, "input").unwrap();
        let actions = workflow.on_activity_completed(&input_id, json!({"message": {}}));
        let reason_id = find_activity_id(&actions, "reason").unwrap();
        let actions = workflow.on_activity_completed(
            &reason_id,
            json!({
                "success": true,
                "text": "Checking...",
                "tool_calls": [{"id": "call_1", "name": "get_time", "arguments": {}}],
                "has_tool_calls": true,
                "tool_definitions": [{"type": "builtin", "name": "get_time", "description": "Get time", "parameters": {}}],
                "max_iterations": 10
            }),
        );
        let act_id = find_activity_id(&actions, "act").unwrap();

        // Complete act
        let actions = workflow.on_activity_completed(
            &act_id,
            json!({
                "results": [{"tool_call": {"id": "call_1"}, "result": {}, "success": true, "status": "success"}],
                "completed": true,
                "success_count": 1,
                "error_count": 0
            }),
        );

        // Should transition back to reason
        assert_eq!(actions.len(), 1);
        assert!(find_activity_id(&actions, "reason").is_some());
    }
}

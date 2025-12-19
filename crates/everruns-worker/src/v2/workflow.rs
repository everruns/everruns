// V2 Session Workflow - Temporal-based infinite loop state machine
//
// Decision: Workflow is an infinite loop representing the entire session
// Decision: States are: Initializing -> Waiting <-> Running -> Completed/Failed
// Decision: New messages arrive via Temporal signals
// Decision: Error if message arrives while running, accept if waiting
// Decision: Tool calls execute in parallel as separate activities
//
// This workflow produces commands that map directly to Temporal workflow commands:
// - ScheduleActivity -> temporal ScheduleActivity command
// - ScheduleParallelActivities -> multiple ScheduleActivity commands
// - WaitForSignal -> no commands (workflow waits for signal)
// - Complete -> CompleteWorkflowExecution command

use serde::{Deserialize, Serialize};
use tracing::{info, warn};
use uuid::Uuid;

use crate::temporal::{
    activity_names, CallLlmInput as TemporalCallLlmInput, ExecuteSingleToolInput,
    ExecuteSingleToolOutput, LoadAgentInput, LoadAgentOutput, MessageData, ToolCallData,
};

/// Maximum number of agent iterations per turn (prevent infinite loops)
const MAX_ITERATIONS_PER_TURN: u8 = 10;

/// Workflow state for the v2 session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum V2SessionState {
    /// Initial state - need to load agent config
    Starting,

    /// Loading agent configuration
    LoadingAgent { activity_seq: u32 },

    /// Waiting for user input (idle) - workflow blocks on signal
    Waiting {
        agent_config: LoadAgentOutput,
        messages: Vec<MessageData>,
        turn_count: u32,
    },

    /// Calling LLM
    CallingLlm {
        activity_seq: u32,
        agent_config: LoadAgentOutput,
        messages: Vec<MessageData>,
        turn_count: u32,
        iteration: u8,
    },

    /// Executing tool calls in parallel
    ExecutingTools {
        activity_seq: u32,
        agent_config: LoadAgentOutput,
        messages: Vec<MessageData>,
        turn_count: u32,
        iteration: u8,
        pending_tools: Vec<PendingToolActivity>,
        tool_results: Vec<ToolResultData>,
    },

    /// Workflow completed (terminal)
    Completed { turn_count: u32 },

    /// Workflow failed (terminal)
    Failed { error: String, turn_count: u32 },
}

/// Pending tool activity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingToolActivity {
    pub activity_id: String,
    pub tool_call_id: String,
    pub tool_name: String,
}

/// Tool result data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResultData {
    pub tool_call_id: String,
    pub result: Option<serde_json::Value>,
    pub error: Option<String>,
}

/// Workflow input
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct V2SessionWorkflowInput {
    pub session_id: Uuid,
    pub agent_id: Uuid,
}

/// Workflow output
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct V2SessionWorkflowOutput {
    pub session_id: Uuid,
    pub status: String,
    pub total_turns: u32,
    pub error: Option<String>,
}

/// Signal: new message from user
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct V2NewMessageSignal {
    pub content: String,
}

/// Signal: shutdown the session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct V2ShutdownSignal;

/// Workflow action (maps to Temporal commands)
#[derive(Debug)]
pub enum V2WorkflowAction {
    /// Schedule an activity
    ScheduleActivity {
        activity_id: String,
        activity_type: String,
        input: serde_json::Value,
    },
    /// Complete the workflow
    CompleteWorkflow { result: V2SessionWorkflowOutput },
    /// Fail the workflow
    FailWorkflow { reason: String },
    /// No action (waiting for more events)
    None,
}

/// The V2 session workflow state machine
#[derive(Debug)]
pub struct V2SessionWorkflow {
    input: V2SessionWorkflowInput,
    state: V2SessionState,
    activity_seq: u32,
}

impl V2SessionWorkflow {
    /// Create a new workflow instance
    pub fn new(input: V2SessionWorkflowInput) -> Self {
        Self {
            input,
            state: V2SessionState::Starting,
            activity_seq: 0,
        }
    }

    /// Get the current state
    pub fn state(&self) -> &V2SessionState {
        &self.state
    }

    /// Get the workflow input
    pub fn input(&self) -> &V2SessionWorkflowInput {
        &self.input
    }

    /// Check if workflow is waiting for a signal
    pub fn is_waiting_for_signal(&self) -> bool {
        matches!(self.state, V2SessionState::Waiting { .. })
    }

    /// Generate unique activity ID
    fn next_activity_id(&mut self, activity_type: &str) -> String {
        self.activity_seq += 1;
        format!("{}-{}", activity_type, self.activity_seq)
    }

    /// Process workflow start
    pub fn on_start(&mut self) -> Vec<V2WorkflowAction> {
        info!(
            session_id = %self.input.session_id,
            agent_id = %self.input.agent_id,
            "Starting v2 session workflow"
        );

        let activity_id = self.next_activity_id("load-agent");
        let seq = self.activity_seq;

        self.state = V2SessionState::LoadingAgent { activity_seq: seq };

        vec![V2WorkflowAction::ScheduleActivity {
            activity_id,
            activity_type: activity_names::LOAD_AGENT.to_string(),
            input: serde_json::to_value(&LoadAgentInput {
                agent_id: self.input.agent_id,
            })
            .unwrap(),
        }]
    }

    /// Process activity completion
    pub fn on_activity_completed(
        &mut self,
        activity_id: &str,
        result: serde_json::Value,
    ) -> Vec<V2WorkflowAction> {
        match &self.state {
            V2SessionState::LoadingAgent { .. } => self.handle_load_agent_completed(result),
            V2SessionState::CallingLlm { .. } => {
                self.handle_call_llm_completed(activity_id, result)
            }
            V2SessionState::ExecutingTools { .. } => {
                self.handle_tool_completed(activity_id, result)
            }
            _ => {
                warn!(
                    activity_id = %activity_id,
                    state = ?std::mem::discriminant(&self.state),
                    "Activity completed in unexpected state"
                );
                vec![V2WorkflowAction::None]
            }
        }
    }

    /// Process activity failure
    pub fn on_activity_failed(&mut self, activity_id: &str, error: &str) -> Vec<V2WorkflowAction> {
        warn!(
            activity_id = %activity_id,
            error = %error,
            "Activity failed"
        );

        let turn_count = self.get_turn_count();
        self.state = V2SessionState::Failed {
            error: error.to_string(),
            turn_count,
        };

        vec![V2WorkflowAction::FailWorkflow {
            reason: format!("Activity {} failed: {}", activity_id, error),
        }]
    }

    /// Process new message signal
    pub fn on_new_message(&mut self, signal: V2NewMessageSignal) -> Vec<V2WorkflowAction> {
        match &self.state {
            V2SessionState::Waiting {
                agent_config,
                messages,
                turn_count,
            } => {
                let agent_config = agent_config.clone();
                let mut messages = messages.clone();
                let turn_count = *turn_count + 1;

                info!(
                    session_id = %self.input.session_id,
                    turn = turn_count,
                    "New message received, starting turn"
                );

                // Add user message
                messages.push(MessageData {
                    role: "user".to_string(),
                    content: signal.content,
                    tool_calls: None,
                    tool_call_id: None,
                });

                // Start LLM call
                self.start_llm_call(agent_config, messages, turn_count, 1)
            }

            _ => {
                warn!(
                    session_id = %self.input.session_id,
                    state = ?std::mem::discriminant(&self.state),
                    "New message received in non-waiting state - ignoring"
                );
                vec![V2WorkflowAction::None]
            }
        }
    }

    /// Process shutdown signal
    pub fn on_shutdown(&mut self) -> Vec<V2WorkflowAction> {
        let turn_count = self.get_turn_count();
        info!(
            session_id = %self.input.session_id,
            turn_count = turn_count,
            "Shutdown signal received"
        );

        self.state = V2SessionState::Completed { turn_count };

        vec![V2WorkflowAction::CompleteWorkflow {
            result: V2SessionWorkflowOutput {
                session_id: self.input.session_id,
                status: "completed".to_string(),
                total_turns: turn_count,
                error: None,
            },
        }]
    }

    // =========================================================================
    // Private handlers
    // =========================================================================

    fn get_turn_count(&self) -> u32 {
        match &self.state {
            V2SessionState::Waiting { turn_count, .. } => *turn_count,
            V2SessionState::CallingLlm { turn_count, .. } => *turn_count,
            V2SessionState::ExecutingTools { turn_count, .. } => *turn_count,
            V2SessionState::Completed { turn_count } => *turn_count,
            V2SessionState::Failed { turn_count, .. } => *turn_count,
            _ => 0,
        }
    }

    fn handle_load_agent_completed(&mut self, result: serde_json::Value) -> Vec<V2WorkflowAction> {
        let agent_config: LoadAgentOutput = match serde_json::from_value(result) {
            Ok(config) => config,
            Err(e) => {
                self.state = V2SessionState::Failed {
                    error: format!("Failed to parse agent config: {}", e),
                    turn_count: 0,
                };
                return vec![V2WorkflowAction::FailWorkflow {
                    reason: format!("Failed to parse agent config: {}", e),
                }];
            }
        };

        info!(
            session_id = %self.input.session_id,
            agent_name = %agent_config.name,
            "Agent loaded, waiting for input"
        );

        self.state = V2SessionState::Waiting {
            agent_config,
            messages: Vec::new(),
            turn_count: 0,
        };

        // Return no actions - workflow now waits for signal
        vec![V2WorkflowAction::None]
    }

    fn start_llm_call(
        &mut self,
        agent_config: LoadAgentOutput,
        messages: Vec<MessageData>,
        turn_count: u32,
        iteration: u8,
    ) -> Vec<V2WorkflowAction> {
        let activity_id = self.next_activity_id("call-llm");
        let seq = self.activity_seq;

        let input = TemporalCallLlmInput {
            session_id: self.input.session_id,
            messages: messages.clone(),
            model_id: agent_config.model_id.clone(),
            system_prompt: agent_config.system_prompt.clone(),
            temperature: agent_config.temperature,
            max_tokens: agent_config.max_tokens,
            capability_ids: agent_config.capability_ids.clone(),
        };

        self.state = V2SessionState::CallingLlm {
            activity_seq: seq,
            agent_config,
            messages,
            turn_count,
            iteration,
        };

        vec![V2WorkflowAction::ScheduleActivity {
            activity_id,
            activity_type: activity_names::CALL_LLM.to_string(),
            input: serde_json::to_value(&input).unwrap(),
        }]
    }

    fn handle_call_llm_completed(
        &mut self,
        _activity_id: &str,
        result: serde_json::Value,
    ) -> Vec<V2WorkflowAction> {
        let (agent_config, mut messages, turn_count, iteration) = match &self.state {
            V2SessionState::CallingLlm {
                agent_config,
                messages,
                turn_count,
                iteration,
                ..
            } => (
                agent_config.clone(),
                messages.clone(),
                *turn_count,
                *iteration,
            ),
            _ => return vec![V2WorkflowAction::None],
        };

        // Parse LLM response
        let text = result
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let tool_calls: Vec<ToolCallData> = result
            .get("tool_calls")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        info!(
            session_id = %self.input.session_id,
            turn = turn_count,
            iteration = iteration,
            tool_call_count = tool_calls.len(),
            "LLM response received"
        );

        // Add assistant message
        let assistant_msg = MessageData {
            role: "assistant".to_string(),
            content: text.clone(),
            tool_calls: if tool_calls.is_empty() {
                None
            } else {
                Some(tool_calls.clone())
            },
            tool_call_id: None,
        };
        messages.push(assistant_msg);

        // If no tool calls, turn is complete - return to waiting
        if tool_calls.is_empty() {
            info!(
                session_id = %self.input.session_id,
                turn = turn_count,
                "Turn complete, waiting for next message"
            );

            self.state = V2SessionState::Waiting {
                agent_config,
                messages,
                turn_count,
            };

            return vec![V2WorkflowAction::None];
        }

        // Check iteration limit
        if iteration >= MAX_ITERATIONS_PER_TURN {
            warn!(
                session_id = %self.input.session_id,
                "Max iterations reached, completing turn"
            );

            self.state = V2SessionState::Waiting {
                agent_config,
                messages,
                turn_count,
            };

            return vec![V2WorkflowAction::None];
        }

        // Schedule tool executions in parallel
        let mut actions = Vec::new();
        let mut pending_tools = Vec::new();

        for tool_call in &tool_calls {
            let activity_id = self.next_activity_id(&format!("tool-{}", tool_call.name));

            let input = ExecuteSingleToolInput {
                session_id: self.input.session_id,
                tool_call: everruns_contracts::tools::ToolCall {
                    id: tool_call.id.clone(),
                    name: tool_call.name.clone(),
                    arguments: serde_json::from_str(&tool_call.arguments).unwrap_or_default(),
                },
                tool_definition_json: tool_call.tool_definition_json.clone(),
            };

            actions.push(V2WorkflowAction::ScheduleActivity {
                activity_id: activity_id.clone(),
                activity_type: activity_names::EXECUTE_SINGLE_TOOL.to_string(),
                input: serde_json::to_value(&input).unwrap(),
            });

            pending_tools.push(PendingToolActivity {
                activity_id,
                tool_call_id: tool_call.id.clone(),
                tool_name: tool_call.name.clone(),
            });
        }

        self.state = V2SessionState::ExecutingTools {
            activity_seq: self.activity_seq,
            agent_config,
            messages,
            turn_count,
            iteration,
            pending_tools,
            tool_results: Vec::new(),
        };

        actions
    }

    fn handle_tool_completed(
        &mut self,
        activity_id: &str,
        result: serde_json::Value,
    ) -> Vec<V2WorkflowAction> {
        let (
            agent_config,
            mut messages,
            turn_count,
            iteration,
            mut pending_tools,
            mut tool_results,
        ) = match &self.state {
            V2SessionState::ExecutingTools {
                agent_config,
                messages,
                turn_count,
                iteration,
                pending_tools,
                tool_results,
                ..
            } => (
                agent_config.clone(),
                messages.clone(),
                *turn_count,
                *iteration,
                pending_tools.clone(),
                tool_results.clone(),
            ),
            _ => return vec![V2WorkflowAction::None],
        };

        // Parse tool result
        let tool_output: ExecuteSingleToolOutput = match serde_json::from_value(result) {
            Ok(output) => output,
            Err(e) => {
                warn!(error = %e, "Failed to parse tool result");
                return vec![V2WorkflowAction::None];
            }
        };

        // Find and remove the completed tool
        let completed_idx = pending_tools
            .iter()
            .position(|pt| pt.activity_id == activity_id);

        let completed_tool = match completed_idx {
            Some(idx) => pending_tools.remove(idx),
            None => {
                warn!(activity_id = %activity_id, "Unknown tool activity completed");
                return vec![V2WorkflowAction::None];
            }
        };

        info!(
            session_id = %self.input.session_id,
            tool = %completed_tool.tool_name,
            remaining = pending_tools.len(),
            "Tool completed"
        );

        // Store result
        tool_results.push(ToolResultData {
            tool_call_id: completed_tool.tool_call_id.clone(),
            result: tool_output.result.result,
            error: tool_output.result.error,
        });

        // If more tools pending, wait
        if !pending_tools.is_empty() {
            self.state = V2SessionState::ExecutingTools {
                activity_seq: self.activity_seq,
                agent_config,
                messages,
                turn_count,
                iteration,
                pending_tools,
                tool_results,
            };
            return vec![V2WorkflowAction::None];
        }

        // All tools done - add tool result messages
        for tool_result in &tool_results {
            let content = if let Some(err) = &tool_result.error {
                format!("Error: {}", err)
            } else if let Some(result) = &tool_result.result {
                result.to_string()
            } else {
                "null".to_string()
            };

            messages.push(MessageData {
                role: "tool".to_string(),
                content,
                tool_calls: None,
                tool_call_id: Some(tool_result.tool_call_id.clone()),
            });
        }

        // Call LLM again with tool results
        self.start_llm_call(agent_config, messages, turn_count, iteration + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_input() -> V2SessionWorkflowInput {
        V2SessionWorkflowInput {
            session_id: Uuid::nil(),
            agent_id: Uuid::nil(),
        }
    }

    fn test_agent_config() -> LoadAgentOutput {
        LoadAgentOutput {
            agent_id: Uuid::nil(),
            name: "test-agent".to_string(),
            model_id: "gpt-4".to_string(),
            system_prompt: Some("You are helpful".to_string()),
            temperature: None,
            max_tokens: None,
            capability_ids: vec![],
        }
    }

    #[test]
    fn test_workflow_start() {
        let mut workflow = V2SessionWorkflow::new(test_input());
        let actions = workflow.on_start();

        assert_eq!(actions.len(), 1);
        assert!(matches!(
            &actions[0],
            V2WorkflowAction::ScheduleActivity { activity_type, .. }
            if activity_type == activity_names::LOAD_AGENT
        ));
        assert!(matches!(
            workflow.state(),
            V2SessionState::LoadingAgent { .. }
        ));
    }

    #[test]
    fn test_workflow_load_agent_then_waiting() {
        let mut workflow = V2SessionWorkflow::new(test_input());
        workflow.on_start();

        let agent_config = test_agent_config();
        let actions = workflow
            .on_activity_completed("load-agent-1", serde_json::to_value(&agent_config).unwrap());

        // Should return None action (waiting for signal)
        assert_eq!(actions.len(), 1);
        assert!(matches!(&actions[0], V2WorkflowAction::None));
        assert!(workflow.is_waiting_for_signal());
    }

    #[test]
    fn test_workflow_new_message_starts_llm() {
        let mut workflow = V2SessionWorkflow::new(test_input());
        workflow.on_start();

        let agent_config = test_agent_config();
        workflow
            .on_activity_completed("load-agent-1", serde_json::to_value(&agent_config).unwrap());

        let signal = V2NewMessageSignal {
            content: "Hello".to_string(),
        };
        let actions = workflow.on_new_message(signal);

        assert_eq!(actions.len(), 1);
        assert!(matches!(
            &actions[0],
            V2WorkflowAction::ScheduleActivity { activity_type, .. }
            if activity_type == activity_names::CALL_LLM
        ));
        assert!(matches!(
            workflow.state(),
            V2SessionState::CallingLlm { .. }
        ));
    }

    #[test]
    fn test_workflow_llm_no_tools_returns_to_waiting() {
        let mut workflow = V2SessionWorkflow::new(test_input());
        workflow.on_start();

        let agent_config = test_agent_config();
        workflow
            .on_activity_completed("load-agent-1", serde_json::to_value(&agent_config).unwrap());

        workflow.on_new_message(V2NewMessageSignal {
            content: "Hello".to_string(),
        });

        // LLM responds without tools
        let llm_result = serde_json::json!({
            "text": "Hello! How can I help?",
            "tool_calls": []
        });
        let actions = workflow.on_activity_completed("call-llm-2", llm_result);

        // Should return to waiting
        assert_eq!(actions.len(), 1);
        assert!(matches!(&actions[0], V2WorkflowAction::None));
        assert!(workflow.is_waiting_for_signal());

        // Check turn count
        if let V2SessionState::Waiting {
            turn_count,
            messages,
            ..
        } = workflow.state()
        {
            assert_eq!(*turn_count, 1);
            assert_eq!(messages.len(), 2); // user + assistant
        }
    }

    #[test]
    fn test_workflow_llm_with_tools() {
        let mut workflow = V2SessionWorkflow::new(test_input());
        workflow.on_start();

        let agent_config = test_agent_config();
        workflow
            .on_activity_completed("load-agent-1", serde_json::to_value(&agent_config).unwrap());

        workflow.on_new_message(V2NewMessageSignal {
            content: "What time is it?".to_string(),
        });

        // LLM responds with tool call
        let llm_result = serde_json::json!({
            "text": "Let me check",
            "tool_calls": [{
                "id": "call_123",
                "name": "get_time",
                "arguments": "{}"
            }]
        });
        let actions = workflow.on_activity_completed("call-llm-2", llm_result);

        // Should schedule tool activity
        assert_eq!(actions.len(), 1);
        assert!(matches!(
            &actions[0],
            V2WorkflowAction::ScheduleActivity { activity_type, .. }
            if activity_type == activity_names::EXECUTE_SINGLE_TOOL
        ));
        assert!(matches!(
            workflow.state(),
            V2SessionState::ExecutingTools { .. }
        ));
    }

    #[test]
    fn test_workflow_shutdown() {
        let mut workflow = V2SessionWorkflow::new(test_input());
        workflow.on_start();

        let agent_config = test_agent_config();
        workflow
            .on_activity_completed("load-agent-1", serde_json::to_value(&agent_config).unwrap());

        let actions = workflow.on_shutdown();

        assert_eq!(actions.len(), 1);
        assert!(matches!(
            &actions[0],
            V2WorkflowAction::CompleteWorkflow { result }
            if result.status == "completed"
        ));
    }

    #[test]
    fn test_workflow_message_ignored_while_running() {
        let mut workflow = V2SessionWorkflow::new(test_input());
        workflow.on_start();

        let agent_config = test_agent_config();
        workflow
            .on_activity_completed("load-agent-1", serde_json::to_value(&agent_config).unwrap());

        workflow.on_new_message(V2NewMessageSignal {
            content: "Hello".to_string(),
        });

        // Now in CallingLlm state - message should be ignored
        let actions = workflow.on_new_message(V2NewMessageSignal {
            content: "Another message".to_string(),
        });

        assert_eq!(actions.len(), 1);
        assert!(matches!(&actions[0], V2WorkflowAction::None));
    }

    #[test]
    fn test_workflow_multiple_turns() {
        let mut workflow = V2SessionWorkflow::new(test_input());
        workflow.on_start();

        let agent_config = test_agent_config();
        workflow
            .on_activity_completed("load-agent-1", serde_json::to_value(&agent_config).unwrap());

        // Turn 1
        workflow.on_new_message(V2NewMessageSignal {
            content: "Hello".to_string(),
        });
        workflow.on_activity_completed(
            "call-llm-2",
            serde_json::json!({"text": "Hi!", "tool_calls": []}),
        );

        // Turn 2
        workflow.on_new_message(V2NewMessageSignal {
            content: "Bye".to_string(),
        });
        workflow.on_activity_completed(
            "call-llm-3",
            serde_json::json!({"text": "Goodbye!", "tool_calls": []}),
        );

        if let V2SessionState::Waiting {
            turn_count,
            messages,
            ..
        } = workflow.state()
        {
            assert_eq!(*turn_count, 2);
            assert_eq!(messages.len(), 4); // 2 user + 2 assistant
        }
    }
}

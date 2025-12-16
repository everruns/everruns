// Temporal workflow implementations
// Decision: Workflows are state machines that produce commands in response to activations
//
// The agent run workflow orchestrates:
// 1. Loading agent configuration
// 2. Loading thread messages
// 3. Calling LLM (may return tool calls)
// 4. Executing tools if needed
// 5. Iterating until done or max iterations reached
//
// All state is deterministic and replayable from Temporal history.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use super::types::*;

/// Maximum number of tool iterations before forcing completion
const MAX_TOOL_ITERATIONS: u8 = 10;

/// Workflow state for agent run
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AgentRunWorkflowState {
    /// Initial state - need to load agent config
    Starting,
    /// Loading agent configuration
    LoadingAgent { activity_seq: u32 },
    /// Loading thread messages
    LoadingMessages {
        activity_seq: u32,
        agent_config: LoadAgentOutput,
    },
    /// Calling LLM
    CallingLlm {
        activity_seq: u32,
        agent_config: LoadAgentOutput,
        messages: Vec<MessageData>,
        iteration: u8,
    },
    /// Executing tool calls
    ExecutingTools {
        activity_seq: u32,
        agent_config: LoadAgentOutput,
        messages: Vec<MessageData>,
        tool_calls: Vec<ToolCallData>,
        iteration: u8,
    },
    /// Saving assistant message
    SavingMessage {
        activity_seq: u32,
        agent_config: LoadAgentOutput,
        messages: Vec<MessageData>,
        iteration: u8,
        has_more_tools: bool,
        assistant_text: String,
    },
    /// Updating run status
    UpdatingStatus {
        activity_seq: u32,
        final_status: String,
    },
    /// Workflow completed
    Completed,
    /// Workflow failed
    Failed { error: String },
}

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

/// Agent run workflow logic
#[derive(Debug)]
pub struct AgentRunWorkflow {
    /// Workflow input
    input: AgentRunWorkflowInput,
    /// Current state
    state: AgentRunWorkflowState,
    /// Activity sequence counter for generating unique IDs
    activity_seq: u32,
    /// Pending activity results (activity_id -> result)
    pending_results: HashMap<String, ActivityResult>,
}

/// Activity result from Temporal
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ActivityResult {
    Completed(serde_json::Value),
    Failed(String),
}

impl AgentRunWorkflow {
    /// Create a new workflow instance
    pub fn new(input: AgentRunWorkflowInput) -> Self {
        Self {
            input,
            state: AgentRunWorkflowState::Starting,
            activity_seq: 0,
            pending_results: HashMap::new(),
        }
    }

    /// Get the current workflow state
    pub fn state(&self) -> &AgentRunWorkflowState {
        &self.state
    }

    /// Get the workflow input
    pub fn input(&self) -> &AgentRunWorkflowInput {
        &self.input
    }

    /// Record an activity result
    pub fn record_activity_result(&mut self, activity_id: String, result: ActivityResult) {
        self.pending_results.insert(activity_id, result);
    }

    /// Generate a unique activity ID
    fn next_activity_id(&mut self, activity_type: &str) -> String {
        self.activity_seq += 1;
        format!("{}-{}", activity_type, self.activity_seq)
    }

    /// Process workflow start - called when workflow begins
    pub fn on_start(&mut self) -> Vec<WorkflowAction> {
        info!(
            run_id = %self.input.run_id,
            agent_id = %self.input.agent_id,
            "Starting agent run workflow"
        );

        // First, update status to running
        let activity_id = self.next_activity_id("update-status");
        let seq = self.activity_seq;

        let input = UpdateStatusInput {
            run_id: self.input.run_id,
            status: "running".to_string(),
            started_at: Some(chrono::Utc::now()),
            finished_at: None,
        };

        self.state = AgentRunWorkflowState::LoadingAgent { activity_seq: seq };

        // Schedule update status and load agent in parallel
        let status_activity_id = activity_id;
        let load_agent_activity_id = self.next_activity_id("load-agent");
        let load_agent_seq = self.activity_seq;
        self.state = AgentRunWorkflowState::LoadingAgent {
            activity_seq: load_agent_seq,
        };

        vec![
            WorkflowAction::ScheduleActivity {
                activity_id: status_activity_id,
                activity_type: activity_names::UPDATE_STATUS.to_string(),
                input: serde_json::to_value(&input).unwrap(),
            },
            WorkflowAction::ScheduleActivity {
                activity_id: load_agent_activity_id,
                activity_type: activity_names::LOAD_AGENT.to_string(),
                input: serde_json::to_value(&LoadAgentInput {
                    agent_id: self.input.agent_id,
                })
                .unwrap(),
            },
        ]
    }

    /// Process activity completion
    pub fn on_activity_completed(
        &mut self,
        activity_id: &str,
        result: serde_json::Value,
    ) -> Vec<WorkflowAction> {
        info!(
            activity_id = %activity_id,
            state = ?std::mem::discriminant(&self.state),
            "Activity completed"
        );

        match &self.state {
            AgentRunWorkflowState::LoadingAgent { .. } => {
                // Check which activity completed
                // We schedule update-status and load-agent in parallel at start
                // Only proceed when load-agent completes
                if activity_id.starts_with("update-status") {
                    // Status update completed, wait for load-agent
                    return vec![];
                }

                // Parse agent config
                let agent_config: LoadAgentOutput = match serde_json::from_value(result) {
                    Ok(config) => config,
                    Err(e) => {
                        return vec![
                            self.fail_workflow(&format!("Failed to parse agent config: {}", e))
                        ];
                    }
                };

                // Now load messages
                let activity_id = self.next_activity_id("load-messages");
                let seq = self.activity_seq;
                self.state = AgentRunWorkflowState::LoadingMessages {
                    activity_seq: seq,
                    agent_config,
                };

                vec![WorkflowAction::ScheduleActivity {
                    activity_id,
                    activity_type: activity_names::LOAD_MESSAGES.to_string(),
                    input: serde_json::to_value(&LoadMessagesInput {
                        thread_id: self.input.thread_id,
                    })
                    .unwrap(),
                }]
            }

            AgentRunWorkflowState::LoadingMessages { agent_config, .. } => {
                let agent_config = agent_config.clone();

                // Parse messages
                let messages_output: LoadMessagesOutput = match serde_json::from_value(result) {
                    Ok(output) => output,
                    Err(e) => {
                        return vec![
                            self.fail_workflow(&format!("Failed to parse messages: {}", e))
                        ];
                    }
                };

                // Now call LLM
                self.call_llm(agent_config, messages_output.messages, 1)
            }

            AgentRunWorkflowState::CallingLlm {
                agent_config,
                messages,
                iteration,
                ..
            } => {
                let agent_config = agent_config.clone();
                let mut messages = messages.clone();
                let iteration = *iteration;

                // Parse LLM output
                let llm_output: CallLlmOutput = match serde_json::from_value(result) {
                    Ok(output) => output,
                    Err(e) => {
                        return vec![
                            self.fail_workflow(&format!("Failed to parse LLM output: {}", e))
                        ];
                    }
                };

                // Add assistant message to history
                if !llm_output.text.is_empty() {
                    messages.push(MessageData {
                        role: "assistant".to_string(),
                        content: llm_output.text.clone(),
                    });
                }

                // Check if there are tool calls
                if let Some(tool_calls) = llm_output.tool_calls {
                    if !tool_calls.is_empty() {
                        // Execute tools
                        return self.execute_tools(agent_config, messages, tool_calls, iteration);
                    }
                }

                // No tool calls - save message and complete
                self.save_message_and_complete(agent_config, messages, llm_output.text, iteration)
            }

            AgentRunWorkflowState::ExecutingTools {
                agent_config,
                messages,
                iteration,
                ..
            } => {
                let agent_config = agent_config.clone();
                let mut messages = messages.clone();
                let iteration = *iteration;

                // Parse tool results
                let tools_output: ExecuteToolsOutput = match serde_json::from_value(result) {
                    Ok(output) => output,
                    Err(e) => {
                        return vec![
                            self.fail_workflow(&format!("Failed to parse tool results: {}", e))
                        ];
                    }
                };

                // Add tool results to messages
                for tool_result in tools_output.results {
                    let content = if let Some(result) = tool_result.result {
                        serde_json::to_string(&result).unwrap_or_default()
                    } else if let Some(error) = tool_result.error {
                        format!("Error: {}", error)
                    } else {
                        "No result".to_string()
                    };

                    messages.push(MessageData {
                        role: "tool".to_string(),
                        content,
                    });
                }

                // Check iteration limit
                if iteration >= MAX_TOOL_ITERATIONS {
                    warn!(
                        run_id = %self.input.run_id,
                        iteration = iteration,
                        "Max tool iterations reached"
                    );
                    return self.complete_workflow("completed".to_string());
                }

                // Continue with another LLM call
                self.call_llm(agent_config, messages, iteration + 1)
            }

            AgentRunWorkflowState::SavingMessage {
                iteration,
                has_more_tools,
                ..
            } => {
                let _iteration = *iteration;
                let _has_more_tools = *has_more_tools;

                // Message saved, complete the workflow
                self.complete_workflow("completed".to_string())
            }

            AgentRunWorkflowState::UpdatingStatus { final_status, .. } => {
                let status = final_status.clone();
                info!(run_id = %self.input.run_id, status = %status, "Workflow completing");

                self.state = if status == "completed" {
                    AgentRunWorkflowState::Completed
                } else {
                    AgentRunWorkflowState::Failed {
                        error: status.clone(),
                    }
                };

                vec![WorkflowAction::CompleteWorkflow {
                    result: Some(serde_json::json!({ "status": status })),
                }]
            }

            _ => {
                warn!(
                    activity_id = %activity_id,
                    state = ?self.state,
                    "Unexpected activity completion in state"
                );
                vec![]
            }
        }
    }

    /// Process activity failure
    pub fn on_activity_failed(&mut self, activity_id: &str, error: &str) -> Vec<WorkflowAction> {
        warn!(
            activity_id = %activity_id,
            error = %error,
            "Activity failed"
        );

        vec![self.fail_workflow(&format!("Activity {} failed: {}", activity_id, error))]
    }

    /// Call LLM activity
    fn call_llm(
        &mut self,
        agent_config: LoadAgentOutput,
        messages: Vec<MessageData>,
        iteration: u8,
    ) -> Vec<WorkflowAction> {
        let activity_id = self.next_activity_id("call-llm");
        let seq = self.activity_seq;

        let input = CallLlmInput {
            run_id: self.input.run_id,
            model_id: agent_config.model_id.clone(),
            messages: messages.clone(),
            system_prompt: agent_config.system_prompt.clone(),
            temperature: agent_config.temperature,
            max_tokens: agent_config.max_tokens,
        };

        self.state = AgentRunWorkflowState::CallingLlm {
            activity_seq: seq,
            agent_config,
            messages,
            iteration,
        };

        vec![WorkflowAction::ScheduleActivity {
            activity_id,
            activity_type: activity_names::CALL_LLM.to_string(),
            input: serde_json::to_value(&input).unwrap(),
        }]
    }

    /// Execute tools activity
    fn execute_tools(
        &mut self,
        agent_config: LoadAgentOutput,
        messages: Vec<MessageData>,
        tool_calls: Vec<ToolCallData>,
        iteration: u8,
    ) -> Vec<WorkflowAction> {
        let activity_id = self.next_activity_id("execute-tools");
        let seq = self.activity_seq;

        let input = ExecuteToolsInput {
            run_id: self.input.run_id,
            thread_id: self.input.thread_id,
            tool_calls: tool_calls.clone(),
        };

        self.state = AgentRunWorkflowState::ExecutingTools {
            activity_seq: seq,
            agent_config,
            messages,
            tool_calls,
            iteration,
        };

        vec![WorkflowAction::ScheduleActivity {
            activity_id,
            activity_type: activity_names::EXECUTE_TOOLS.to_string(),
            input: serde_json::to_value(&input).unwrap(),
        }]
    }

    /// Save message and complete
    fn save_message_and_complete(
        &mut self,
        agent_config: LoadAgentOutput,
        messages: Vec<MessageData>,
        assistant_text: String,
        iteration: u8,
    ) -> Vec<WorkflowAction> {
        let activity_id = self.next_activity_id("save-message");
        let seq = self.activity_seq;

        let input = SaveMessageInput {
            thread_id: self.input.thread_id,
            role: "assistant".to_string(),
            content: assistant_text.clone(),
        };

        self.state = AgentRunWorkflowState::SavingMessage {
            activity_seq: seq,
            agent_config,
            messages,
            iteration,
            has_more_tools: false,
            assistant_text,
        };

        vec![WorkflowAction::ScheduleActivity {
            activity_id,
            activity_type: activity_names::SAVE_MESSAGE.to_string(),
            input: serde_json::to_value(&input).unwrap(),
        }]
    }

    /// Complete workflow with final status
    fn complete_workflow(&mut self, status: String) -> Vec<WorkflowAction> {
        let activity_id = self.next_activity_id("final-status");
        let seq = self.activity_seq;

        let input = UpdateStatusInput {
            run_id: self.input.run_id,
            status: status.clone(),
            started_at: None,
            finished_at: Some(chrono::Utc::now()),
        };

        self.state = AgentRunWorkflowState::UpdatingStatus {
            activity_seq: seq,
            final_status: status,
        };

        vec![WorkflowAction::ScheduleActivity {
            activity_id,
            activity_type: activity_names::UPDATE_STATUS.to_string(),
            input: serde_json::to_value(&input).unwrap(),
        }]
    }

    /// Fail workflow
    fn fail_workflow(&mut self, error: &str) -> WorkflowAction {
        self.state = AgentRunWorkflowState::Failed {
            error: error.to_string(),
        };

        // Update run status to failed
        let activity_id = self.next_activity_id("fail-status");
        let seq = self.activity_seq;

        let input = UpdateStatusInput {
            run_id: self.input.run_id,
            status: "failed".to_string(),
            started_at: None,
            finished_at: Some(chrono::Utc::now()),
        };

        self.state = AgentRunWorkflowState::UpdatingStatus {
            activity_seq: seq,
            final_status: "failed".to_string(),
        };

        WorkflowAction::ScheduleActivity {
            activity_id,
            activity_type: activity_names::UPDATE_STATUS.to_string(),
            input: serde_json::to_value(&input).unwrap(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn test_input() -> AgentRunWorkflowInput {
        AgentRunWorkflowInput {
            run_id: Uuid::now_v7(),
            agent_id: Uuid::now_v7(),
            thread_id: Uuid::now_v7(),
        }
    }

    #[test]
    fn test_workflow_start() {
        let input = test_input();
        let mut workflow = AgentRunWorkflow::new(input);

        let actions = workflow.on_start();

        // Should schedule update-status and load-agent activities
        assert_eq!(actions.len(), 2);
        assert!(matches!(
            &actions[0],
            WorkflowAction::ScheduleActivity { activity_type, .. }
            if activity_type == activity_names::UPDATE_STATUS
        ));
        assert!(matches!(
            &actions[1],
            WorkflowAction::ScheduleActivity { activity_type, .. }
            if activity_type == activity_names::LOAD_AGENT
        ));
    }

    #[test]
    fn test_workflow_load_agent_completion() {
        let input = test_input();
        let mut workflow = AgentRunWorkflow::new(input.clone());

        // Start workflow
        workflow.on_start();

        // Complete load agent activity
        let agent_output = LoadAgentOutput {
            agent_id: input.agent_id,
            name: "Test Agent".to_string(),
            model_id: "gpt-4".to_string(),
            system_prompt: Some("You are helpful".to_string()),
            temperature: Some(0.7),
            max_tokens: Some(1000),
        };

        let actions = workflow
            .on_activity_completed("load-agent-2", serde_json::to_value(&agent_output).unwrap());

        // Should schedule load-messages
        assert_eq!(actions.len(), 1);
        assert!(matches!(
            &actions[0],
            WorkflowAction::ScheduleActivity { activity_type, .. }
            if activity_type == activity_names::LOAD_MESSAGES
        ));
    }

    #[test]
    fn test_workflow_state_transitions() {
        let input = test_input();
        let workflow = AgentRunWorkflow::new(input);

        assert!(matches!(workflow.state(), AgentRunWorkflowState::Starting));
    }
}

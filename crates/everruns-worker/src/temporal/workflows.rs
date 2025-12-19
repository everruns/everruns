// Temporal workflow implementations (M2)
// Decision: Workflows are state machines that produce commands in response to activations
// Decision: Each LLM call and each tool execution is a separate Temporal activity (node)
//
// The session workflow orchestrates using step.rs abstractions:
// 1. SetupStep: Load agent configuration and messages
// 2. ExecuteLlmStep: Call LLM (may return tool calls)
// 3. ExecuteSingleTool: Execute each tool as separate activity
// 4. Loop back to ExecuteLlmStep if more tool calls
// 5. FinalizeStep: Save final message and update status
//
// All state is deterministic and replayable from Temporal history.

use std::collections::HashMap;

use everruns_agent_loop::message::ConversationMessage;
use everruns_agent_loop::step::StepOutput;
use everruns_contracts::tools::{ToolCall, ToolResult};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::{info, warn};

use super::types::*;

/// Maximum number of tool iterations before forcing completion
const MAX_TOOL_ITERATIONS: u8 = 10;

/// Workflow state for session execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AgentRunWorkflowState {
    /// Initial state - need to load agent config
    Starting,
    /// Loading agent configuration
    LoadingAgent { activity_seq: u32 },
    /// Loading session messages
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
    /// Updating session status
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

/// Session workflow logic (M2)
#[derive(Debug)]
pub struct AgentRunWorkflow {
    /// Workflow input
    input: SessionWorkflowInput,
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
    pub fn new(input: SessionWorkflowInput) -> Self {
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
    pub fn input(&self) -> &SessionWorkflowInput {
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
            session_id = %self.input.session_id,
            agent_id = %self.input.agent_id,
            "Starting session workflow"
        );

        // First, update status to running
        let activity_id = self.next_activity_id("update-status");
        let seq = self.activity_seq;

        let input = UpdateStatusInput {
            session_id: self.input.session_id,
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
        // Log the state BEFORE processing
        let state_name = match &self.state {
            AgentRunWorkflowState::Starting => "Starting",
            AgentRunWorkflowState::LoadingAgent { .. } => "LoadingAgent",
            AgentRunWorkflowState::LoadingMessages { .. } => "LoadingMessages",
            AgentRunWorkflowState::CallingLlm { .. } => "CallingLlm",
            AgentRunWorkflowState::ExecutingTools { .. } => "ExecutingTools",
            AgentRunWorkflowState::SavingMessage { .. } => "SavingMessage",
            AgentRunWorkflowState::UpdatingStatus { .. } => "UpdatingStatus",
            AgentRunWorkflowState::Completed => "Completed",
            AgentRunWorkflowState::Failed { .. } => "Failed",
        };
        info!(
            activity_id = %activity_id,
            session_id = %self.input.session_id,
            state = %state_name,
            state_discriminant = ?std::mem::discriminant(&self.state),
            "Activity completed - processing"
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

                // Debug log the raw result
                tracing::debug!(
                    session_id = %self.input.session_id,
                    result = %result,
                    "LoadingAgent: received activity result"
                );

                // Parse agent config
                let agent_config: LoadAgentOutput = match serde_json::from_value(result.clone()) {
                    Ok(config) => config,
                    Err(e) => {
                        warn!(
                            session_id = %self.input.session_id,
                            error = %e,
                            result = %result,
                            "Failed to parse agent config"
                        );
                        return vec![
                            self.fail_workflow(&format!("Failed to parse agent config: {}", e))
                        ];
                    }
                };

                info!(
                    session_id = %self.input.session_id,
                    agent_id = %agent_config.agent_id,
                    agent_name = %agent_config.name,
                    capability_count = agent_config.capability_ids.len(),
                    "LoadingAgent: parsed agent config, loading messages"
                );

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
                        session_id: self.input.session_id,
                    })
                    .unwrap(),
                }]
            }

            AgentRunWorkflowState::LoadingMessages { agent_config, .. } => {
                // The update-status activity from on_start() may complete while we're in this state
                // Just ignore it and wait for load-messages to complete
                if activity_id.starts_with("update-status") {
                    tracing::debug!(
                        session_id = %self.input.session_id,
                        activity_id = %activity_id,
                        "LoadingMessages: ignoring update-status activity completion"
                    );
                    return vec![];
                }

                let agent_config = agent_config.clone();

                // Debug log the raw result
                tracing::debug!(
                    session_id = %self.input.session_id,
                    result = %result,
                    "LoadingMessages: received activity result"
                );

                // Parse messages
                let messages_output: LoadMessagesOutput = match serde_json::from_value(
                    result.clone(),
                ) {
                    Ok(output) => output,
                    Err(e) => {
                        warn!(
                            session_id = %self.input.session_id,
                            error = %e,
                            result = %result,
                            "Failed to parse messages output - this is causing the workflow to fail"
                        );
                        return vec![
                            self.fail_workflow(&format!("Failed to parse messages: {}", e))
                        ];
                    }
                };

                info!(
                    session_id = %self.input.session_id,
                    message_count = messages_output.messages.len(),
                    "LoadingMessages: parsed messages, calling LLM"
                );

                // Now call LLM
                self.call_llm(agent_config, messages_output.messages, 1)
            }

            AgentRunWorkflowState::CallingLlm {
                agent_config,
                messages,
                iteration,
                ..
            } => {
                // Check which activity completed
                // We may receive save message completions from previous iteration
                if activity_id.starts_with("save-assistant-msg")
                    || activity_id.starts_with("save-tool-msg")
                    || activity_id.starts_with("save-tool-call-msg")
                {
                    tracing::debug!(
                        session_id = %self.input.session_id,
                        activity_id = %activity_id,
                        "CallingLlm: ignoring save message activity completion"
                    );
                    return vec![];
                }

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

                // Check if there are tool calls
                if let Some(ref tool_calls) = llm_output.tool_calls {
                    if !tool_calls.is_empty() {
                        // Build save actions:
                        // 1. Assistant message with tool_calls (for LLM context on reload)
                        // 2. Individual tool_call messages (for UI display as ToolCallCard)
                        let mut save_actions = Vec::new();

                        // Save assistant message with embedded tool_calls (for LLM API compatibility)
                        save_actions.push(
                            self.save_assistant_message_action(&llm_output.text, Some(tool_calls)),
                        );

                        // Save individual tool_call messages (for UI display)
                        for tool_call in tool_calls {
                            save_actions.push(self.save_tool_call_action(tool_call));
                        }

                        // Add assistant message with tool_calls to history
                        // This is required by OpenAI - assistant message with tool_calls must precede tool results
                        messages.push(MessageData {
                            role: "assistant".to_string(),
                            content: llm_output.text.clone(),
                            tool_calls: Some(tool_calls.clone()),
                            tool_call_id: None,
                        });

                        // Execute tools (schedule save actions and execute in parallel)
                        let mut actions = self.execute_tools(
                            agent_config,
                            messages,
                            llm_output.tool_calls.unwrap(),
                            iteration,
                        );
                        // Add save actions first (will run in parallel with tool execution)
                        for (i, save_action) in save_actions.into_iter().enumerate() {
                            actions.insert(i, save_action);
                        }
                        return actions;
                    }
                }

                // No tool calls - add assistant message without tool_calls
                if !llm_output.text.is_empty() {
                    messages.push(MessageData {
                        role: "assistant".to_string(),
                        content: llm_output.text.clone(),
                        tool_calls: None,
                        tool_call_id: None,
                    });
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
                // Check which activity completed
                // We may receive save message completions - ignore them
                if activity_id.starts_with("save-assistant-msg")
                    || activity_id.starts_with("save-tool-call-msg")
                {
                    tracing::debug!(
                        session_id = %self.input.session_id,
                        activity_id = %activity_id,
                        "ExecutingTools: ignoring save message activity completion"
                    );
                    return vec![];
                }

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

                // Save tool result messages to database and add to in-memory history
                // Each tool result must include tool_call_id to link back to the original tool call
                let mut save_actions: Vec<WorkflowAction> = Vec::new();

                for tool_result in tools_output.results {
                    // Save tool result message to database (UI-compatible format)
                    save_actions.push(self.save_tool_result_action(
                        &tool_result.tool_call_id,
                        tool_result.result.as_ref(),
                        tool_result.error.as_deref(),
                    ));

                    // Build text content for in-memory message history (OpenAI API format)
                    let content = if let Some(ref result) = tool_result.result {
                        serde_json::to_string(&result).unwrap_or_default()
                    } else if let Some(ref error) = tool_result.error {
                        format!("Error: {}", error)
                    } else {
                        "No result".to_string()
                    };

                    // Add to in-memory message history
                    messages.push(MessageData {
                        role: "tool".to_string(),
                        content,
                        tool_calls: None,
                        tool_call_id: Some(tool_result.tool_call_id),
                    });
                }

                // Check iteration limit
                if iteration >= MAX_TOOL_ITERATIONS {
                    warn!(
                        session_id = %self.input.session_id,
                        iteration = iteration,
                        "Max tool iterations reached"
                    );
                    // Still save tool results before completing
                    let mut actions = save_actions;
                    actions.extend(self.complete_workflow("pending".to_string()));
                    return actions;
                }

                // Continue with another LLM call
                // Schedule save actions in parallel with the LLM call
                let mut actions = save_actions;
                actions.extend(self.call_llm(agent_config, messages, iteration + 1));
                actions
            }

            AgentRunWorkflowState::SavingMessage {
                iteration,
                has_more_tools,
                ..
            } => {
                let _iteration = *iteration;
                let _has_more_tools = *has_more_tools;

                // Message saved, complete the workflow (back to pending for M2)
                self.complete_workflow("pending".to_string())
            }

            AgentRunWorkflowState::UpdatingStatus { final_status, .. } => {
                let status = final_status.clone();
                info!(session_id = %self.input.session_id, status = %status, "Workflow completing");

                self.state = if status == "pending" || status == "completed" {
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
        // Log the failure with full context
        tracing::error!(
            session_id = %self.input.session_id,
            activity_id = %activity_id,
            error = %error,
            state = ?std::mem::discriminant(&self.state),
            "Activity failed - this is causing the workflow to fail"
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
            session_id: self.input.session_id,
            model_id: agent_config.model_id.clone(),
            messages: messages.clone(),
            system_prompt: agent_config.system_prompt.clone(),
            temperature: agent_config.temperature,
            max_tokens: agent_config.max_tokens,
            capability_ids: agent_config.capability_ids.clone(),
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
            session_id: self.input.session_id,
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
            session_id: self.input.session_id,
            role: "assistant".to_string(),
            content: json!({ "text": assistant_text.clone() }),
            tool_call_id: None,
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

    /// Build content JSON for assistant message with optional tool_calls
    fn build_assistant_content(
        text: &str,
        tool_calls: Option<&[ToolCallData]>,
    ) -> serde_json::Value {
        match tool_calls {
            Some(calls) if !calls.is_empty() => {
                json!({
                    "text": text,
                    "tool_calls": calls
                })
            }
            _ => json!({ "text": text }),
        }
    }

    /// Create a save message action for an assistant message with tool_calls
    fn save_assistant_message_action(
        &mut self,
        text: &str,
        tool_calls: Option<&[ToolCallData]>,
    ) -> WorkflowAction {
        let activity_id = self.next_activity_id("save-assistant-msg");
        let input = SaveMessageInput {
            session_id: self.input.session_id,
            role: "assistant".to_string(),
            content: Self::build_assistant_content(text, tool_calls),
            tool_call_id: None,
        };

        WorkflowAction::ScheduleActivity {
            activity_id,
            activity_type: activity_names::SAVE_MESSAGE.to_string(),
            input: serde_json::to_value(&input).unwrap(),
        }
    }

    /// Create a save message action for a tool call (for UI display)
    /// This creates a separate message with role "tool_call" containing the tool call details
    fn save_tool_call_action(&mut self, tool_call: &ToolCallData) -> WorkflowAction {
        let activity_id = self.next_activity_id("save-tool-call-msg");
        let input = SaveMessageInput {
            session_id: self.input.session_id,
            // Use "tool_call" for database (matches messages_role_check constraint)
            role: "tool_call".to_string(),
            content: json!({
                "id": tool_call.id,
                "name": tool_call.name,
                "arguments": serde_json::from_str::<serde_json::Value>(&tool_call.arguments)
                    .unwrap_or(json!({}))
            }),
            tool_call_id: None, // Not needed for tool_call messages
        };

        WorkflowAction::ScheduleActivity {
            activity_id,
            activity_type: activity_names::SAVE_MESSAGE.to_string(),
            input: serde_json::to_value(&input).unwrap(),
        }
    }

    /// Create a save message action for a tool result
    /// Content is stored in UI-compatible format: { result?: Value, error?: String, text?: String }
    fn save_tool_result_action(
        &mut self,
        tool_call_id: &str,
        result: Option<&serde_json::Value>,
        error: Option<&str>,
    ) -> WorkflowAction {
        let activity_id = self.next_activity_id("save-tool-msg");

        // Build content in UI-compatible format
        let content = if let Some(err) = error {
            json!({
                "error": err,
                "text": format!("Error: {}", err)
            })
        } else if let Some(res) = result {
            json!({
                "result": res,
                "text": serde_json::to_string(res).unwrap_or_default()
            })
        } else {
            json!({
                "text": "No result"
            })
        };

        let input = SaveMessageInput {
            session_id: self.input.session_id,
            // Use "tool_result" for database (matches messages_role_check constraint)
            // This gets converted back to "tool" when loading messages for OpenAI API
            role: "tool_result".to_string(),
            content,
            tool_call_id: Some(tool_call_id.to_string()),
        };

        WorkflowAction::ScheduleActivity {
            activity_id,
            activity_type: activity_names::SAVE_MESSAGE.to_string(),
            input: serde_json::to_value(&input).unwrap(),
        }
    }

    /// Complete workflow with final status
    fn complete_workflow(&mut self, status: String) -> Vec<WorkflowAction> {
        let activity_id = self.next_activity_id("final-status");
        let seq = self.activity_seq;

        let input = UpdateStatusInput {
            session_id: self.input.session_id,
            status: status.clone(),
            started_at: None,
            finished_at: None, // Don't set finished_at for M2 - sessions stay open
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
        // Log the failure with full error details
        tracing::error!(
            session_id = %self.input.session_id,
            error = %error,
            "Workflow failing"
        );

        self.state = AgentRunWorkflowState::Failed {
            error: error.to_string(),
        };

        // Update session status to failed
        let activity_id = self.next_activity_id("fail-status");
        let seq = self.activity_seq;

        let input = UpdateStatusInput {
            session_id: self.input.session_id,
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

    fn test_input() -> SessionWorkflowInput {
        SessionWorkflowInput {
            session_id: Uuid::now_v7(),
            agent_id: Uuid::now_v7(),
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
            capability_ids: vec![],
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

    #[test]
    fn test_step_workflow_start() {
        let input = test_input();
        let mut workflow = StepBasedWorkflow::new(input);

        let actions = workflow.on_start();

        // Should schedule setup step and update status
        assert_eq!(actions.len(), 2);
        assert!(actions.iter().any(|a| matches!(
            a,
            WorkflowAction::ScheduleActivity { activity_type, .. }
            if activity_type == activity_names::UPDATE_STATUS
        )));
        assert!(actions.iter().any(|a| matches!(
            a,
            WorkflowAction::ScheduleActivity { activity_type, .. }
            if activity_type == activity_names::SETUP_STEP
        )));
    }

    #[test]
    fn test_workflow_with_tool_calls() {
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
            capability_ids: vec!["CurrentTime".to_string()],
        };

        workflow
            .on_activity_completed("load-agent-2", serde_json::to_value(&agent_output).unwrap());

        // Complete load messages with just a user message
        let messages_output = LoadMessagesOutput {
            messages: vec![MessageData {
                role: "user".to_string(),
                content: "What time is it?".to_string(),
                tool_calls: None,
                tool_call_id: None,
            }],
        };

        let actions = workflow.on_activity_completed(
            "load-messages-3",
            serde_json::to_value(&messages_output).unwrap(),
        );

        // Should schedule call-llm
        assert_eq!(actions.len(), 1);
        assert!(matches!(
            &actions[0],
            WorkflowAction::ScheduleActivity { activity_type, .. }
            if activity_type == activity_names::CALL_LLM
        ));

        // Simulate LLM response with tool calls
        let llm_output = CallLlmOutput {
            text: "Let me check the time.".to_string(),
            tool_calls: Some(vec![ToolCallData {
                id: "call_abc123".to_string(),
                name: "get_current_time".to_string(),
                arguments: r#"{"timezone": "UTC"}"#.to_string(),
                tool_definition_json: None,
            }]),
        };

        let actions = workflow
            .on_activity_completed("call-llm-4", serde_json::to_value(&llm_output).unwrap());

        // Should schedule:
        // 1. save-assistant-msg (assistant message with tool_calls for LLM context)
        // 2. save-tool-call-msg (tool_call message for UI display)
        // 3. execute-tools (actual tool execution)
        assert_eq!(actions.len(), 3);
        assert!(matches!(
            &actions[0],
            WorkflowAction::ScheduleActivity { activity_type, .. }
            if activity_type == activity_names::SAVE_MESSAGE
        ));
        assert!(matches!(
            &actions[1],
            WorkflowAction::ScheduleActivity { activity_type, .. }
            if activity_type == activity_names::SAVE_MESSAGE
        ));
        assert!(matches!(
            &actions[2],
            WorkflowAction::ScheduleActivity { activity_type, .. }
            if activity_type == activity_names::EXECUTE_TOOLS
        ));

        // Verify the second action is a tool_call message
        if let WorkflowAction::ScheduleActivity { input, .. } = &actions[1] {
            let save_input: SaveMessageInput = serde_json::from_value(input.clone()).unwrap();
            assert_eq!(save_input.role, "tool_call");
            let content = save_input.content;
            assert_eq!(content["name"], "get_current_time");
            assert_eq!(content["id"], "call_abc123");
        }

        // Check the state includes tool calls
        assert!(matches!(
            workflow.state(),
            AgentRunWorkflowState::ExecutingTools { .. }
        ));
    }

    #[test]
    fn test_workflow_tool_results_include_tool_call_id() {
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
            capability_ids: vec![],
        };

        workflow
            .on_activity_completed("load-agent-2", serde_json::to_value(&agent_output).unwrap());

        // Complete load messages
        let messages_output = LoadMessagesOutput {
            messages: vec![MessageData {
                role: "user".to_string(),
                content: "What time is it?".to_string(),
                tool_calls: None,
                tool_call_id: None,
            }],
        };

        workflow.on_activity_completed(
            "load-messages-3",
            serde_json::to_value(&messages_output).unwrap(),
        );

        // Simulate LLM response with tool calls
        let llm_output = CallLlmOutput {
            text: "".to_string(),
            tool_calls: Some(vec![ToolCallData {
                id: "call_tool1".to_string(),
                name: "get_time".to_string(),
                arguments: "{}".to_string(),
                tool_definition_json: None,
            }]),
        };

        workflow.on_activity_completed("call-llm-4", serde_json::to_value(&llm_output).unwrap());

        // Simulate tool execution results
        let tools_output = ExecuteToolsOutput {
            results: vec![ToolResultData {
                tool_call_id: "call_tool1".to_string(),
                result: Some(serde_json::json!({"time": "12:00 UTC"})),
                error: None,
            }],
        };

        let actions = workflow.on_activity_completed(
            "execute-tools-6",
            serde_json::to_value(&tools_output).unwrap(),
        );

        // Should schedule save-tool-msg AND call-llm (in parallel)
        assert_eq!(actions.len(), 2);
        assert!(matches!(
            &actions[0],
            WorkflowAction::ScheduleActivity { activity_type, .. }
            if activity_type == activity_names::SAVE_MESSAGE
        ));
        assert!(matches!(
            &actions[1],
            WorkflowAction::ScheduleActivity { activity_type, .. }
            if activity_type == activity_names::CALL_LLM
        ));

        // Check the call-llm input includes proper message structure
        if let WorkflowAction::ScheduleActivity { input, .. } = &actions[1] {
            let call_llm_input: CallLlmInput = serde_json::from_value(input.clone()).unwrap();

            // Should have: user, assistant (with tool_calls), tool (with tool_call_id)
            assert_eq!(call_llm_input.messages.len(), 3);

            // Check user message
            assert_eq!(call_llm_input.messages[0].role, "user");
            assert!(call_llm_input.messages[0].tool_calls.is_none());
            assert!(call_llm_input.messages[0].tool_call_id.is_none());

            // Check assistant message has tool_calls
            assert_eq!(call_llm_input.messages[1].role, "assistant");
            assert!(call_llm_input.messages[1].tool_calls.is_some());
            assert_eq!(
                call_llm_input.messages[1]
                    .tool_calls
                    .as_ref()
                    .unwrap()
                    .len(),
                1
            );
            assert_eq!(
                call_llm_input.messages[1].tool_calls.as_ref().unwrap()[0].id,
                "call_tool1"
            );

            // Check tool message has tool_call_id
            assert_eq!(call_llm_input.messages[2].role, "tool");
            assert!(call_llm_input.messages[2].tool_calls.is_none());
            assert_eq!(
                call_llm_input.messages[2].tool_call_id,
                Some("call_tool1".to_string())
            );
        } else {
            panic!("Expected ScheduleActivity action");
        }
    }

    #[test]
    fn test_workflow_ignores_update_status_in_loading_messages() {
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
            capability_ids: vec![],
        };

        // Complete load-agent to move to LoadingMessages state
        workflow
            .on_activity_completed("load-agent-2", serde_json::to_value(&agent_output).unwrap());

        // Now in LoadingMessages state, simulate update-status completing
        // This should be ignored (race condition handling)
        let actions = workflow.on_activity_completed("update-status-1", serde_json::json!({}));
        assert!(
            actions.is_empty(),
            "update-status should be ignored in LoadingMessages state"
        );

        // State should still be LoadingMessages
        assert!(matches!(
            workflow.state(),
            AgentRunWorkflowState::LoadingMessages { .. }
        ));
    }

    #[test]
    fn test_workflow_no_tool_calls_completes() {
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
            capability_ids: vec![],
        };

        workflow
            .on_activity_completed("load-agent-2", serde_json::to_value(&agent_output).unwrap());

        // Complete load messages
        let messages_output = LoadMessagesOutput {
            messages: vec![MessageData {
                role: "user".to_string(),
                content: "Hello!".to_string(),
                tool_calls: None,
                tool_call_id: None,
            }],
        };

        workflow.on_activity_completed(
            "load-messages-3",
            serde_json::to_value(&messages_output).unwrap(),
        );

        // Simulate LLM response WITHOUT tool calls
        let llm_output = CallLlmOutput {
            text: "Hello! How can I help you today?".to_string(),
            tool_calls: None,
        };

        let actions = workflow
            .on_activity_completed("call-llm-4", serde_json::to_value(&llm_output).unwrap());

        // Should schedule save-message (not execute-tools)
        assert_eq!(actions.len(), 1);
        assert!(matches!(
            &actions[0],
            WorkflowAction::ScheduleActivity { activity_type, .. }
            if activity_type == activity_names::SAVE_MESSAGE
        ));
    }

    #[test]
    fn test_workflow_empty_tool_calls_treated_as_no_tools() {
        let input = test_input();
        let mut workflow = AgentRunWorkflow::new(input.clone());

        // Start workflow
        workflow.on_start();

        // Complete load agent activity
        let agent_output = LoadAgentOutput {
            agent_id: input.agent_id,
            name: "Test Agent".to_string(),
            model_id: "gpt-4".to_string(),
            system_prompt: None,
            temperature: None,
            max_tokens: None,
            capability_ids: vec![],
        };

        workflow
            .on_activity_completed("load-agent-2", serde_json::to_value(&agent_output).unwrap());

        let messages_output = LoadMessagesOutput {
            messages: vec![MessageData {
                role: "user".to_string(),
                content: "Hi".to_string(),
                tool_calls: None,
                tool_call_id: None,
            }],
        };

        workflow.on_activity_completed(
            "load-messages-3",
            serde_json::to_value(&messages_output).unwrap(),
        );

        // Simulate LLM response with empty tool_calls array (not None)
        let llm_output = CallLlmOutput {
            text: "Hi there!".to_string(),
            tool_calls: Some(vec![]), // Empty, not None
        };

        let actions = workflow
            .on_activity_completed("call-llm-4", serde_json::to_value(&llm_output).unwrap());

        // Should schedule save-message (empty tool_calls should be treated as no tools)
        assert_eq!(actions.len(), 1);
        assert!(matches!(
            &actions[0],
            WorkflowAction::ScheduleActivity { activity_type, .. }
            if activity_type == activity_names::SAVE_MESSAGE
        ));
    }

    #[test]
    fn test_save_assistant_message_action_includes_tool_calls() {
        let input = test_input();
        let mut workflow = AgentRunWorkflow::new(input);

        let tool_calls = vec![
            ToolCallData {
                id: "call_1".to_string(),
                name: "get_time".to_string(),
                arguments: r#"{"timezone": "UTC"}"#.to_string(),
                tool_definition_json: None,
            },
            ToolCallData {
                id: "call_2".to_string(),
                name: "get_weather".to_string(),
                arguments: r#"{"city": "NYC"}"#.to_string(),
                tool_definition_json: None,
            },
        ];

        let action =
            workflow.save_assistant_message_action("Let me check that for you.", Some(&tool_calls));

        if let WorkflowAction::ScheduleActivity { input, .. } = action {
            let save_input: SaveMessageInput = serde_json::from_value(input).unwrap();
            assert_eq!(save_input.role, "assistant");
            assert!(save_input.tool_call_id.is_none());

            // Verify content includes tool_calls
            let content = save_input.content;
            assert_eq!(content["text"], "Let me check that for you.");
            assert!(content.get("tool_calls").is_some());

            let saved_tool_calls: Vec<ToolCallData> =
                serde_json::from_value(content["tool_calls"].clone()).unwrap();
            assert_eq!(saved_tool_calls.len(), 2);
            assert_eq!(saved_tool_calls[0].id, "call_1");
            assert_eq!(saved_tool_calls[1].id, "call_2");
        } else {
            panic!("Expected ScheduleActivity action");
        }
    }

    #[test]
    fn test_save_tool_result_action_includes_tool_call_id() {
        let input = test_input();
        let mut workflow = AgentRunWorkflow::new(input);

        let result_value = serde_json::json!({"time": "12:00 UTC"});
        let action = workflow.save_tool_result_action("call_abc123", Some(&result_value), None);

        if let WorkflowAction::ScheduleActivity { input, .. } = action {
            let save_input: SaveMessageInput = serde_json::from_value(input).unwrap();
            // Database uses "tool_result" to match messages_role_check constraint
            assert_eq!(save_input.role, "tool_result");
            assert_eq!(save_input.tool_call_id, Some("call_abc123".to_string()));

            // Verify content has UI-compatible format with result and text
            let content = save_input.content;
            assert_eq!(content["result"], serde_json::json!({"time": "12:00 UTC"}));
            assert!(content["text"].as_str().unwrap().contains("12:00 UTC"));
        } else {
            panic!("Expected ScheduleActivity action");
        }
    }

    #[test]
    fn test_workflow_ignores_save_assistant_msg_in_executing_tools() {
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
            capability_ids: vec![],
        };

        workflow
            .on_activity_completed("load-agent-2", serde_json::to_value(&agent_output).unwrap());

        let messages_output = LoadMessagesOutput {
            messages: vec![MessageData {
                role: "user".to_string(),
                content: "What time is it?".to_string(),
                tool_calls: None,
                tool_call_id: None,
            }],
        };

        workflow.on_activity_completed(
            "load-messages-3",
            serde_json::to_value(&messages_output).unwrap(),
        );

        // Simulate LLM response with tool calls
        let llm_output = CallLlmOutput {
            text: "Let me check.".to_string(),
            tool_calls: Some(vec![ToolCallData {
                id: "call_tool1".to_string(),
                name: "get_time".to_string(),
                arguments: "{}".to_string(),
                tool_definition_json: None,
            }]),
        };

        // This schedules save-assistant-msg AND execute-tools
        workflow.on_activity_completed("call-llm-4", serde_json::to_value(&llm_output).unwrap());

        // Verify we're in ExecutingTools state
        assert!(matches!(
            workflow.state(),
            AgentRunWorkflowState::ExecutingTools { .. }
        ));

        // Now simulate save-assistant-msg completing (race condition)
        // This should be ignored
        let actions = workflow.on_activity_completed("save-assistant-msg-5", serde_json::json!({}));
        assert!(
            actions.is_empty(),
            "save-assistant-msg should be ignored in ExecutingTools state"
        );

        // State should still be ExecutingTools
        assert!(matches!(
            workflow.state(),
            AgentRunWorkflowState::ExecutingTools { .. }
        ));
    }

    #[test]
    fn test_workflow_saves_multiple_tool_results() {
        let input = test_input();
        let mut workflow = AgentRunWorkflow::new(input.clone());

        // Start workflow
        workflow.on_start();

        // Complete load agent activity
        let agent_output = LoadAgentOutput {
            agent_id: input.agent_id,
            name: "Test Agent".to_string(),
            model_id: "gpt-4".to_string(),
            system_prompt: None,
            temperature: None,
            max_tokens: None,
            capability_ids: vec![],
        };

        workflow
            .on_activity_completed("load-agent-2", serde_json::to_value(&agent_output).unwrap());

        let messages_output = LoadMessagesOutput {
            messages: vec![MessageData {
                role: "user".to_string(),
                content: "Get the time and weather.".to_string(),
                tool_calls: None,
                tool_call_id: None,
            }],
        };

        workflow.on_activity_completed(
            "load-messages-3",
            serde_json::to_value(&messages_output).unwrap(),
        );

        // Simulate LLM response with MULTIPLE tool calls
        let llm_output = CallLlmOutput {
            text: "Let me check both.".to_string(),
            tool_calls: Some(vec![
                ToolCallData {
                    id: "call_time".to_string(),
                    name: "get_time".to_string(),
                    arguments: "{}".to_string(),
                    tool_definition_json: None,
                },
                ToolCallData {
                    id: "call_weather".to_string(),
                    name: "get_weather".to_string(),
                    arguments: r#"{"city": "NYC"}"#.to_string(),
                    tool_definition_json: None,
                },
            ]),
        };

        workflow.on_activity_completed("call-llm-4", serde_json::to_value(&llm_output).unwrap());

        // Simulate tool execution results with MULTIPLE results
        let tools_output = ExecuteToolsOutput {
            results: vec![
                ToolResultData {
                    tool_call_id: "call_time".to_string(),
                    result: Some(serde_json::json!({"time": "12:00 UTC"})),
                    error: None,
                },
                ToolResultData {
                    tool_call_id: "call_weather".to_string(),
                    result: Some(serde_json::json!({"temp": 72, "unit": "F"})),
                    error: None,
                },
            ],
        };

        let actions = workflow.on_activity_completed(
            "execute-tools-6",
            serde_json::to_value(&tools_output).unwrap(),
        );

        // Should schedule 2 save-tool-msg actions + 1 call-llm action
        assert_eq!(actions.len(), 3);

        // First two should be save-tool-msg
        assert!(matches!(
            &actions[0],
            WorkflowAction::ScheduleActivity { activity_type, .. }
            if activity_type == activity_names::SAVE_MESSAGE
        ));
        assert!(matches!(
            &actions[1],
            WorkflowAction::ScheduleActivity { activity_type, .. }
            if activity_type == activity_names::SAVE_MESSAGE
        ));

        // Last should be call-llm
        assert!(matches!(
            &actions[2],
            WorkflowAction::ScheduleActivity { activity_type, .. }
            if activity_type == activity_names::CALL_LLM
        ));

        // Verify the save actions have correct tool_call_ids
        // Database uses "tool_result" to match messages_role_check constraint
        if let WorkflowAction::ScheduleActivity { input, .. } = &actions[0] {
            let save_input: SaveMessageInput = serde_json::from_value(input.clone()).unwrap();
            assert_eq!(save_input.role, "tool_result");
            assert_eq!(save_input.tool_call_id, Some("call_time".to_string()));
        }

        if let WorkflowAction::ScheduleActivity { input, .. } = &actions[1] {
            let save_input: SaveMessageInput = serde_json::from_value(input.clone()).unwrap();
            assert_eq!(save_input.role, "tool_result");
            assert_eq!(save_input.tool_call_id, Some("call_weather".to_string()));
        }
    }
}

// =============================================================================
// Step-based workflow (using step.rs abstractions)
// Each LLM call and each tool is a separate Temporal activity (node)
// =============================================================================

/// Workflow state for step-based session execution
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StepBasedWorkflowState {
    /// Initial state
    Starting,
    /// Running setup step (load agent + messages)
    Setup { activity_seq: u32 },
    /// Executing LLM step
    ExecutingLlm {
        activity_seq: u32,
        agent_config: LoadAgentOutput,
        messages: Vec<ConversationMessage>,
        iteration: usize,
    },
    /// Executing individual tool calls (one at a time)
    ExecutingTool {
        activity_seq: u32,
        agent_config: LoadAgentOutput,
        messages: Vec<ConversationMessage>,
        iteration: usize,
        pending_tools: Vec<ToolCall>,
        completed_results: Vec<ToolResult>,
        current_tool_index: usize,
    },
    /// Finalizing session
    Finalizing {
        activity_seq: u32,
        messages: Vec<ConversationMessage>,
        iteration: usize,
        final_response: Option<String>,
    },
    /// Updating final status
    UpdatingStatus {
        activity_seq: u32,
        final_status: String,
    },
    /// Workflow completed
    Completed,
    /// Workflow failed
    Failed { error: String },
}

/// Step-based session workflow
/// Uses step.rs abstractions with each LLM call and tool as separate Temporal nodes
#[allow(dead_code)]
#[derive(Debug)]
pub struct StepBasedWorkflow {
    /// Workflow input
    input: SessionWorkflowInput,
    /// Current state
    state: StepBasedWorkflowState,
    /// Activity sequence counter
    activity_seq: u32,
    /// Pending activity results
    pending_results: HashMap<String, ActivityResult>,
}

#[allow(dead_code)]
impl StepBasedWorkflow {
    /// Create a new step-based workflow
    pub fn new(input: SessionWorkflowInput) -> Self {
        Self {
            input,
            state: StepBasedWorkflowState::Starting,
            activity_seq: 0,
            pending_results: HashMap::new(),
        }
    }

    /// Get the current state
    pub fn state(&self) -> &StepBasedWorkflowState {
        &self.state
    }

    /// Get the workflow input
    pub fn input(&self) -> &SessionWorkflowInput {
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

    /// Process workflow start
    pub fn on_start(&mut self) -> Vec<WorkflowAction> {
        info!(
            session_id = %self.input.session_id,
            agent_id = %self.input.agent_id,
            "Starting step-based session workflow"
        );

        // Update status to running
        let status_activity_id = self.next_activity_id("update-status");
        let status_input = UpdateStatusInput {
            session_id: self.input.session_id,
            status: "running".to_string(),
            started_at: Some(chrono::Utc::now()),
            finished_at: None,
        };

        // Schedule setup step
        let setup_activity_id = self.next_activity_id("setup-step");
        let setup_seq = self.activity_seq;
        let setup_input = SetupStepInput {
            session_id: self.input.session_id,
            agent_id: self.input.agent_id,
        };

        self.state = StepBasedWorkflowState::Setup {
            activity_seq: setup_seq,
        };

        vec![
            WorkflowAction::ScheduleActivity {
                activity_id: status_activity_id,
                activity_type: activity_names::UPDATE_STATUS.to_string(),
                input: serde_json::to_value(&status_input).unwrap(),
            },
            WorkflowAction::ScheduleActivity {
                activity_id: setup_activity_id,
                activity_type: activity_names::SETUP_STEP.to_string(),
                input: serde_json::to_value(&setup_input).unwrap(),
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
            "Step activity completed"
        );

        match &self.state {
            StepBasedWorkflowState::Setup { .. } => {
                // Ignore status update completion
                if activity_id.starts_with("update-status") {
                    return vec![];
                }

                // Parse setup output
                let setup_output: SetupStepOutput = match serde_json::from_value(result) {
                    Ok(output) => output,
                    Err(e) => {
                        return vec![
                            self.fail_workflow(&format!("Failed to parse setup output: {}", e))
                        ];
                    }
                };

                // Proceed to LLM step
                self.execute_llm_step(setup_output.agent_config, setup_output.messages, 1)
            }

            StepBasedWorkflowState::ExecutingLlm {
                agent_config,
                iteration,
                ..
            } => {
                let agent_config = agent_config.clone();
                let iteration = *iteration;

                // Parse LLM step output
                let llm_output: ExecuteLlmStepOutput = match serde_json::from_value(result) {
                    Ok(output) => output,
                    Err(e) => {
                        return vec![
                            self.fail_workflow(&format!("Failed to parse LLM output: {}", e))
                        ];
                    }
                };

                if llm_output.has_tool_calls {
                    // Execute tools one by one
                    self.execute_tools_sequentially(
                        agent_config,
                        llm_output.step_output.messages,
                        iteration,
                        llm_output.pending_tool_calls,
                    )
                } else {
                    // No tools, finalize
                    let final_response = self.extract_final_response(&llm_output.step_output);
                    self.finalize(llm_output.step_output.messages, iteration, final_response)
                }
            }

            StepBasedWorkflowState::ExecutingTool {
                agent_config,
                messages,
                iteration,
                pending_tools,
                completed_results,
                current_tool_index,
                ..
            } => {
                let agent_config = agent_config.clone();
                let mut messages = messages.clone();
                let iteration = *iteration;
                let pending_tools = pending_tools.clone();
                let mut completed_results = completed_results.clone();
                let current_tool_index = *current_tool_index;

                // Parse tool output
                let tool_output: ExecuteSingleToolOutput = match serde_json::from_value(result) {
                    Ok(output) => output,
                    Err(e) => {
                        return vec![
                            self.fail_workflow(&format!("Failed to parse tool output: {}", e))
                        ];
                    }
                };

                // Add tool result to completed list
                completed_results.push(tool_output.result.clone());

                // Add tool result message to conversation
                let tool_call = &pending_tools[current_tool_index];
                let result_msg = ConversationMessage::tool_result(
                    &tool_call.id,
                    tool_output.result.result,
                    tool_output.result.error,
                );
                messages.push(result_msg);

                let next_index = current_tool_index + 1;

                if next_index < pending_tools.len() {
                    // More tools to execute
                    self.execute_next_tool(
                        agent_config,
                        messages,
                        iteration,
                        pending_tools,
                        completed_results,
                        next_index,
                    )
                } else {
                    // All tools done, check iteration limit
                    if iteration >= MAX_TOOL_ITERATIONS as usize {
                        warn!(
                            session_id = %self.input.session_id,
                            iteration = iteration,
                            "Max tool iterations reached"
                        );
                        self.finalize(messages, iteration, None)
                    } else {
                        // Call LLM again with tool results
                        self.execute_llm_step(agent_config, messages, iteration + 1)
                    }
                }
            }

            StepBasedWorkflowState::Finalizing { .. } => {
                // Finalize completed, update status
                self.complete_workflow("pending".to_string())
            }

            StepBasedWorkflowState::UpdatingStatus { final_status, .. } => {
                let status = final_status.clone();
                info!(session_id = %self.input.session_id, status = %status, "Step workflow completing");

                self.state = if status == "pending" || status == "completed" {
                    StepBasedWorkflowState::Completed
                } else {
                    StepBasedWorkflowState::Failed {
                        error: status.clone(),
                    }
                };

                vec![WorkflowAction::CompleteWorkflow {
                    result: Some(json!({ "status": status })),
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
            "Step activity failed"
        );

        vec![self.fail_workflow(&format!("Activity {} failed: {}", activity_id, error))]
    }

    // =========================================================================
    // Private helper methods
    // =========================================================================

    fn execute_llm_step(
        &mut self,
        agent_config: LoadAgentOutput,
        messages: Vec<ConversationMessage>,
        iteration: usize,
    ) -> Vec<WorkflowAction> {
        let activity_id = self.next_activity_id("execute-llm-step");
        let seq = self.activity_seq;

        let input = ExecuteLlmStepInput {
            session_id: self.input.session_id,
            agent_config: agent_config.clone(),
            messages: messages.clone(),
            iteration,
        };

        self.state = StepBasedWorkflowState::ExecutingLlm {
            activity_seq: seq,
            agent_config,
            messages,
            iteration,
        };

        vec![WorkflowAction::ScheduleActivity {
            activity_id,
            activity_type: activity_names::EXECUTE_LLM_STEP.to_string(),
            input: serde_json::to_value(&input).unwrap(),
        }]
    }

    fn execute_tools_sequentially(
        &mut self,
        agent_config: LoadAgentOutput,
        messages: Vec<ConversationMessage>,
        iteration: usize,
        pending_tools: Vec<ToolCall>,
    ) -> Vec<WorkflowAction> {
        if pending_tools.is_empty() {
            return self.finalize(messages, iteration, None);
        }

        self.execute_next_tool(
            agent_config,
            messages,
            iteration,
            pending_tools,
            Vec::new(),
            0,
        )
    }

    fn execute_next_tool(
        &mut self,
        agent_config: LoadAgentOutput,
        messages: Vec<ConversationMessage>,
        iteration: usize,
        pending_tools: Vec<ToolCall>,
        completed_results: Vec<ToolResult>,
        tool_index: usize,
    ) -> Vec<WorkflowAction> {
        let tool_call = &pending_tools[tool_index];
        let activity_id = self.next_activity_id(&format!("execute-tool-{}", tool_call.name));
        let seq = self.activity_seq;

        let input = ExecuteSingleToolInput {
            session_id: self.input.session_id,
            tool_call: tool_call.clone(),
            tool_definition_json: None, // TODO: pass tool definitions from agent config
        };

        self.state = StepBasedWorkflowState::ExecutingTool {
            activity_seq: seq,
            agent_config,
            messages,
            iteration,
            pending_tools,
            completed_results,
            current_tool_index: tool_index,
        };

        vec![WorkflowAction::ScheduleActivity {
            activity_id,
            activity_type: activity_names::EXECUTE_SINGLE_TOOL.to_string(),
            input: serde_json::to_value(&input).unwrap(),
        }]
    }

    fn finalize(
        &mut self,
        messages: Vec<ConversationMessage>,
        iteration: usize,
        final_response: Option<String>,
    ) -> Vec<WorkflowAction> {
        let activity_id = self.next_activity_id("finalize-step");
        let seq = self.activity_seq;

        let input = FinalizeStepInput {
            session_id: self.input.session_id,
            final_messages: messages.clone(),
            total_iterations: iteration,
            final_response: final_response.clone(),
        };

        self.state = StepBasedWorkflowState::Finalizing {
            activity_seq: seq,
            messages,
            iteration,
            final_response,
        };

        vec![WorkflowAction::ScheduleActivity {
            activity_id,
            activity_type: activity_names::FINALIZE_STEP.to_string(),
            input: serde_json::to_value(&input).unwrap(),
        }]
    }

    fn complete_workflow(&mut self, status: String) -> Vec<WorkflowAction> {
        let activity_id = self.next_activity_id("final-status");
        let seq = self.activity_seq;

        let input = UpdateStatusInput {
            session_id: self.input.session_id,
            status: status.clone(),
            started_at: None,
            finished_at: None,
        };

        self.state = StepBasedWorkflowState::UpdatingStatus {
            activity_seq: seq,
            final_status: status,
        };

        vec![WorkflowAction::ScheduleActivity {
            activity_id,
            activity_type: activity_names::UPDATE_STATUS.to_string(),
            input: serde_json::to_value(&input).unwrap(),
        }]
    }

    fn fail_workflow(&mut self, error: &str) -> WorkflowAction {
        self.state = StepBasedWorkflowState::Failed {
            error: error.to_string(),
        };

        let activity_id = self.next_activity_id("fail-status");
        let seq = self.activity_seq;

        let input = UpdateStatusInput {
            session_id: self.input.session_id,
            status: "failed".to_string(),
            started_at: None,
            finished_at: Some(chrono::Utc::now()),
        };

        self.state = StepBasedWorkflowState::UpdatingStatus {
            activity_seq: seq,
            final_status: "failed".to_string(),
        };

        WorkflowAction::ScheduleActivity {
            activity_id,
            activity_type: activity_names::UPDATE_STATUS.to_string(),
            input: serde_json::to_value(&input).unwrap(),
        }
    }

    fn extract_final_response(&self, step_output: &StepOutput) -> Option<String> {
        // Find the last assistant message
        step_output
            .messages
            .iter()
            .rev()
            .find(|m| m.role == everruns_agent_loop::MessageRole::Assistant)
            .and_then(|m| match &m.content {
                everruns_agent_loop::message::MessageContent::Text(text) => Some(text.clone()),
                _ => None,
            })
    }
}

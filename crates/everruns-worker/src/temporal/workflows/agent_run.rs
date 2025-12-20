// TemporalSessionWorkflow - Durable session-based agent execution workflow
// Decision: Workflows are state machines that produce commands in response to activations
// Decision: Each LLM call and each tool execution is a separate Temporal activity (node)
//
// The session workflow orchestrates:
// 1. Load agent configuration
// 2. Load session messages
// 3. Call LLM (may return tool calls)
// 4. Execute tools if needed
// 5. Loop back to LLM if more tool calls
// 6. Save final message and update status
//
// All state is deterministic and replayable from Temporal history.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::{info, warn};

use crate::temporal::types::*;
use crate::workflow_traits::{Workflow, WorkflowInput};

/// Maximum number of tool iterations before forcing completion
const MAX_TOOL_ITERATIONS: u8 = 10;

/// Workflow state for Temporal session execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TemporalSessionWorkflowState {
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

/// Activity result from Temporal
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ActivityResult {
    Completed(serde_json::Value),
    Failed(String),
}

/// Temporal session workflow for durable agent execution
///
/// This workflow runs via Temporal for durability, retry, and recovery.
#[derive(Debug)]
pub struct TemporalSessionWorkflow {
    /// Workflow input
    input: SessionWorkflowInput,
    /// Current state
    state: TemporalSessionWorkflowState,
    /// Activity sequence counter for generating unique IDs
    activity_seq: u32,
    /// Pending activity results (activity_id -> result)
    pending_results: HashMap<String, ActivityResult>,
}

impl TemporalSessionWorkflow {
    /// Create a new workflow instance
    pub fn new(input: SessionWorkflowInput) -> Self {
        Self {
            input,
            state: TemporalSessionWorkflowState::Starting,
            activity_seq: 0,
            pending_results: HashMap::new(),
        }
    }

    /// Get the current workflow state
    pub fn state(&self) -> &TemporalSessionWorkflowState {
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
    fn do_start(&mut self) -> Vec<WorkflowAction> {
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

        self.state = TemporalSessionWorkflowState::LoadingAgent { activity_seq: seq };

        // Schedule update status and load agent in parallel
        let status_activity_id = activity_id;
        let load_agent_activity_id = self.next_activity_id("load-agent");
        let load_agent_seq = self.activity_seq;
        self.state = TemporalSessionWorkflowState::LoadingAgent {
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
    fn do_activity_completed(
        &mut self,
        activity_id: &str,
        result: serde_json::Value,
    ) -> Vec<WorkflowAction> {
        // Log the state BEFORE processing
        let state_name = match &self.state {
            TemporalSessionWorkflowState::Starting => "Starting",
            TemporalSessionWorkflowState::LoadingAgent { .. } => "LoadingAgent",
            TemporalSessionWorkflowState::LoadingMessages { .. } => "LoadingMessages",
            TemporalSessionWorkflowState::CallingLlm { .. } => "CallingLlm",
            TemporalSessionWorkflowState::ExecutingTools { .. } => "ExecutingTools",
            TemporalSessionWorkflowState::SavingMessage { .. } => "SavingMessage",
            TemporalSessionWorkflowState::UpdatingStatus { .. } => "UpdatingStatus",
            TemporalSessionWorkflowState::Completed => "Completed",
            TemporalSessionWorkflowState::Failed { .. } => "Failed",
        };
        info!(
            activity_id = %activity_id,
            session_id = %self.input.session_id,
            state = %state_name,
            state_discriminant = ?std::mem::discriminant(&self.state),
            "Activity completed - processing"
        );

        match &self.state {
            TemporalSessionWorkflowState::LoadingAgent { .. } => {
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
                self.state = TemporalSessionWorkflowState::LoadingMessages {
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

            TemporalSessionWorkflowState::LoadingMessages { agent_config, .. } => {
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

            TemporalSessionWorkflowState::CallingLlm {
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

            TemporalSessionWorkflowState::ExecutingTools {
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

            TemporalSessionWorkflowState::SavingMessage {
                iteration,
                has_more_tools,
                ..
            } => {
                let _iteration = *iteration;
                let _has_more_tools = *has_more_tools;

                // Message saved, complete the workflow (back to pending for M2)
                self.complete_workflow("pending".to_string())
            }

            TemporalSessionWorkflowState::UpdatingStatus { final_status, .. } => {
                let status = final_status.clone();
                info!(session_id = %self.input.session_id, status = %status, "Workflow completing");

                self.state = if status == "pending" || status == "completed" {
                    TemporalSessionWorkflowState::Completed
                } else {
                    TemporalSessionWorkflowState::Failed {
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
    fn do_activity_failed(&mut self, activity_id: &str, error: &str) -> Vec<WorkflowAction> {
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

        self.state = TemporalSessionWorkflowState::CallingLlm {
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

        self.state = TemporalSessionWorkflowState::ExecutingTools {
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

        self.state = TemporalSessionWorkflowState::SavingMessage {
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

        self.state = TemporalSessionWorkflowState::UpdatingStatus {
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

        self.state = TemporalSessionWorkflowState::Failed {
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

        self.state = TemporalSessionWorkflowState::UpdatingStatus {
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

// =============================================================================
// Workflow trait implementation
// =============================================================================

impl Workflow for TemporalSessionWorkflow {
    fn workflow_type(&self) -> &'static str {
        workflow_names::SESSION_WORKFLOW
    }

    fn on_start(&mut self) -> Vec<WorkflowAction> {
        self.do_start()
    }

    fn on_activity_completed(
        &mut self,
        activity_id: &str,
        result: serde_json::Value,
    ) -> Vec<WorkflowAction> {
        self.do_activity_completed(activity_id, result)
    }

    fn on_activity_failed(&mut self, activity_id: &str, error: &str) -> Vec<WorkflowAction> {
        self.do_activity_failed(activity_id, error)
    }

    fn is_completed(&self) -> bool {
        matches!(
            self.state,
            TemporalSessionWorkflowState::Completed | TemporalSessionWorkflowState::Failed { .. }
        )
    }
}

impl WorkflowInput for TemporalSessionWorkflow {
    const WORKFLOW_TYPE: &'static str = workflow_names::SESSION_WORKFLOW;
    type Input = SessionWorkflowInput;

    fn from_input(input: Self::Input) -> Self {
        TemporalSessionWorkflow::new(input)
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
        let mut workflow = TemporalSessionWorkflow::new(input);

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
        let mut workflow = TemporalSessionWorkflow::new(input.clone());

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
        let workflow = TemporalSessionWorkflow::new(input);

        assert!(matches!(
            workflow.state(),
            TemporalSessionWorkflowState::Starting
        ));
        assert!(!workflow.is_completed());
    }

    #[test]
    fn test_workflow_type() {
        let input = test_input();
        let workflow = TemporalSessionWorkflow::new(input);

        assert_eq!(workflow.workflow_type(), "session_workflow");
    }

    #[test]
    fn test_workflow_is_completed() {
        let input = test_input();
        let mut workflow = TemporalSessionWorkflow::new(input);

        assert!(!workflow.is_completed());

        // Manually set to completed state
        workflow.state = TemporalSessionWorkflowState::Completed;
        assert!(workflow.is_completed());

        // Check failed state
        workflow.state = TemporalSessionWorkflowState::Failed {
            error: "test error".to_string(),
        };
        assert!(workflow.is_completed());
    }

    #[test]
    fn test_workflow_with_tool_calls() {
        let input = test_input();
        let mut workflow = TemporalSessionWorkflow::new(input.clone());

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

        // Check the state includes tool calls
        assert!(matches!(
            workflow.state(),
            TemporalSessionWorkflowState::ExecutingTools { .. }
        ));
    }

    #[test]
    fn test_workflow_no_tool_calls_completes() {
        let input = test_input();
        let mut workflow = TemporalSessionWorkflow::new(input.clone());

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
    fn test_workflow_ignores_update_status_in_loading_messages() {
        let input = test_input();
        let mut workflow = TemporalSessionWorkflow::new(input.clone());

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
            TemporalSessionWorkflowState::LoadingMessages { .. }
        ));
    }
}

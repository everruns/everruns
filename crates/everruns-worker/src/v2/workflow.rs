// V2 Session Workflow - Infinite loop state machine
//
// Decision: Workflow is an infinite loop representing the entire session
// Decision: States are: Waiting -> Running -> (agent loop) -> Waiting
// Decision: New messages arrive via signals
// Decision: Error if message arrives while running, accept if waiting
// Decision: Tool calls execute in parallel

use serde::{Deserialize, Serialize};
use tracing::{info, warn};
use uuid::Uuid;

use super::types::*;

/// Maximum number of agent iterations per turn (prevent infinite loops)
const MAX_ITERATIONS_PER_TURN: usize = 10;

/// Workflow state for the session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SessionState {
    /// Initial state - loading agent config
    Initializing,

    /// Waiting for user input (idle)
    Waiting {
        /// Messages in the conversation so far
        messages: Vec<Message>,
        /// Number of turns completed
        turn_count: u32,
    },

    /// Running the agent loop
    Running {
        /// Agent configuration
        agent_config: AgentConfig,
        /// Messages in the conversation
        messages: Vec<Message>,
        /// Current iteration within the turn
        iteration: usize,
        /// Current turn number
        turn_count: u32,
        /// Pending LLM activity
        pending_llm: Option<String>,
        /// Pending tool activities (activity_id -> tool_call_id)
        pending_tools: Vec<PendingTool>,
    },

    /// Session completed (terminal state)
    Completed {
        messages: Vec<Message>,
        turn_count: u32,
    },

    /// Session failed (terminal state)
    Failed {
        error: String,
        messages: Vec<Message>,
        turn_count: u32,
    },
}

/// A pending tool execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingTool {
    /// Activity ID
    pub activity_id: String,
    /// Tool call ID
    pub tool_call_id: String,
    /// Tool name
    pub tool_name: String,
}

/// Action to perform (commands from workflow to executor)
#[derive(Debug, Clone)]
pub enum WorkflowAction {
    /// Schedule an activity
    ScheduleActivity {
        activity_id: String,
        activity_type: ActivityType,
        input: serde_json::Value,
    },
    /// Schedule multiple activities in parallel
    ScheduleParallelActivities {
        activities: Vec<(String, ActivityType, serde_json::Value)>,
    },
    /// Wait for signal (new message)
    WaitForSignal,
    /// Complete the workflow
    Complete { output: SessionOutput },
    /// No action needed
    None,
}

/// Activity types
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ActivityType {
    LoadAgent,
    CallLlm,
    ExecuteTool,
}

impl std::fmt::Display for ActivityType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ActivityType::LoadAgent => write!(f, "load_agent"),
            ActivityType::CallLlm => write!(f, "call_llm"),
            ActivityType::ExecuteTool => write!(f, "execute_tool"),
        }
    }
}

/// Activity result (from executor to workflow)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ActivityResult {
    Completed(serde_json::Value),
    Failed(String),
}

/// Signal types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WorkflowSignal {
    /// New message from user
    NewMessage(NewMessageSignal),
    /// Shutdown the session
    Shutdown,
}

/// The session workflow state machine
#[derive(Debug)]
pub struct SessionWorkflow {
    /// Workflow input
    input: SessionInput,
    /// Current state
    state: SessionState,
    /// Activity sequence counter
    activity_seq: u32,
    /// Agent configuration (cached after loading)
    agent_config: Option<AgentConfig>,
}

impl SessionWorkflow {
    /// Create a new session workflow
    pub fn new(input: SessionInput) -> Self {
        Self {
            input,
            state: SessionState::Initializing,
            activity_seq: 0,
            agent_config: None,
        }
    }

    /// Get the current state
    pub fn state(&self) -> &SessionState {
        &self.state
    }

    /// Get the session ID
    pub fn session_id(&self) -> Uuid {
        self.input.session_id
    }

    /// Check if the workflow is in a terminal state
    pub fn is_terminal(&self) -> bool {
        matches!(
            self.state,
            SessionState::Completed { .. } | SessionState::Failed { .. }
        )
    }

    /// Check if the workflow is waiting for input
    pub fn is_waiting(&self) -> bool {
        matches!(self.state, SessionState::Waiting { .. })
    }

    /// Check if the workflow is running
    pub fn is_running(&self) -> bool {
        matches!(self.state, SessionState::Running { .. })
    }

    /// Generate a unique activity ID
    fn next_activity_id(&mut self, activity_type: &str) -> String {
        self.activity_seq += 1;
        format!("{}-{}", activity_type, self.activity_seq)
    }

    /// Process workflow start
    pub fn on_start(&mut self) -> WorkflowAction {
        info!(
            session_id = %self.input.session_id,
            agent_id = %self.input.agent_id,
            "Starting v2 session workflow"
        );

        let activity_id = self.next_activity_id("load-agent");
        let input = LoadAgentInput {
            agent_id: self.input.agent_id,
        };

        WorkflowAction::ScheduleActivity {
            activity_id,
            activity_type: ActivityType::LoadAgent,
            input: serde_json::to_value(&input).unwrap(),
        }
    }

    /// Process activity completion
    pub fn on_activity_completed(
        &mut self,
        activity_id: &str,
        result: serde_json::Value,
    ) -> WorkflowAction {
        match &self.state {
            SessionState::Initializing => self.handle_load_agent_completed(result),

            SessionState::Running { pending_llm, .. } if pending_llm.is_some() => {
                self.handle_llm_completed(activity_id, result)
            }

            SessionState::Running { pending_tools, .. } if !pending_tools.is_empty() => {
                self.handle_tool_completed(activity_id, result)
            }

            _ => {
                warn!(
                    activity_id = %activity_id,
                    state = ?std::mem::discriminant(&self.state),
                    "Unexpected activity completion"
                );
                WorkflowAction::None
            }
        }
    }

    /// Process activity failure
    pub fn on_activity_failed(&mut self, activity_id: &str, error: &str) -> WorkflowAction {
        warn!(
            activity_id = %activity_id,
            error = %error,
            "Activity failed"
        );

        let (messages, turn_count) = match &self.state {
            SessionState::Running {
                messages,
                turn_count,
                ..
            } => (messages.clone(), *turn_count),
            SessionState::Waiting {
                messages,
                turn_count,
            } => (messages.clone(), *turn_count),
            _ => (Vec::new(), 0),
        };

        self.state = SessionState::Failed {
            error: format!("Activity {} failed: {}", activity_id, error),
            messages,
            turn_count,
        };

        WorkflowAction::Complete {
            output: SessionOutput {
                session_id: self.input.session_id,
                status: SessionStatus::Failed,
                total_turns: turn_count,
                error: Some(error.to_string()),
            },
        }
    }

    /// Process a signal
    pub fn on_signal(&mut self, signal: WorkflowSignal) -> WorkflowAction {
        match signal {
            WorkflowSignal::NewMessage(msg_signal) => self.handle_new_message(msg_signal),
            WorkflowSignal::Shutdown => self.handle_shutdown(),
        }
    }

    // =========================================================================
    // Private handlers
    // =========================================================================

    fn handle_load_agent_completed(&mut self, result: serde_json::Value) -> WorkflowAction {
        let agent_config: AgentConfig = match serde_json::from_value(result) {
            Ok(config) => config,
            Err(e) => {
                return self.fail(&format!("Failed to parse agent config: {}", e));
            }
        };

        info!(
            session_id = %self.input.session_id,
            agent_name = %agent_config.name,
            "Agent loaded, waiting for input"
        );

        self.agent_config = Some(agent_config);
        self.state = SessionState::Waiting {
            messages: Vec::new(),
            turn_count: 0,
        };

        WorkflowAction::WaitForSignal
    }

    fn handle_new_message(&mut self, signal: NewMessageSignal) -> WorkflowAction {
        match &self.state {
            SessionState::Waiting {
                messages,
                turn_count,
            } => {
                let mut messages = messages.clone();
                let turn_count = *turn_count + 1;

                info!(
                    session_id = %self.input.session_id,
                    turn = turn_count,
                    role = ?signal.message.role,
                    "New message received, starting turn"
                );

                // Add the new message
                messages.push(signal.message);

                // Start the agent loop
                self.start_turn(messages, turn_count)
            }

            SessionState::Running { .. } => {
                warn!(
                    session_id = %self.input.session_id,
                    "Message received while running - rejected"
                );
                // In a real implementation, this would return an error signal
                // For now, we just ignore it
                WorkflowAction::None
            }

            _ => {
                warn!(
                    session_id = %self.input.session_id,
                    state = ?std::mem::discriminant(&self.state),
                    "Message received in unexpected state"
                );
                WorkflowAction::None
            }
        }
    }

    fn handle_shutdown(&mut self) -> WorkflowAction {
        let (messages, turn_count) = match &self.state {
            SessionState::Waiting {
                messages,
                turn_count,
            } => (messages.clone(), *turn_count),
            SessionState::Running {
                messages,
                turn_count,
                ..
            } => (messages.clone(), *turn_count),
            _ => (Vec::new(), 0),
        };

        info!(
            session_id = %self.input.session_id,
            "Shutdown signal received"
        );

        self.state = SessionState::Completed {
            messages,
            turn_count,
        };

        WorkflowAction::Complete {
            output: SessionOutput {
                session_id: self.input.session_id,
                status: SessionStatus::Completed,
                total_turns: turn_count,
                error: None,
            },
        }
    }

    fn start_turn(&mut self, messages: Vec<Message>, turn_count: u32) -> WorkflowAction {
        let agent_config = match &self.agent_config {
            Some(config) => config.clone(),
            None => {
                return self.fail("Agent config not loaded");
            }
        };

        // Start LLM call
        let activity_id = self.next_activity_id("call-llm");
        let input = CallLlmInput {
            session_id: self.input.session_id,
            agent_config: agent_config.clone(),
            messages: messages.clone(),
        };

        self.state = SessionState::Running {
            agent_config,
            messages,
            iteration: 1,
            turn_count,
            pending_llm: Some(activity_id.clone()),
            pending_tools: Vec::new(),
        };

        WorkflowAction::ScheduleActivity {
            activity_id,
            activity_type: ActivityType::CallLlm,
            input: serde_json::to_value(&input).unwrap(),
        }
    }

    fn handle_llm_completed(
        &mut self,
        _activity_id: &str,
        result: serde_json::Value,
    ) -> WorkflowAction {
        let llm_response: LlmResponse = match serde_json::from_value(result) {
            Ok(resp) => resp,
            Err(e) => {
                return self.fail(&format!("Failed to parse LLM response: {}", e));
            }
        };

        // Extract current state
        let (agent_config, mut messages, iteration, turn_count) = match &self.state {
            SessionState::Running {
                agent_config,
                messages,
                iteration,
                turn_count,
                ..
            } => (
                agent_config.clone(),
                messages.clone(),
                *iteration,
                *turn_count,
            ),
            _ => return self.fail("Invalid state for LLM completion"),
        };

        info!(
            session_id = %self.input.session_id,
            turn = turn_count,
            iteration = iteration,
            has_tool_calls = !llm_response.tool_calls.is_empty(),
            "LLM response received"
        );

        // Add assistant message
        if llm_response.tool_calls.is_empty() {
            // No tool calls - add final response and complete turn
            messages.push(Message::assistant(&llm_response.text));

            self.state = SessionState::Waiting {
                messages,
                turn_count,
            };

            return WorkflowAction::WaitForSignal;
        }

        // Has tool calls - add assistant message with tool calls
        messages.push(Message::assistant_with_tool_calls(
            &llm_response.text,
            llm_response.tool_calls.clone(),
        ));

        // Check iteration limit
        if iteration >= MAX_ITERATIONS_PER_TURN {
            warn!(
                session_id = %self.input.session_id,
                "Max iterations reached, completing turn"
            );
            self.state = SessionState::Waiting {
                messages,
                turn_count,
            };
            return WorkflowAction::WaitForSignal;
        }

        // Schedule tool executions in parallel
        let mut activities = Vec::new();
        let mut pending_tools = Vec::new();

        for tool_call in &llm_response.tool_calls {
            let activity_id = self.next_activity_id(&format!("tool-{}", tool_call.name));

            let tool_def = agent_config
                .tools
                .iter()
                .find(|t| t.name == tool_call.name)
                .cloned();

            let input = ExecuteSingleToolInput {
                session_id: self.input.session_id,
                tool_call: tool_call.clone(),
                tool_definition: tool_def,
            };

            activities.push((
                activity_id.clone(),
                ActivityType::ExecuteTool,
                serde_json::to_value(&input).unwrap(),
            ));

            pending_tools.push(PendingTool {
                activity_id,
                tool_call_id: tool_call.id.clone(),
                tool_name: tool_call.name.clone(),
            });
        }

        self.state = SessionState::Running {
            agent_config,
            messages,
            iteration,
            turn_count,
            pending_llm: None,
            pending_tools,
        };

        WorkflowAction::ScheduleParallelActivities { activities }
    }

    fn handle_tool_completed(
        &mut self,
        activity_id: &str,
        result: serde_json::Value,
    ) -> WorkflowAction {
        let tool_output: ExecuteSingleToolOutput = match serde_json::from_value(result) {
            Ok(output) => output,
            Err(e) => {
                return self.fail(&format!("Failed to parse tool result: {}", e));
            }
        };

        // Extract and update state
        let (agent_config, mut messages, iteration, turn_count, mut pending_tools) =
            match &self.state {
                SessionState::Running {
                    agent_config,
                    messages,
                    iteration,
                    turn_count,
                    pending_tools,
                    ..
                } => (
                    agent_config.clone(),
                    messages.clone(),
                    *iteration,
                    *turn_count,
                    pending_tools.clone(),
                ),
                _ => return self.fail("Invalid state for tool completion"),
            };

        // Find and remove the completed tool
        let completed_idx = pending_tools
            .iter()
            .position(|pt| pt.activity_id == activity_id);

        let completed_tool = match completed_idx {
            Some(idx) => pending_tools.remove(idx),
            None => {
                warn!(
                    activity_id = %activity_id,
                    "Unknown tool activity completed"
                );
                return WorkflowAction::None;
            }
        };

        info!(
            session_id = %self.input.session_id,
            tool = %completed_tool.tool_name,
            remaining = pending_tools.len(),
            "Tool completed"
        );

        // Add tool result message
        if let Some(err) = &tool_output.result.error {
            messages.push(Message::tool_error(&completed_tool.tool_call_id, err));
        } else if let Some(result) = &tool_output.result.result {
            messages.push(Message::tool_result(
                &completed_tool.tool_call_id,
                result.clone(),
            ));
        }

        // Check if all tools are done
        if !pending_tools.is_empty() {
            // Still waiting for more tools
            self.state = SessionState::Running {
                agent_config,
                messages,
                iteration,
                turn_count,
                pending_llm: None,
                pending_tools,
            };
            return WorkflowAction::None;
        }

        // All tools done - call LLM again
        let activity_id = self.next_activity_id("call-llm");
        let input = CallLlmInput {
            session_id: self.input.session_id,
            agent_config: agent_config.clone(),
            messages: messages.clone(),
        };

        self.state = SessionState::Running {
            agent_config,
            messages,
            iteration: iteration + 1,
            turn_count,
            pending_llm: Some(activity_id.clone()),
            pending_tools: Vec::new(),
        };

        WorkflowAction::ScheduleActivity {
            activity_id,
            activity_type: ActivityType::CallLlm,
            input: serde_json::to_value(&input).unwrap(),
        }
    }

    fn fail(&mut self, error: &str) -> WorkflowAction {
        let (messages, turn_count) = match &self.state {
            SessionState::Running {
                messages,
                turn_count,
                ..
            } => (messages.clone(), *turn_count),
            SessionState::Waiting {
                messages,
                turn_count,
            } => (messages.clone(), *turn_count),
            _ => (Vec::new(), 0),
        };

        self.state = SessionState::Failed {
            error: error.to_string(),
            messages,
            turn_count,
        };

        WorkflowAction::Complete {
            output: SessionOutput {
                session_id: self.input.session_id,
                status: SessionStatus::Failed,
                total_turns: turn_count,
                error: Some(error.to_string()),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_input() -> SessionInput {
        SessionInput::new(Uuid::now_v7())
    }

    fn test_agent_config() -> AgentConfig {
        AgentConfig::test("test-agent")
            .with_system_prompt("You are helpful")
            .with_tool(ToolDefinition::new("get_time", "Get current time"))
    }

    #[test]
    fn test_workflow_start() {
        let input = test_input();
        let mut workflow = SessionWorkflow::new(input.clone());

        let action = workflow.on_start();

        assert!(matches!(
            action,
            WorkflowAction::ScheduleActivity {
                activity_type: ActivityType::LoadAgent,
                ..
            }
        ));
        assert!(matches!(workflow.state(), SessionState::Initializing));
    }

    #[test]
    fn test_workflow_load_agent() {
        let input = test_input();
        let mut workflow = SessionWorkflow::new(input);

        workflow.on_start();

        let agent_config = test_agent_config();
        let action = workflow
            .on_activity_completed("load-agent-1", serde_json::to_value(&agent_config).unwrap());

        assert!(matches!(action, WorkflowAction::WaitForSignal));
        assert!(workflow.is_waiting());
    }

    #[test]
    fn test_workflow_new_message_starts_turn() {
        let input = test_input();
        let mut workflow = SessionWorkflow::new(input);

        // Initialize
        workflow.on_start();
        let agent_config = test_agent_config();
        workflow
            .on_activity_completed("load-agent-1", serde_json::to_value(&agent_config).unwrap());

        // Send message
        let signal = WorkflowSignal::NewMessage(NewMessageSignal {
            message: Message::user("Hello"),
        });
        let action = workflow.on_signal(signal);

        assert!(matches!(
            action,
            WorkflowAction::ScheduleActivity {
                activity_type: ActivityType::CallLlm,
                ..
            }
        ));
        assert!(workflow.is_running());
    }

    #[test]
    fn test_workflow_message_rejected_while_running() {
        let input = test_input();
        let mut workflow = SessionWorkflow::new(input);

        // Initialize and start turn
        workflow.on_start();
        let agent_config = test_agent_config();
        workflow
            .on_activity_completed("load-agent-1", serde_json::to_value(&agent_config).unwrap());

        let signal = WorkflowSignal::NewMessage(NewMessageSignal {
            message: Message::user("Hello"),
        });
        workflow.on_signal(signal);

        // Try to send another message while running
        let signal2 = WorkflowSignal::NewMessage(NewMessageSignal {
            message: Message::user("Another message"),
        });
        let action = workflow.on_signal(signal2);

        assert!(matches!(action, WorkflowAction::None));
        assert!(workflow.is_running());
    }

    #[test]
    fn test_workflow_llm_response_without_tools() {
        let input = test_input();
        let mut workflow = SessionWorkflow::new(input);

        // Initialize and start turn
        workflow.on_start();
        let agent_config = test_agent_config();
        workflow
            .on_activity_completed("load-agent-1", serde_json::to_value(&agent_config).unwrap());

        let signal = WorkflowSignal::NewMessage(NewMessageSignal {
            message: Message::user("Hello"),
        });
        workflow.on_signal(signal);

        // LLM responds without tools
        let llm_response = LlmResponse::text("Hello! How can I help?");
        let action = workflow
            .on_activity_completed("call-llm-2", serde_json::to_value(&llm_response).unwrap());

        assert!(matches!(action, WorkflowAction::WaitForSignal));
        assert!(workflow.is_waiting());

        // Check messages
        if let SessionState::Waiting { messages, .. } = workflow.state() {
            assert_eq!(messages.len(), 2); // user + assistant
            assert_eq!(messages[0].role, MessageRole::User);
            assert_eq!(messages[1].role, MessageRole::Assistant);
        } else {
            panic!("Expected Waiting state");
        }
    }

    #[test]
    fn test_workflow_llm_response_with_tools() {
        let input = test_input();
        let mut workflow = SessionWorkflow::new(input);

        // Initialize and start turn
        workflow.on_start();
        let agent_config = test_agent_config();
        workflow
            .on_activity_completed("load-agent-1", serde_json::to_value(&agent_config).unwrap());

        let signal = WorkflowSignal::NewMessage(NewMessageSignal {
            message: Message::user("What time is it?"),
        });
        workflow.on_signal(signal);

        // LLM responds with tool call
        let llm_response = LlmResponse::with_tools(
            "Let me check the time",
            vec![ToolCall::new("get_time", serde_json::json!({}))],
        );
        let action = workflow
            .on_activity_completed("call-llm-2", serde_json::to_value(&llm_response).unwrap());

        assert!(matches!(
            action,
            WorkflowAction::ScheduleParallelActivities { activities }
            if activities.len() == 1
        ));
        assert!(workflow.is_running());
    }

    #[test]
    fn test_workflow_tool_completion() {
        let input = test_input();
        let mut workflow = SessionWorkflow::new(input);

        // Initialize and start turn
        workflow.on_start();
        let agent_config = test_agent_config();
        workflow
            .on_activity_completed("load-agent-1", serde_json::to_value(&agent_config).unwrap());

        workflow.on_signal(WorkflowSignal::NewMessage(NewMessageSignal {
            message: Message::user("What time is it?"),
        }));

        // LLM responds with tool call
        let llm_response = LlmResponse::with_tools(
            "Let me check",
            vec![ToolCall {
                id: "call_123".to_string(),
                name: "get_time".to_string(),
                arguments: serde_json::json!({}),
            }],
        );
        workflow.on_activity_completed("call-llm-2", serde_json::to_value(&llm_response).unwrap());

        // Tool completes
        let tool_result = ExecuteSingleToolOutput {
            result: ToolResult::success("call_123", serde_json::json!({"time": "12:00"})),
        };
        let action = workflow.on_activity_completed(
            "tool-get_time-3",
            serde_json::to_value(&tool_result).unwrap(),
        );

        // Should call LLM again
        assert!(matches!(
            action,
            WorkflowAction::ScheduleActivity {
                activity_type: ActivityType::CallLlm,
                ..
            }
        ));
    }

    #[test]
    fn test_workflow_parallel_tools() {
        let input = test_input();
        let mut workflow = SessionWorkflow::new(input);

        // Initialize
        workflow.on_start();
        let agent_config = AgentConfig::test("test-agent")
            .with_tool(ToolDefinition::new("get_time", "Get time"))
            .with_tool(ToolDefinition::new("get_weather", "Get weather"));
        workflow
            .on_activity_completed("load-agent-1", serde_json::to_value(&agent_config).unwrap());

        workflow.on_signal(WorkflowSignal::NewMessage(NewMessageSignal {
            message: Message::user("What's the time and weather?"),
        }));

        // LLM responds with multiple tool calls
        let llm_response = LlmResponse::with_tools(
            "Let me check both",
            vec![
                ToolCall {
                    id: "call_1".to_string(),
                    name: "get_time".to_string(),
                    arguments: serde_json::json!({}),
                },
                ToolCall {
                    id: "call_2".to_string(),
                    name: "get_weather".to_string(),
                    arguments: serde_json::json!({}),
                },
            ],
        );
        let action = workflow
            .on_activity_completed("call-llm-2", serde_json::to_value(&llm_response).unwrap());

        // Should schedule 2 parallel activities
        assert!(matches!(
            action,
            WorkflowAction::ScheduleParallelActivities { activities }
            if activities.len() == 2
        ));

        // First tool completes
        let tool_result1 = ExecuteSingleToolOutput {
            result: ToolResult::success("call_1", serde_json::json!({"time": "12:00"})),
        };
        let action = workflow.on_activity_completed(
            "tool-get_time-3",
            serde_json::to_value(&tool_result1).unwrap(),
        );

        // Should wait for second tool
        assert!(matches!(action, WorkflowAction::None));

        // Second tool completes
        let tool_result2 = ExecuteSingleToolOutput {
            result: ToolResult::success("call_2", serde_json::json!({"temp": 72})),
        };
        let action = workflow.on_activity_completed(
            "tool-get_weather-4",
            serde_json::to_value(&tool_result2).unwrap(),
        );

        // Now should call LLM again
        assert!(matches!(
            action,
            WorkflowAction::ScheduleActivity {
                activity_type: ActivityType::CallLlm,
                ..
            }
        ));
    }

    #[test]
    fn test_workflow_shutdown() {
        let input = test_input();
        let mut workflow = SessionWorkflow::new(input);

        workflow.on_start();
        let agent_config = test_agent_config();
        workflow
            .on_activity_completed("load-agent-1", serde_json::to_value(&agent_config).unwrap());

        let action = workflow.on_signal(WorkflowSignal::Shutdown);

        assert!(matches!(
            action,
            WorkflowAction::Complete { output }
            if output.status == SessionStatus::Completed
        ));
        assert!(workflow.is_terminal());
    }

    #[test]
    fn test_workflow_activity_failure() {
        let input = test_input();
        let mut workflow = SessionWorkflow::new(input);

        workflow.on_start();
        let agent_config = test_agent_config();
        workflow
            .on_activity_completed("load-agent-1", serde_json::to_value(&agent_config).unwrap());

        workflow.on_signal(WorkflowSignal::NewMessage(NewMessageSignal {
            message: Message::user("Hello"),
        }));

        let action = workflow.on_activity_failed("call-llm-2", "Connection timeout");

        assert!(matches!(
            action,
            WorkflowAction::Complete { output }
            if output.status == SessionStatus::Failed
        ));
        assert!(workflow.is_terminal());
    }

    #[test]
    fn test_multiple_turns() {
        let input = test_input();
        let mut workflow = SessionWorkflow::new(input);

        // Initialize
        workflow.on_start();
        let agent_config = test_agent_config();
        workflow
            .on_activity_completed("load-agent-1", serde_json::to_value(&agent_config).unwrap());

        // First turn
        workflow.on_signal(WorkflowSignal::NewMessage(NewMessageSignal {
            message: Message::user("Hello"),
        }));
        workflow.on_activity_completed(
            "call-llm-2",
            serde_json::to_value(&LlmResponse::text("Hi there!")).unwrap(),
        );

        assert!(workflow.is_waiting());

        // Check turn count
        if let SessionState::Waiting { turn_count, .. } = workflow.state() {
            assert_eq!(*turn_count, 1);
        }

        // Second turn
        workflow.on_signal(WorkflowSignal::NewMessage(NewMessageSignal {
            message: Message::user("How are you?"),
        }));
        workflow.on_activity_completed(
            "call-llm-3",
            serde_json::to_value(&LlmResponse::text("I'm doing well!")).unwrap(),
        );

        // Check turn count
        if let SessionState::Waiting {
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

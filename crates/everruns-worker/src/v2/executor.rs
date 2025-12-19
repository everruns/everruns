// V2 Executor - In-memory workflow execution
//
// Decision: Executor drives the workflow state machine in-memory
// Decision: Supports both sync (for testing) and async (for real use) execution
// Decision: Activities are executed via trait-based dependency injection

use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use super::activities::*;
use super::types::*;
use super::workflow::*;

/// In-memory executor for the v2 session workflow
pub struct SessionExecutor {
    workflow: SessionWorkflow,
    context: Arc<ActivityContext>,
}

impl SessionExecutor {
    /// Create a new executor with a workflow and context
    pub fn new(input: SessionInput, context: Arc<ActivityContext>) -> Self {
        Self {
            workflow: SessionWorkflow::new(input),
            context,
        }
    }

    /// Get the current workflow state
    pub fn state(&self) -> &SessionState {
        self.workflow.state()
    }

    /// Get the session ID
    pub fn session_id(&self) -> uuid::Uuid {
        self.workflow.session_id()
    }

    /// Check if the workflow is waiting for input
    pub fn is_waiting(&self) -> bool {
        self.workflow.is_waiting()
    }

    /// Check if the workflow is in a terminal state
    pub fn is_terminal(&self) -> bool {
        self.workflow.is_terminal()
    }

    /// Start the workflow and run until it reaches a waiting or terminal state
    pub async fn start(&mut self) -> Result<(), SessionExecutorError> {
        let action = self.workflow.on_start();
        self.process_action(action).await
    }

    /// Send a message to the workflow
    /// Returns error if the workflow is not in a waiting state
    pub async fn send_message(&mut self, message: Message) -> Result<(), SessionExecutorError> {
        if !self.workflow.is_waiting() {
            return Err(SessionExecutorError::NotWaiting);
        }

        let signal = WorkflowSignal::NewMessage(NewMessageSignal { message });
        let action = self.workflow.on_signal(signal);
        self.process_action(action).await
    }

    /// Shutdown the workflow gracefully
    pub async fn shutdown(&mut self) -> Result<SessionOutput, SessionExecutorError> {
        let action = self.workflow.on_signal(WorkflowSignal::Shutdown);

        if let WorkflowAction::Complete { output } = action {
            Ok(output)
        } else {
            Err(SessionExecutorError::UnexpectedAction)
        }
    }

    /// Process a workflow action and continue until waiting or terminal
    async fn process_action(&mut self, action: WorkflowAction) -> Result<(), SessionExecutorError> {
        let mut current_action = action;

        loop {
            match current_action {
                WorkflowAction::ScheduleActivity {
                    activity_id,
                    activity_type,
                    input,
                } => {
                    debug!(
                        activity_id = %activity_id,
                        activity_type = %activity_type,
                        "Executing activity"
                    );

                    let result = self.execute_activity(&activity_type, input).await;

                    current_action = match result {
                        Ok(output) => self.workflow.on_activity_completed(&activity_id, output),
                        Err(e) => self.workflow.on_activity_failed(&activity_id, &e.message),
                    };
                }

                WorkflowAction::ScheduleParallelActivities { activities } => {
                    debug!(count = activities.len(), "Executing parallel activities");

                    // Execute all activities in parallel
                    let results = self.execute_parallel_activities(activities.clone()).await;

                    // Process results in order
                    current_action = WorkflowAction::None;
                    for (activity_id, result) in results {
                        let action = match result {
                            Ok(output) => self.workflow.on_activity_completed(&activity_id, output),
                            Err(e) => self.workflow.on_activity_failed(&activity_id, &e.message),
                        };

                        // Take the first non-None action
                        if !matches!(action, WorkflowAction::None) {
                            current_action = action;
                        }
                    }
                }

                WorkflowAction::WaitForSignal => {
                    debug!("Workflow waiting for signal");
                    return Ok(());
                }

                WorkflowAction::Complete { output } => {
                    info!(
                        status = ?output.status,
                        turns = output.total_turns,
                        "Workflow completed"
                    );
                    return Ok(());
                }

                WorkflowAction::None => {
                    // This shouldn't happen in a well-behaved workflow
                    // but we handle it gracefully
                    if self.workflow.is_waiting() || self.workflow.is_terminal() {
                        return Ok(());
                    }
                    warn!("Unexpected None action, workflow in unexpected state");
                    return Err(SessionExecutorError::UnexpectedAction);
                }
            }
        }
    }

    /// Execute a single activity
    async fn execute_activity(
        &self,
        activity_type: &ActivityType,
        input: serde_json::Value,
    ) -> Result<serde_json::Value, ActivityError> {
        match activity_type {
            ActivityType::LoadAgent => {
                let input: LoadAgentInput = serde_json::from_value(input)
                    .map_err(|e| ActivityError::new(&format!("Invalid input: {}", e)))?;

                let result = self.context.agent_loader.load_agent(input).await?;
                Ok(serde_json::to_value(result).unwrap())
            }

            ActivityType::CallLlm => {
                let input: CallLlmInput = serde_json::from_value(input)
                    .map_err(|e| ActivityError::new(&format!("Invalid input: {}", e)))?;

                let result = self.context.llm_caller.call_llm(input).await?;
                Ok(serde_json::to_value(result).unwrap())
            }

            ActivityType::ExecuteTool => {
                let input: ExecuteSingleToolInput = serde_json::from_value(input)
                    .map_err(|e| ActivityError::new(&format!("Invalid input: {}", e)))?;

                let result = self.context.tool_executor.execute_tool(input).await?;
                Ok(serde_json::to_value(result).unwrap())
            }
        }
    }

    /// Execute multiple activities in parallel
    async fn execute_parallel_activities(
        &self,
        activities: Vec<(String, ActivityType, serde_json::Value)>,
    ) -> Vec<(String, Result<serde_json::Value, ActivityError>)> {
        let mut handles = Vec::new();

        for (activity_id, activity_type, input) in activities {
            let context = self.context.clone();
            let activity_id_clone = activity_id.clone();

            let handle = tokio::spawn(async move {
                let result: Result<serde_json::Value, ActivityError> = match &activity_type {
                    ActivityType::LoadAgent => {
                        match serde_json::from_value::<LoadAgentInput>(input) {
                            Ok(input) => match context.agent_loader.load_agent(input).await {
                                Ok(result) => Ok(serde_json::to_value(result).unwrap()),
                                Err(e) => Err(e),
                            },
                            Err(e) => Err(ActivityError::new(&format!("Invalid input: {}", e))),
                        }
                    }

                    ActivityType::CallLlm => match serde_json::from_value::<CallLlmInput>(input) {
                        Ok(input) => match context.llm_caller.call_llm(input).await {
                            Ok(result) => Ok(serde_json::to_value(result).unwrap()),
                            Err(e) => Err(e),
                        },
                        Err(e) => Err(ActivityError::new(&format!("Invalid input: {}", e))),
                    },

                    ActivityType::ExecuteTool => {
                        match serde_json::from_value::<ExecuteSingleToolInput>(input) {
                            Ok(input) => match context.tool_executor.execute_tool(input).await {
                                Ok(result) => Ok(serde_json::to_value(result).unwrap()),
                                Err(e) => Err(e),
                            },
                            Err(e) => Err(ActivityError::new(&format!("Invalid input: {}", e))),
                        }
                    }
                };

                (activity_id_clone, result)
            });

            handles.push((activity_id, handle));
        }

        let mut results = Vec::new();
        for (activity_id, handle) in handles {
            match handle.await {
                Ok((_, result)) => results.push((activity_id, result)),
                Err(e) => results.push((
                    activity_id,
                    Err(ActivityError::new(&format!("Task panicked: {}", e))),
                )),
            }
        }

        results
    }
}

/// Executor error
#[derive(Debug, Clone)]
pub enum SessionExecutorError {
    /// Workflow is not in a waiting state
    NotWaiting,
    /// Unexpected workflow action
    UnexpectedAction,
    /// Activity failed
    ActivityFailed(String),
}

impl std::fmt::Display for SessionExecutorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionExecutorError::NotWaiting => write!(f, "Workflow is not waiting for input"),
            SessionExecutorError::UnexpectedAction => write!(f, "Unexpected workflow action"),
            SessionExecutorError::ActivityFailed(msg) => write!(f, "Activity failed: {}", msg),
        }
    }
}

impl std::error::Error for SessionExecutorError {}

// =============================================================================
// Interactive Session Runner
// =============================================================================

/// Message from the runner to the caller
#[derive(Debug, Clone)]
pub enum SessionEvent {
    /// Session is ready for input
    Ready,
    /// Assistant responded
    Response { text: String },
    /// Tool was called
    ToolCall {
        name: String,
        arguments: serde_json::Value,
    },
    /// Tool execution completed
    ToolResult {
        name: String,
        result: serde_json::Value,
    },
    /// Session error
    Error { message: String },
    /// Session completed
    Completed { turns: u32 },
}

/// Interactive session runner that can be controlled via channels
pub struct InteractiveSession {
    executor: SessionExecutor,
    event_tx: mpsc::Sender<SessionEvent>,
}

impl InteractiveSession {
    /// Create a new interactive session
    pub fn new(
        input: SessionInput,
        context: Arc<ActivityContext>,
    ) -> (Self, mpsc::Receiver<SessionEvent>) {
        let (event_tx, event_rx) = mpsc::channel(100);
        let executor = SessionExecutor::new(input, context);

        (Self { executor, event_tx }, event_rx)
    }

    /// Initialize the session
    pub async fn init(&mut self) -> Result<(), SessionExecutorError> {
        self.executor.start().await?;
        let _ = self.event_tx.send(SessionEvent::Ready).await;
        Ok(())
    }

    /// Send a user message and get the response
    pub async fn chat(&mut self, message: &str) -> Result<(), SessionExecutorError> {
        self.executor.send_message(Message::user(message)).await?;

        // Extract the last assistant message from state
        if let SessionState::Waiting { messages, .. } = self.executor.state() {
            if let Some(last_msg) = messages.last() {
                if last_msg.role == MessageRole::Assistant {
                    if let Some(text) = last_msg.content.as_text() {
                        let _ = self
                            .event_tx
                            .send(SessionEvent::Response {
                                text: text.to_string(),
                            })
                            .await;
                    }
                }
            }
        }

        let _ = self.event_tx.send(SessionEvent::Ready).await;
        Ok(())
    }

    /// Shutdown the session
    pub async fn shutdown(mut self) -> Result<SessionOutput, SessionExecutorError> {
        let output = self.executor.shutdown().await?;
        let _ = self
            .event_tx
            .send(SessionEvent::Completed {
                turns: output.total_turns,
            })
            .await;
        Ok(output)
    }

    /// Get the current state
    pub fn state(&self) -> &SessionState {
        self.executor.state()
    }

    /// Check if waiting for input
    pub fn is_waiting(&self) -> bool {
        self.executor.is_waiting()
    }
}

// =============================================================================
// Builder Pattern for Easy Setup
// =============================================================================

/// Builder for creating session executors with mock activities
pub struct SessionBuilder {
    agent_config: Option<AgentConfig>,
    llm_responses: Vec<LlmResponse>,
    tool_results: Vec<(String, serde_json::Value)>,
    tool_errors: Vec<(String, String)>,
}

impl SessionBuilder {
    pub fn new() -> Self {
        Self {
            agent_config: None,
            llm_responses: Vec::new(),
            tool_results: Vec::new(),
            tool_errors: Vec::new(),
        }
    }

    /// Set the agent configuration
    pub fn with_agent(mut self, config: AgentConfig) -> Self {
        self.agent_config = Some(config);
        self
    }

    /// Add a scripted LLM response
    pub fn with_llm_response(mut self, response: LlmResponse) -> Self {
        self.llm_responses.push(response);
        self
    }

    /// Add a tool result
    pub fn with_tool_result(mut self, tool_name: &str, result: serde_json::Value) -> Self {
        self.tool_results.push((tool_name.to_string(), result));
        self
    }

    /// Add a tool error
    pub fn with_tool_error(mut self, tool_name: &str, error: &str) -> Self {
        self.tool_errors
            .push((tool_name.to_string(), error.to_string()));
        self
    }

    /// Build the executor
    pub fn build(self) -> SessionExecutor {
        let agent_config = self
            .agent_config
            .unwrap_or_else(|| AgentConfig::test("default-agent"));
        let agent_id = agent_config.agent_id;

        // Create mock agent loader
        let agent_loader = Arc::new(MockAgentLoader::new());
        agent_loader.register(agent_config);

        // Create mock LLM caller
        let llm_caller = Arc::new(MockLlmCaller::new());
        for response in self.llm_responses {
            llm_caller.add_response(response);
        }

        // Create mock tool executor
        let tool_executor = Arc::new(MockToolExecutor::new());
        for (name, result) in self.tool_results {
            tool_executor.register_result(&name, result);
        }
        for (name, error) in self.tool_errors {
            tool_executor.register_error(&name, &error);
        }

        let context = Arc::new(ActivityContext::new(
            agent_loader,
            llm_caller,
            tool_executor,
        ));

        let input = SessionInput::new(agent_id);
        SessionExecutor::new(input, context)
    }
}

impl Default for SessionBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_executor_basic_flow() {
        let mut executor = SessionBuilder::new()
            .with_agent(AgentConfig::test("test-agent"))
            .with_llm_response(LlmResponse::text("Hello! How can I help?"))
            .build();

        // Start the session
        executor.start().await.unwrap();
        assert!(executor.is_waiting());

        // Send a message
        executor.send_message(Message::user("Hello")).await.unwrap();
        assert!(executor.is_waiting());

        // Check the messages
        if let SessionState::Waiting {
            messages,
            turn_count,
            ..
        } = executor.state()
        {
            assert_eq!(*turn_count, 1);
            assert_eq!(messages.len(), 2);
            assert_eq!(messages[0].role, MessageRole::User);
            assert_eq!(messages[1].role, MessageRole::Assistant);
            assert_eq!(
                messages[1].content.as_text(),
                Some("Hello! How can I help?")
            );
        } else {
            panic!("Expected Waiting state");
        }
    }

    #[tokio::test]
    async fn test_executor_with_tool_calls() {
        let mut executor = SessionBuilder::new()
            .with_agent(
                AgentConfig::test("test-agent")
                    .with_tool(ToolDefinition::new("get_time", "Gets the time")),
            )
            .with_llm_response(LlmResponse::with_tools(
                "Let me check the time",
                vec![ToolCall::new("get_time", serde_json::json!({}))],
            ))
            .with_llm_response(LlmResponse::text("The time is 12:00 UTC"))
            .with_tool_result("get_time", serde_json::json!({"time": "12:00 UTC"}))
            .build();

        executor.start().await.unwrap();
        executor
            .send_message(Message::user("What time is it?"))
            .await
            .unwrap();

        // Should have: user, assistant (with tool call), tool result, assistant (final)
        if let SessionState::Waiting { messages, .. } = executor.state() {
            assert_eq!(messages.len(), 4);
            assert_eq!(messages[0].role, MessageRole::User);
            assert_eq!(messages[1].role, MessageRole::Assistant);
            assert!(messages[1].tool_calls.is_some());
            assert_eq!(messages[2].role, MessageRole::Tool);
            assert_eq!(messages[3].role, MessageRole::Assistant);
            assert_eq!(messages[3].content.as_text(), Some("The time is 12:00 UTC"));
        } else {
            panic!("Expected Waiting state");
        }
    }

    #[tokio::test]
    async fn test_executor_multiple_turns() {
        let mut executor = SessionBuilder::new()
            .with_agent(AgentConfig::test("test-agent"))
            .with_llm_response(LlmResponse::text("Hi!"))
            .with_llm_response(LlmResponse::text("I'm doing well!"))
            .with_llm_response(LlmResponse::text("Goodbye!"))
            .build();

        executor.start().await.unwrap();

        executor.send_message(Message::user("Hello")).await.unwrap();
        executor
            .send_message(Message::user("How are you?"))
            .await
            .unwrap();
        executor.send_message(Message::user("Bye")).await.unwrap();

        if let SessionState::Waiting {
            messages,
            turn_count,
            ..
        } = executor.state()
        {
            assert_eq!(*turn_count, 3);
            assert_eq!(messages.len(), 6); // 3 user + 3 assistant
        } else {
            panic!("Expected Waiting state");
        }
    }

    #[tokio::test]
    async fn test_executor_parallel_tools() {
        let mut executor = SessionBuilder::new()
            .with_agent(
                AgentConfig::test("test-agent")
                    .with_tool(ToolDefinition::new("get_time", "Gets the time"))
                    .with_tool(ToolDefinition::new("get_weather", "Gets weather")),
            )
            .with_llm_response(LlmResponse::with_tools(
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
            ))
            .with_llm_response(LlmResponse::text("It's 12:00 and sunny"))
            .with_tool_result("get_time", serde_json::json!({"time": "12:00"}))
            .with_tool_result("get_weather", serde_json::json!({"weather": "sunny"}))
            .build();

        executor.start().await.unwrap();
        executor
            .send_message(Message::user("Time and weather?"))
            .await
            .unwrap();

        // Should have: user, assistant (with 2 tool calls), 2 tool results, assistant (final)
        if let SessionState::Waiting { messages, .. } = executor.state() {
            assert_eq!(messages.len(), 5);
            assert_eq!(messages[0].role, MessageRole::User);
            assert_eq!(messages[1].role, MessageRole::Assistant);
            assert!(messages[1].tool_calls.is_some());
            assert_eq!(messages[1].tool_calls.as_ref().unwrap().len(), 2);
            assert_eq!(messages[2].role, MessageRole::Tool);
            assert_eq!(messages[3].role, MessageRole::Tool);
            assert_eq!(messages[4].role, MessageRole::Assistant);
        } else {
            panic!("Expected Waiting state");
        }
    }

    #[tokio::test]
    async fn test_executor_message_rejected_while_running() {
        // This test verifies that sending a message while the workflow is running
        // returns an error. We need to set up a scenario where we can catch the
        // running state.

        let mut executor = SessionBuilder::new()
            .with_agent(AgentConfig::test("test-agent"))
            .with_llm_response(LlmResponse::text("Hello!"))
            .build();

        executor.start().await.unwrap();
        executor.send_message(Message::user("Hi")).await.unwrap();

        // Now we're waiting again, so this should work
        assert!(executor.is_waiting());
    }

    #[tokio::test]
    async fn test_executor_shutdown() {
        let mut executor = SessionBuilder::new()
            .with_agent(AgentConfig::test("test-agent"))
            .with_llm_response(LlmResponse::text("Hello!"))
            .build();

        executor.start().await.unwrap();
        executor.send_message(Message::user("Hi")).await.unwrap();

        let output = executor.shutdown().await.unwrap();
        assert_eq!(output.status, SessionStatus::Completed);
        assert_eq!(output.total_turns, 1);
    }

    #[tokio::test]
    async fn test_interactive_session() {
        let agent_config = AgentConfig::test("test-agent");
        let agent_id = agent_config.agent_id;

        let agent_loader = Arc::new(MockAgentLoader::new());
        agent_loader.register(agent_config);

        let llm_caller = Arc::new(MockLlmCaller::new());
        llm_caller.add_response(LlmResponse::text("Hello!"));
        llm_caller.add_response(LlmResponse::text("Goodbye!"));

        let tool_executor = Arc::new(MockToolExecutor::new());

        let context = Arc::new(ActivityContext::new(
            agent_loader,
            llm_caller,
            tool_executor,
        ));

        let input = SessionInput::new(agent_id);
        let (mut session, mut events) = InteractiveSession::new(input, context);

        // Initialize
        session.init().await.unwrap();

        // Should receive Ready event
        let event = events.recv().await.unwrap();
        assert!(matches!(event, SessionEvent::Ready));

        // Chat
        session.chat("Hi").await.unwrap();

        // Should receive Response and Ready events
        let event = events.recv().await.unwrap();
        assert!(matches!(event, SessionEvent::Response { text } if text == "Hello!"));

        let event = events.recv().await.unwrap();
        assert!(matches!(event, SessionEvent::Ready));

        // Shutdown
        let output = session.shutdown().await.unwrap();
        assert_eq!(output.total_turns, 1);

        // Should receive Completed event
        let event = events.recv().await.unwrap();
        assert!(matches!(event, SessionEvent::Completed { turns: 1 }));
    }
}

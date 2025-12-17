// Agent Loop Executor
//
// The main orchestrator for the agentic loop. Coordinates:
// - Loading messages from MessageStore
// - Calling LLM via LlmProvider
// - Executing tools via ToolExecutor
// - Emitting events via EventEmitter

use std::sync::Arc;

use everruns_contracts::events::{AgUiEvent, AgUiMessageRole};
use futures::StreamExt;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::config::AgentConfig;
use crate::error::{AgentLoopError, Result};
use crate::events::LoopEvent;
use crate::message::{ConversationMessage, MessageRole};
use crate::step::{LoopStep, StepInput, StepOutput, StepResult};
use crate::traits::{
    EventEmitter, LlmCallConfig, LlmMessage, LlmMessageRole, LlmProvider, LlmStreamEvent,
    MessageStore, ToolExecutor,
};

/// Result of a complete loop execution
#[derive(Debug, Clone)]
pub struct LoopResult {
    /// Session ID
    pub session_id: Uuid,
    /// Final messages (including all responses)
    pub messages: Vec<ConversationMessage>,
    /// Total iterations executed
    pub iterations: usize,
    /// Final assistant response text (if any)
    pub final_response: Option<String>,
}

/// The Agent Loop executor
///
/// Orchestrates the agentic loop with pluggable backends for:
/// - Event emission (EventEmitter)
/// - Message storage (MessageStore)
/// - LLM calls (LlmProvider)
/// - Tool execution (ToolExecutor)
pub struct AgentLoop<E, M, L, T>
where
    E: EventEmitter,
    M: MessageStore,
    L: LlmProvider,
    T: ToolExecutor,
{
    /// Configuration for this agent
    config: AgentConfig,
    /// Event emitter for streaming
    event_emitter: Arc<E>,
    /// Message store for persistence
    message_store: Arc<M>,
    /// LLM provider for inference
    llm_provider: Arc<L>,
    /// Tool executor for tool calls
    tool_executor: Arc<T>,
}

impl<E, M, L, T> AgentLoop<E, M, L, T>
where
    E: EventEmitter,
    M: MessageStore,
    L: LlmProvider,
    T: ToolExecutor,
{
    /// Create a new agent loop
    pub fn new(
        config: AgentConfig,
        event_emitter: E,
        message_store: M,
        llm_provider: L,
        tool_executor: T,
    ) -> Self {
        Self {
            config,
            event_emitter: Arc::new(event_emitter),
            message_store: Arc::new(message_store),
            llm_provider: Arc::new(llm_provider),
            tool_executor: Arc::new(tool_executor),
        }
    }

    /// Create a new agent loop with Arc-wrapped components
    pub fn with_arcs(
        config: AgentConfig,
        event_emitter: Arc<E>,
        message_store: Arc<M>,
        llm_provider: Arc<L>,
        tool_executor: Arc<T>,
    ) -> Self {
        Self {
            config,
            event_emitter,
            message_store,
            llm_provider,
            tool_executor,
        }
    }

    /// Get the configuration
    pub fn config(&self) -> &AgentConfig {
        &self.config
    }

    /// Run the complete agentic loop for a session
    ///
    /// This is the main entry point for in-process execution.
    pub async fn run(&self, session_id: Uuid) -> Result<LoopResult> {
        info!(session_id = %session_id, "Starting agent loop");

        // Emit loop started event
        self.event_emitter
            .emit(LoopEvent::loop_started(session_id.to_string()))
            .await?;

        // Emit AG-UI session started event
        self.event_emitter
            .emit(LoopEvent::ag_ui(AgUiEvent::session_started(
                session_id.to_string(),
            )))
            .await?;

        // Load existing messages
        let mut messages = self.message_store.load(session_id).await?;

        if messages.is_empty() {
            warn!(session_id = %session_id, "No messages to process");
            return Err(AgentLoopError::NoMessages);
        }

        // Run the loop
        let mut iteration = 0;
        let mut final_response = None;

        loop {
            iteration += 1;

            if iteration > self.config.max_iterations {
                warn!(
                    session_id = %session_id,
                    max = self.config.max_iterations,
                    "Max iterations reached"
                );
                self.event_emitter
                    .emit(LoopEvent::loop_error(
                        session_id.to_string(),
                        format!("Max iterations ({}) reached", self.config.max_iterations),
                    ))
                    .await?;
                return Err(AgentLoopError::MaxIterationsReached(
                    self.config.max_iterations,
                ));
            }

            info!(
                session_id = %session_id,
                iteration = iteration,
                "Starting iteration"
            );

            // Emit iteration started
            self.event_emitter
                .emit(LoopEvent::iteration_started(
                    session_id.to_string(),
                    iteration,
                ))
                .await?;

            // Call LLM
            let llm_result = self.call_llm(session_id, iteration, &messages).await?;

            // Check if we have tool calls to execute
            let has_tool_calls = llm_result
                .tool_calls
                .as_ref()
                .is_some_and(|tc| !tc.is_empty());

            // Store assistant response as message (even with empty text if there are tool_calls)
            if !llm_result.text.is_empty() || has_tool_calls {
                let assistant_msg = if let Some(ref tool_calls) = llm_result.tool_calls {
                    ConversationMessage::assistant_with_tools(&llm_result.text, tool_calls.clone())
                } else {
                    ConversationMessage::assistant(&llm_result.text)
                };

                self.message_store
                    .store(session_id, assistant_msg.clone())
                    .await?;
                messages.push(assistant_msg);
                if !llm_result.text.is_empty() {
                    final_response = Some(llm_result.text.clone());
                }
            }

            // Emit LLM completed
            self.event_emitter
                .emit(LoopEvent::llm_call_completed(
                    session_id.to_string(),
                    iteration,
                    has_tool_calls,
                ))
                .await?;

            if has_tool_calls {
                let tool_calls = llm_result.tool_calls.unwrap();

                // Store tool call messages
                for tool_call in &tool_calls {
                    let tool_call_msg = ConversationMessage::tool_call(tool_call);
                    self.message_store
                        .store(session_id, tool_call_msg.clone())
                        .await?;
                    messages.push(tool_call_msg);
                }

                // Execute tools
                let tool_results = self.execute_tools(session_id, &tool_calls).await?;

                // Store tool result messages and add to conversation
                for (tool_call, result) in tool_calls.iter().zip(tool_results.iter()) {
                    let result_msg = ConversationMessage::tool_result(
                        &tool_call.id,
                        result.result.clone(),
                        result.error.clone(),
                    );
                    self.message_store
                        .store(session_id, result_msg.clone())
                        .await?;
                    messages.push(result_msg);
                }

                // Emit iteration completed (continue loop)
                self.event_emitter
                    .emit(LoopEvent::iteration_completed(
                        session_id.to_string(),
                        iteration,
                        true,
                    ))
                    .await?;

                // Continue loop with tool results
                continue;
            }

            // No tool calls, loop is complete
            self.event_emitter
                .emit(LoopEvent::iteration_completed(
                    session_id.to_string(),
                    iteration,
                    false,
                ))
                .await?;

            break;
        }

        // Emit loop completed
        self.event_emitter
            .emit(LoopEvent::loop_completed(session_id.to_string(), iteration))
            .await?;

        // Emit AG-UI session finished event
        self.event_emitter
            .emit(LoopEvent::ag_ui(AgUiEvent::session_finished(
                session_id.to_string(),
            )))
            .await?;

        info!(
            session_id = %session_id,
            iterations = iteration,
            "Agent loop completed"
        );

        Ok(LoopResult {
            session_id,
            messages,
            iterations: iteration,
            final_response,
        })
    }

    /// Execute a single step (for decomposed execution)
    ///
    /// This allows the loop to be broken into discrete steps that can be
    /// executed independently (e.g., as Temporal activities).
    pub async fn execute_step(&self, input: StepInput) -> Result<StepOutput> {
        let session_id = input.session_id;

        // If we have pending tool calls, execute them
        if !input.pending_tool_calls.is_empty() {
            let step = LoopStep::tool_execution(session_id, input.iteration);

            let tool_results = self
                .execute_tools(session_id, &input.pending_tool_calls)
                .await?;

            // Create result messages
            let mut messages = input.messages;
            for (tool_call, result) in input.pending_tool_calls.iter().zip(tool_results.iter()) {
                let result_msg = ConversationMessage::tool_result(
                    &tool_call.id,
                    result.result.clone(),
                    result.error.clone(),
                );
                messages.push(result_msg);
            }

            let step = step.complete(StepResult::ToolExecutionComplete {
                results: tool_results,
            });

            // Continue loop - need to call LLM again
            return Ok(StepOutput::continue_with(step, messages, Vec::new()));
        }

        // Otherwise, call LLM
        let step = LoopStep::llm_call(session_id, input.iteration);

        let llm_result = self
            .call_llm(session_id, input.iteration, &input.messages)
            .await?;

        let mut messages = input.messages;

        let has_tool_calls = llm_result
            .tool_calls
            .as_ref()
            .is_some_and(|tc| !tc.is_empty());

        // Add assistant response (even with empty text if there are tool_calls)
        if !llm_result.text.is_empty() || has_tool_calls {
            let assistant_msg = if let Some(ref tool_calls) = llm_result.tool_calls {
                ConversationMessage::assistant_with_tools(&llm_result.text, tool_calls.clone())
            } else {
                ConversationMessage::assistant(&llm_result.text)
            };
            messages.push(assistant_msg);
        }

        let step = step.complete(StepResult::LlmCallComplete {
            response_text: llm_result.text,
            tool_calls: llm_result.tool_calls.clone().unwrap_or_default(),
            continue_loop: has_tool_calls,
        });

        if has_tool_calls {
            // Add tool call messages
            let tool_calls = llm_result.tool_calls.unwrap();
            for tool_call in &tool_calls {
                messages.push(ConversationMessage::tool_call(tool_call));
            }

            Ok(StepOutput::continue_with(step, messages, tool_calls))
        } else {
            Ok(StepOutput::complete(step, messages))
        }
    }

    /// Run a single turn (user message → assistant response)
    ///
    /// Convenience method that adds a user message and runs until completion.
    pub async fn run_turn(
        &self,
        session_id: Uuid,
        user_message: impl Into<String>,
    ) -> Result<LoopResult> {
        // Store user message
        let user_msg = ConversationMessage::user(user_message);
        self.message_store.store(session_id, user_msg).await?;

        // Run the loop
        self.run(session_id).await
    }

    // =========================================================================
    // Private methods
    // =========================================================================

    /// Call LLM with streaming and event emission
    async fn call_llm(
        &self,
        session_id: Uuid,
        iteration: usize,
        messages: &[ConversationMessage],
    ) -> Result<LlmCallResult> {
        // Emit LLM call started
        self.event_emitter
            .emit(LoopEvent::llm_call_started(
                session_id.to_string(),
                iteration,
            ))
            .await?;

        // Build LLM messages
        let mut llm_messages = Vec::new();

        // Add system prompt if configured
        if !self.config.system_prompt.is_empty() {
            llm_messages.push(LlmMessage {
                role: LlmMessageRole::System,
                content: self.config.system_prompt.clone(),
                tool_calls: None,
                tool_call_id: None,
            });
        }

        // Add conversation messages
        for msg in messages {
            // Skip tool_call messages as they're represented in assistant messages
            if msg.role == MessageRole::ToolCall {
                continue;
            }
            llm_messages.push(msg.into());
        }

        // Build LLM config
        let llm_config = LlmCallConfig::from(&self.config);

        // Call LLM with streaming
        let mut stream = self
            .llm_provider
            .chat_completion_stream(llm_messages, &llm_config)
            .await?;

        // Emit text message start
        let message_id = Uuid::now_v7().to_string();
        self.event_emitter
            .emit(LoopEvent::ag_ui(AgUiEvent::text_message_start(
                &message_id,
                AgUiMessageRole::Assistant,
            )))
            .await?;

        // Process stream
        let mut text = String::new();
        let mut tool_calls = None;

        while let Some(event) = stream.next().await {
            match event? {
                LlmStreamEvent::TextDelta(delta) => {
                    if !delta.is_empty() {
                        text.push_str(&delta);

                        // Emit text delta
                        self.event_emitter
                            .emit(LoopEvent::text_delta(
                                session_id.to_string(),
                                &message_id,
                                &delta,
                            ))
                            .await?;

                        // Emit AG-UI text content
                        self.event_emitter
                            .emit(LoopEvent::ag_ui(AgUiEvent::text_message_content(
                                &message_id,
                                &delta,
                            )))
                            .await?;
                    }
                }
                LlmStreamEvent::ToolCalls(calls) => {
                    tool_calls = Some(calls);
                }
                LlmStreamEvent::Done(_metadata) => {
                    // Emit text message end
                    self.event_emitter
                        .emit(LoopEvent::ag_ui(AgUiEvent::text_message_end(&message_id)))
                        .await?;
                    break;
                }
                LlmStreamEvent::Error(err) => {
                    error!(session_id = %session_id, error = %err, "LLM stream error");
                    return Err(AgentLoopError::llm(err));
                }
            }
        }

        Ok(LlmCallResult { text, tool_calls })
    }

    /// Execute tool calls with event emission
    async fn execute_tools(
        &self,
        session_id: Uuid,
        tool_calls: &[everruns_contracts::tools::ToolCall],
    ) -> Result<Vec<everruns_contracts::tools::ToolResult>> {
        let mut results = Vec::with_capacity(tool_calls.len());

        for tool_call in tool_calls {
            // Emit tool started
            self.event_emitter
                .emit(LoopEvent::tool_started(
                    session_id.to_string(),
                    &tool_call.id,
                    &tool_call.name,
                ))
                .await?;

            // Emit AG-UI tool call events
            self.event_emitter
                .emit(LoopEvent::ag_ui(AgUiEvent::tool_call_start(
                    &tool_call.id,
                    &tool_call.name,
                )))
                .await?;

            let args_json = serde_json::to_string(&tool_call.arguments).unwrap_or_default();
            self.event_emitter
                .emit(LoopEvent::ag_ui(AgUiEvent::tool_call_args(
                    &tool_call.id,
                    args_json,
                )))
                .await?;

            self.event_emitter
                .emit(LoopEvent::ag_ui(AgUiEvent::tool_call_end(&tool_call.id)))
                .await?;

            // Find tool definition
            let tool_def = self
                .config
                .tools
                .iter()
                .find(|def| {
                    let name = match def {
                        everruns_contracts::tools::ToolDefinition::Webhook(w) => &w.name,
                        everruns_contracts::tools::ToolDefinition::Builtin(b) => &b.name,
                    };
                    name == &tool_call.name
                })
                .ok_or_else(|| {
                    AgentLoopError::tool(format!("Tool not found: {}", tool_call.name))
                })?;

            // Execute tool
            let result = self.tool_executor.execute(tool_call, tool_def).await?;
            let success = result.error.is_none();

            // Emit AG-UI tool result
            let result_message_id = Uuid::now_v7().to_string();
            self.event_emitter
                .emit(LoopEvent::ag_ui(AgUiEvent::tool_call_result(
                    &result_message_id,
                    &tool_call.id,
                    result.result.clone().unwrap_or(serde_json::Value::Null),
                )))
                .await?;

            // Emit tool completed
            self.event_emitter
                .emit(LoopEvent::tool_completed(
                    session_id.to_string(),
                    &tool_call.id,
                    success,
                ))
                .await?;

            results.push(result);
        }

        Ok(results)
    }
}

/// Result from calling the LLM
struct LlmCallResult {
    text: String,
    tool_calls: Option<Vec<everruns_contracts::tools::ToolCall>>,
}

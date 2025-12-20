// In-memory implementations for examples and testing
//
// These implementations keep all data in memory, making them perfect for:
// - Standalone examples that don't need a database
// - Unit tests
// - Quick prototyping

use async_trait::async_trait;
use everruns_contracts::tools::{ToolCall, ToolDefinition, ToolResult};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::error::Result;
use crate::events::LoopEvent;
use crate::message::ConversationMessage;
use crate::traits::{EventEmitter, MessageStore, ToolExecutor};

// ============================================================================
// InMemoryEventEmitter - Collects events in memory
// ============================================================================

/// In-memory event emitter that collects all events
///
/// Useful for testing and examples where you want to inspect events after execution.
#[derive(Debug, Default)]
pub struct InMemoryEventEmitter {
    events: Arc<RwLock<Vec<LoopEvent>>>,
}

impl InMemoryEventEmitter {
    /// Create a new in-memory event emitter
    pub fn new() -> Self {
        Self {
            events: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Get all collected events
    pub async fn events(&self) -> Vec<LoopEvent> {
        self.events.read().await.clone()
    }

    /// Clear all events
    pub async fn clear(&self) {
        self.events.write().await.clear();
    }

    /// Get event count
    pub async fn count(&self) -> usize {
        self.events.read().await.len()
    }
}

#[async_trait]
impl EventEmitter for InMemoryEventEmitter {
    async fn emit(&self, event: LoopEvent) -> Result<()> {
        self.events.write().await.push(event);
        Ok(())
    }
}

// ============================================================================
// ChannelEventEmitter - Sends events to a channel
// ============================================================================

/// Event emitter that sends events to a tokio broadcast channel
///
/// Useful for real-time streaming to multiple subscribers.
pub struct ChannelEventEmitter {
    sender: tokio::sync::broadcast::Sender<LoopEvent>,
}

impl ChannelEventEmitter {
    /// Create a new channel event emitter with the given capacity
    pub fn new(capacity: usize) -> (Self, tokio::sync::broadcast::Receiver<LoopEvent>) {
        let (sender, receiver) = tokio::sync::broadcast::channel(capacity);
        (Self { sender }, receiver)
    }

    /// Subscribe to events
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<LoopEvent> {
        self.sender.subscribe()
    }
}

#[async_trait]
impl EventEmitter for ChannelEventEmitter {
    async fn emit(&self, event: LoopEvent) -> Result<()> {
        // Ignore send errors (no receivers)
        let _ = self.sender.send(event);
        Ok(())
    }
}

// ============================================================================
// NoOpEventEmitter - Discards all events
// ============================================================================

/// Event emitter that discards all events
///
/// Useful when you don't need event streaming.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoOpEventEmitter;

impl NoOpEventEmitter {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl EventEmitter for NoOpEventEmitter {
    async fn emit(&self, _event: LoopEvent) -> Result<()> {
        Ok(())
    }
}

// ============================================================================
// InMemoryMessageStore - Stores messages in memory
// ============================================================================

/// In-memory message store
///
/// Stores messages in a HashMap keyed by session ID.
#[derive(Debug, Default, Clone)]
pub struct InMemoryMessageStore {
    messages: Arc<RwLock<HashMap<Uuid, Vec<ConversationMessage>>>>,
}

impl InMemoryMessageStore {
    /// Create a new in-memory message store
    pub fn new() -> Self {
        Self {
            messages: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Get all sessions
    pub async fn sessions(&self) -> Vec<Uuid> {
        self.messages.read().await.keys().copied().collect()
    }

    /// Clear all messages
    pub async fn clear(&self) {
        self.messages.write().await.clear();
    }

    /// Clear messages for a specific session
    pub async fn clear_session(&self, session_id: Uuid) {
        self.messages.write().await.remove(&session_id);
    }

    /// Pre-populate with messages (useful for testing)
    pub async fn seed(&self, session_id: Uuid, messages: Vec<ConversationMessage>) {
        self.messages.write().await.insert(session_id, messages);
    }
}

#[async_trait]
impl MessageStore for InMemoryMessageStore {
    async fn store(&self, session_id: Uuid, message: ConversationMessage) -> Result<()> {
        self.messages
            .write()
            .await
            .entry(session_id)
            .or_default()
            .push(message);
        Ok(())
    }

    async fn load(&self, session_id: Uuid) -> Result<Vec<ConversationMessage>> {
        Ok(self
            .messages
            .read()
            .await
            .get(&session_id)
            .cloned()
            .unwrap_or_default())
    }

    async fn count(&self, session_id: Uuid) -> Result<usize> {
        Ok(self
            .messages
            .read()
            .await
            .get(&session_id)
            .map(|m| m.len())
            .unwrap_or(0))
    }
}

// ============================================================================
// MockToolExecutor - Returns predefined results
// ============================================================================

/// Mock tool executor for testing
///
/// Returns predefined results based on tool name.
#[derive(Debug, Default)]
pub struct MockToolExecutor {
    results: Arc<RwLock<HashMap<String, serde_json::Value>>>,
    call_log: Arc<RwLock<Vec<ToolCall>>>,
}

impl MockToolExecutor {
    /// Create a new mock tool executor
    pub fn new() -> Self {
        Self {
            results: Arc::new(RwLock::new(HashMap::new())),
            call_log: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Set the result for a specific tool
    pub async fn set_result(&self, tool_name: impl Into<String>, result: serde_json::Value) {
        self.results.write().await.insert(tool_name.into(), result);
    }

    /// Get the call log
    pub async fn calls(&self) -> Vec<ToolCall> {
        self.call_log.read().await.clone()
    }

    /// Clear the call log
    pub async fn clear_calls(&self) {
        self.call_log.write().await.clear();
    }
}

#[async_trait]
impl ToolExecutor for MockToolExecutor {
    async fn execute(
        &self,
        tool_call: &ToolCall,
        _tool_def: &ToolDefinition,
    ) -> Result<ToolResult> {
        // Log the call
        self.call_log.write().await.push(tool_call.clone());

        // Return predefined result or default
        let result = self
            .results
            .read()
            .await
            .get(&tool_call.name)
            .cloned()
            .unwrap_or_else(|| serde_json::json!({"status": "ok"}));

        Ok(ToolResult {
            tool_call_id: tool_call.id.clone(),
            result: Some(result),
            error: None,
        })
    }
}

// ============================================================================
// EchoToolExecutor - Echoes back the arguments
// ============================================================================

/// Tool executor that echoes back the arguments
///
/// Useful for simple testing without setting up mock results.
#[derive(Debug, Default, Clone, Copy)]
pub struct EchoToolExecutor;

impl EchoToolExecutor {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl ToolExecutor for EchoToolExecutor {
    async fn execute(
        &self,
        tool_call: &ToolCall,
        _tool_def: &ToolDefinition,
    ) -> Result<ToolResult> {
        Ok(ToolResult {
            tool_call_id: tool_call.id.clone(),
            result: Some(serde_json::json!({
                "echoed_tool": tool_call.name,
                "echoed_arguments": tool_call.arguments
            })),
            error: None,
        })
    }
}

// ============================================================================
// FailingToolExecutor - Always returns an error
// ============================================================================

/// Tool executor that always fails
///
/// Useful for testing error handling.
#[derive(Debug, Clone)]
pub struct FailingToolExecutor {
    error_message: String,
}

impl FailingToolExecutor {
    pub fn new(error_message: impl Into<String>) -> Self {
        Self {
            error_message: error_message.into(),
        }
    }
}

impl Default for FailingToolExecutor {
    fn default() -> Self {
        Self::new("Tool execution failed")
    }
}

#[async_trait]
impl ToolExecutor for FailingToolExecutor {
    async fn execute(
        &self,
        tool_call: &ToolCall,
        _tool_def: &ToolDefinition,
    ) -> Result<ToolResult> {
        Ok(ToolResult {
            tool_call_id: tool_call.id.clone(),
            result: None,
            error: Some(self.error_message.clone()),
        })
    }
}

// ============================================================================
// MockLlmProvider - Returns predefined responses
// ============================================================================

use crate::llm::{
    LlmCallConfig, LlmCompletionMetadata, LlmMessage, LlmProvider, LlmResponseStream,
    LlmStreamEvent,
};
use futures::stream;

/// Mock LLM provider for testing
///
/// Returns predefined responses in sequence.
#[derive(Debug, Default)]
pub struct MockLlmProvider {
    responses: Arc<RwLock<Vec<MockLlmResponse>>>,
    call_index: Arc<RwLock<usize>>,
    call_log: Arc<RwLock<Vec<Vec<LlmMessage>>>>,
}

/// A mock LLM response
#[derive(Debug, Clone)]
pub struct MockLlmResponse {
    pub text: String,
    pub tool_calls: Option<Vec<ToolCall>>,
}

impl MockLlmResponse {
    /// Create a text-only response
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            tool_calls: None,
        }
    }

    /// Create a response with tool calls
    pub fn with_tools(text: impl Into<String>, tool_calls: Vec<ToolCall>) -> Self {
        Self {
            text: text.into(),
            tool_calls: Some(tool_calls),
        }
    }
}

impl MockLlmProvider {
    /// Create a new mock LLM provider
    pub fn new() -> Self {
        Self {
            responses: Arc::new(RwLock::new(Vec::new())),
            call_index: Arc::new(RwLock::new(0)),
            call_log: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Add a response to the queue
    pub async fn add_response(&self, response: MockLlmResponse) {
        self.responses.write().await.push(response);
    }

    /// Set all responses at once
    pub async fn set_responses(&self, responses: Vec<MockLlmResponse>) {
        *self.responses.write().await = responses;
        *self.call_index.write().await = 0;
    }

    /// Get the call log
    pub async fn calls(&self) -> Vec<Vec<LlmMessage>> {
        self.call_log.read().await.clone()
    }

    /// Reset the provider
    pub async fn reset(&self) {
        self.responses.write().await.clear();
        *self.call_index.write().await = 0;
        self.call_log.write().await.clear();
    }
}

#[async_trait]
impl LlmProvider for MockLlmProvider {
    async fn chat_completion_stream(
        &self,
        messages: Vec<LlmMessage>,
        _config: &LlmCallConfig,
    ) -> Result<LlmResponseStream> {
        // Log the call
        self.call_log.write().await.push(messages);

        // Get next response
        let mut index = self.call_index.write().await;
        let responses = self.responses.read().await;

        let response = responses.get(*index).cloned().unwrap_or_else(|| {
            MockLlmResponse::text("Mock response (no more responses configured)")
        });

        *index += 1;
        drop(index);
        drop(responses);

        // Create a stream that emits the response
        let events = vec![
            Ok(LlmStreamEvent::TextDelta(response.text.clone())),
            if let Some(tool_calls) = response.tool_calls {
                Ok(LlmStreamEvent::ToolCalls(tool_calls))
            } else {
                Ok(LlmStreamEvent::Done(LlmCompletionMetadata::default()))
            },
            Ok(LlmStreamEvent::Done(LlmCompletionMetadata::default())),
        ];

        Ok(Box::pin(stream::iter(events)))
    }
}

// ============================================================================
// Builder for easy setup
// ============================================================================

use crate::config::AgentConfig;
use crate::executor::AgentLoop;

/// Builder for creating an AgentLoop with in-memory components
pub struct InMemoryAgentLoopBuilder {
    config: AgentConfig,
    event_emitter: Option<InMemoryEventEmitter>,
    message_store: Option<InMemoryMessageStore>,
    llm_provider: Option<MockLlmProvider>,
    tool_executor: Option<MockToolExecutor>,
}

impl InMemoryAgentLoopBuilder {
    /// Create a new builder with the given config
    pub fn new(config: AgentConfig) -> Self {
        Self {
            config,
            event_emitter: None,
            message_store: None,
            llm_provider: None,
            tool_executor: None,
        }
    }

    /// Use a custom event emitter
    pub fn event_emitter(mut self, emitter: InMemoryEventEmitter) -> Self {
        self.event_emitter = Some(emitter);
        self
    }

    /// Use a custom message store
    pub fn message_store(mut self, store: InMemoryMessageStore) -> Self {
        self.message_store = Some(store);
        self
    }

    /// Use a custom LLM provider
    pub fn llm_provider(mut self, provider: MockLlmProvider) -> Self {
        self.llm_provider = Some(provider);
        self
    }

    /// Use a custom tool executor
    pub fn tool_executor(mut self, executor: MockToolExecutor) -> Self {
        self.tool_executor = Some(executor);
        self
    }

    /// Build the agent loop
    pub fn build(
        self,
    ) -> AgentLoop<InMemoryEventEmitter, InMemoryMessageStore, MockLlmProvider, MockToolExecutor>
    {
        AgentLoop::new(
            self.config,
            self.event_emitter.unwrap_or_default(),
            self.message_store.unwrap_or_default(),
            self.llm_provider.unwrap_or_default(),
            self.tool_executor.unwrap_or_default(),
        )
    }

    /// Build and return references to components for inspection
    #[allow(clippy::type_complexity)]
    pub fn build_with_refs(
        self,
    ) -> (
        AgentLoop<InMemoryEventEmitter, InMemoryMessageStore, MockLlmProvider, MockToolExecutor>,
        Arc<InMemoryEventEmitter>,
        Arc<InMemoryMessageStore>,
        Arc<MockLlmProvider>,
        Arc<MockToolExecutor>,
    ) {
        let event_emitter = Arc::new(self.event_emitter.unwrap_or_default());
        let message_store = Arc::new(self.message_store.unwrap_or_default());
        let llm_provider = Arc::new(self.llm_provider.unwrap_or_default());
        let tool_executor = Arc::new(self.tool_executor.unwrap_or_default());

        let loop_instance = AgentLoop::with_arcs(
            self.config,
            event_emitter.clone(),
            message_store.clone(),
            llm_provider.clone(),
            tool_executor.clone(),
        );

        (
            loop_instance,
            event_emitter,
            message_store,
            llm_provider,
            tool_executor,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_in_memory_event_emitter() {
        let emitter = InMemoryEventEmitter::new();

        emitter
            .emit(LoopEvent::loop_started("test-session"))
            .await
            .unwrap();

        assert_eq!(emitter.count().await, 1);

        let events = emitter.events().await;
        assert!(matches!(events[0], LoopEvent::LoopStarted { .. }));
    }

    #[tokio::test]
    async fn test_in_memory_message_store() {
        let store = InMemoryMessageStore::new();
        let session_id = Uuid::now_v7();

        store
            .store(session_id, ConversationMessage::user("Hello"))
            .await
            .unwrap();

        let messages = store.load(session_id).await.unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].text(), Some("Hello"));
    }

    #[tokio::test]
    async fn test_mock_tool_executor() {
        let executor = MockToolExecutor::new();
        executor
            .set_result("get_weather", serde_json::json!({"temp": 72}))
            .await;

        let tool_call = ToolCall {
            id: "call_1".to_string(),
            name: "get_weather".to_string(),
            arguments: serde_json::json!({"city": "NYC"}),
        };

        let tool_def = ToolDefinition::Builtin(everruns_contracts::tools::BuiltinTool {
            name: "get_weather".to_string(),
            description: "Get weather".to_string(),
            parameters: serde_json::json!({}),
            kind: everruns_contracts::tools::BuiltinToolKind::HttpGet,
            policy: everruns_contracts::tools::ToolPolicy::Auto,
        });

        let result = executor.execute(&tool_call, &tool_def).await.unwrap();

        assert!(result.error.is_none());
        assert_eq!(result.result, Some(serde_json::json!({"temp": 72})));
    }
}

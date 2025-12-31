// In-memory implementations for examples and testing
//
// These implementations keep all data in memory, making them perfect for:
// - Standalone examples that don't need a database
// - Unit tests
// - Quick prototyping

use crate::agent::Agent;
use crate::llm_models::LlmProviderType;
use crate::session::Session;
use crate::tool_types::{ToolCall, ToolDefinition, ToolResult};
use crate::traits::ModelWithProvider;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::error::Result;
use crate::message::Message;
use crate::traits::{AgentStore, LlmProviderStore, MessageStore, SessionStore, ToolExecutor};

// ============================================================================
// InMemoryMessageStore - Stores messages in memory
// ============================================================================

/// In-memory message store
///
/// Stores messages in a HashMap keyed by session ID.
#[derive(Debug, Default, Clone)]
pub struct InMemoryMessageStore {
    messages: Arc<RwLock<HashMap<Uuid, Vec<Message>>>>,
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
    pub async fn seed(&self, session_id: Uuid, messages: Vec<Message>) {
        self.messages.write().await.insert(session_id, messages);
    }
}

#[async_trait]
impl MessageStore for InMemoryMessageStore {
    async fn store(&self, session_id: Uuid, message: Message) -> Result<()> {
        self.messages
            .write()
            .await
            .entry(session_id)
            .or_default()
            .push(message);
        Ok(())
    }

    async fn load(&self, session_id: Uuid) -> Result<Vec<Message>> {
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
// InMemoryAgentStore - Stores agents in memory
// ============================================================================

/// In-memory agent store
///
/// Stores agents in a HashMap keyed by agent ID.
/// Useful for testing and examples where you want to configure agents without a database.
#[derive(Debug, Default, Clone)]
pub struct InMemoryAgentStore {
    agents: Arc<RwLock<HashMap<Uuid, Agent>>>,
}

impl InMemoryAgentStore {
    /// Create a new in-memory agent store
    pub fn new() -> Self {
        Self {
            agents: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Add an agent to the store
    pub async fn add_agent(&self, agent: Agent) {
        self.agents.write().await.insert(agent.id, agent);
    }

    /// Get all agent IDs
    pub async fn agent_ids(&self) -> Vec<Uuid> {
        self.agents.read().await.keys().copied().collect()
    }

    /// Clear all agents
    pub async fn clear(&self) {
        self.agents.write().await.clear();
    }
}

#[async_trait]
impl AgentStore for InMemoryAgentStore {
    async fn get_agent(&self, agent_id: Uuid) -> Result<Option<Agent>> {
        Ok(self.agents.read().await.get(&agent_id).cloned())
    }
}

// ============================================================================
// InMemorySessionStore - Stores sessions in memory
// ============================================================================

/// In-memory session store
///
/// Stores sessions in a HashMap keyed by session ID.
/// Useful for testing and examples where you want to configure sessions without a database.
#[derive(Debug, Default, Clone)]
pub struct InMemorySessionStore {
    sessions: Arc<RwLock<HashMap<Uuid, Session>>>,
}

impl InMemorySessionStore {
    /// Create a new in-memory session store
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Add a session to the store
    pub async fn add_session(&self, session: Session) {
        self.sessions.write().await.insert(session.id, session);
    }

    /// Get all session IDs
    pub async fn session_ids(&self) -> Vec<Uuid> {
        self.sessions.read().await.keys().copied().collect()
    }

    /// Clear all sessions
    pub async fn clear(&self) {
        self.sessions.write().await.clear();
    }
}

#[async_trait]
impl SessionStore for InMemorySessionStore {
    async fn get_session(&self, session_id: Uuid) -> Result<Option<Session>> {
        Ok(self.sessions.read().await.get(&session_id).cloned())
    }
}

// ============================================================================
// InMemoryLlmProviderStore - Stores LLM provider configurations in memory
// ============================================================================

/// In-memory LLM provider store
///
/// Stores model configurations in a HashMap keyed by model UUID.
/// Useful for testing and examples where you want to configure providers without a database.
///
/// # Example
///
/// ```ignore
/// use everruns_core::memory::InMemoryLlmProviderStore;
/// use everruns_core::llm_entities::LlmProviderType;
///
/// let store = InMemoryLlmProviderStore::from_env().await;
/// // Uses OPENAI_API_KEY or ANTHROPIC_API_KEY from environment
/// ```
#[derive(Debug, Default, Clone)]
pub struct InMemoryLlmProviderStore {
    models: Arc<RwLock<HashMap<Uuid, ModelWithProvider>>>,
    default_model: Arc<RwLock<Option<ModelWithProvider>>>,
}

impl InMemoryLlmProviderStore {
    /// Create a new empty in-memory provider store
    pub fn new() -> Self {
        Self {
            models: Arc::new(RwLock::new(HashMap::new())),
            default_model: Arc::new(RwLock::new(None)),
        }
    }

    /// Create a provider store from environment variables
    ///
    /// Checks for OPENAI_API_KEY or ANTHROPIC_API_KEY and configures
    /// a default model accordingly.
    pub async fn from_env() -> Self {
        let store = Self::new();

        // Check for OpenAI first
        if let Ok(api_key) = std::env::var("OPENAI_API_KEY") {
            let model = ModelWithProvider {
                model: "gpt-4o".to_string(),
                provider_type: LlmProviderType::Openai,
                api_key: Some(api_key),
                base_url: std::env::var("OPENAI_BASE_URL").ok(),
            };
            store.set_default_model(model).await;
        } else if let Ok(api_key) = std::env::var("ANTHROPIC_API_KEY") {
            let model = ModelWithProvider {
                model: "claude-sonnet-4-20250514".to_string(),
                provider_type: LlmProviderType::Anthropic,
                api_key: Some(api_key),
                base_url: std::env::var("ANTHROPIC_BASE_URL").ok(),
            };
            store.set_default_model(model).await;
        }

        store
    }

    /// Create a provider store with a specific default model
    pub async fn with_default(model: ModelWithProvider) -> Self {
        let store = Self::new();
        store.set_default_model(model).await;
        store
    }

    /// Add a model to the store
    pub async fn add_model(&self, model_uuid: Uuid, model: ModelWithProvider) {
        self.models.write().await.insert(model_uuid, model);
    }

    /// Set the default model
    pub async fn set_default_model(&self, model: ModelWithProvider) {
        *self.default_model.write().await = Some(model);
    }

    /// Clear all models
    pub async fn clear(&self) {
        self.models.write().await.clear();
        *self.default_model.write().await = None;
    }
}

#[async_trait]
impl LlmProviderStore for InMemoryLlmProviderStore {
    async fn get_model_with_provider(&self, model_id: Uuid) -> Result<Option<ModelWithProvider>> {
        Ok(self.models.read().await.get(&model_id).cloned())
    }

    async fn get_default_model(&self) -> Result<Option<ModelWithProvider>> {
        Ok(self.default_model.read().await.clone())
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

use crate::llm_driver_registry::{
    LlmCallConfig, LlmCompletionMetadata, LlmDriver, LlmMessage, LlmResponseStream, LlmStreamEvent,
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
impl LlmDriver for MockLlmProvider {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_in_memory_message_store() {
        let store = InMemoryMessageStore::new();
        let session_id = Uuid::now_v7();

        store
            .store(session_id, Message::user("Hello"))
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

        let tool_def = ToolDefinition::Builtin(crate::tool_types::BuiltinTool {
            name: "get_weather".to_string(),
            description: "Get weather".to_string(),
            parameters: serde_json::json!({}),
            policy: crate::tool_types::ToolPolicy::Auto,
        });

        let result = executor.execute(&tool_call, &tool_def).await.unwrap();

        assert!(result.error.is_none());
        assert_eq!(result.result, Some(serde_json::json!({"temp": 72})));
    }
}

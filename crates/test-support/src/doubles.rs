// Test doubles for the core execution traits.
//
// These are deliberately simple stand-ins used by unit and integration
// tests: a scriptable tool executor, echo/failing executors, and a mock
// chat driver that replays queued responses.

use async_trait::async_trait;
use everruns_core::driver_registry::{
    ChatDriver, LlmCallConfig, LlmMessage, LlmResponseStream, LlmStreamEvent,
};
use everruns_core::error::Result;
use everruns_core::tool_execution::ToolExecutor;
use everruns_core::tool_types::{ToolCall, ToolDefinition, ToolResult};
use futures::stream;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

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
            images: None,
            error: None,
            connection_required: None,
            raw_output: None,
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
            images: None,
            error: None,
            connection_required: None,
            raw_output: None,
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
            images: None,
            error: Some(self.error_message.clone()),
            connection_required: None,
            raw_output: None,
        })
    }
}

// ============================================================================
// MockProvider - Returns predefined responses
// ============================================================================

/// Mock LLM provider for testing
///
/// Returns predefined responses in sequence.
#[derive(Debug, Default)]
pub struct MockProvider {
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

impl MockProvider {
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
impl ChatDriver for MockProvider {
    async fn chat_completion_stream(
        &self,
        _endpoint: &everruns_core::ProviderEndpoint,
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
                Ok(LlmStreamEvent::Done(Box::default()))
            },
            Ok(LlmStreamEvent::Done(Box::default())),
        ];

        Ok(Box::pin(stream::iter(events)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

        let tool_def = ToolDefinition::Builtin(everruns_core::tool_types::BuiltinTool {
            name: "get_weather".to_string(),
            display_name: None,
            description: "Get weather".to_string(),
            parameters: serde_json::json!({}),
            policy: everruns_core::tool_types::ToolPolicy::Auto,
            category: None,
            deferrable: everruns_core::tool_types::DeferrablePolicy::default(),
            hints: everruns_core::tool_types::ToolHints::default(),
            full_parameters: None,
        });

        let result = executor.execute(&tool_call, &tool_def).await.unwrap();

        assert!(result.error.is_none());
        assert_eq!(result.result, Some(serde_json::json!({"temp": 72})));
    }
}

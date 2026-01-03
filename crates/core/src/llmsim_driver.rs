// LLM Simulator Driver
//
// This module provides a fake LLM driver for testing purposes using llmsim.
// It supports:
// - Configurable response generators (fixed, lorem, echo, sequence)
// - Optional tool call responses
// - Configurable latency simulation
// - Token counting
//
// Design: This driver is intended for unit and integration tests.
// It can be configured per-test to return specific responses or tool calls.
//
// Note: This module is only compiled when the "llmsim" feature is enabled.

use async_trait::async_trait;
use futures::stream;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use crate::error::Result;
use crate::llm_driver_registry::{
    BoxedLlmDriver, DriverRegistry, LlmCallConfig, LlmCompletionMetadata, LlmDriver, LlmMessage,
    LlmMessageRole, LlmResponseStream, LlmStreamEvent, ProviderType,
};
use crate::tool_types::ToolCall;
use llmsim::generator::{LoremGenerator, ResponseGenerator};
use llmsim::latency::LatencyProfile;
use llmsim::openai::{ChatCompletionRequest, Message, Role};

// ============================================================================
// Configuration Types
// ============================================================================

/// Configuration for the LlmSim driver
#[derive(Debug, Clone)]
pub struct LlmSimConfig {
    /// Response generation configuration
    pub response: ResponseConfig,
    /// Optional tool calls to include in responses
    pub tool_calls: Option<ToolCallConfig>,
    /// Enable latency simulation (default: false for fast tests)
    pub simulate_latency: bool,
    /// Model name to report in metadata
    pub model_name: String,
}

impl Default for LlmSimConfig {
    fn default() -> Self {
        Self {
            response: ResponseConfig::Fixed("Hello! I'm a simulated LLM response.".to_string()),
            tool_calls: None,
            simulate_latency: false,
            model_name: "llmsim-model".to_string(),
        }
    }
}

impl LlmSimConfig {
    /// Create a new config with a fixed response
    pub fn fixed(response: impl Into<String>) -> Self {
        Self {
            response: ResponseConfig::Fixed(response.into()),
            ..Default::default()
        }
    }

    /// Create a new config that echoes user input
    pub fn echo() -> Self {
        Self {
            response: ResponseConfig::Echo,
            ..Default::default()
        }
    }

    /// Create a new config with lorem ipsum text
    pub fn lorem(target_tokens: usize) -> Self {
        Self {
            response: ResponseConfig::Lorem { target_tokens },
            ..Default::default()
        }
    }

    /// Create a new config with a sequence of responses
    pub fn sequence(responses: Vec<String>) -> Self {
        Self {
            response: ResponseConfig::Sequence(responses),
            ..Default::default()
        }
    }

    /// Add tool calls to the response
    pub fn with_tool_calls(mut self, tool_calls: Vec<ToolCall>) -> Self {
        self.tool_calls = Some(ToolCallConfig::Fixed(tool_calls));
        self
    }

    /// Add a sequence of tool calls (different per call)
    pub fn with_tool_call_sequence(mut self, sequences: Vec<Vec<ToolCall>>) -> Self {
        self.tool_calls = Some(ToolCallConfig::Sequence(sequences));
        self
    }

    /// Enable latency simulation
    pub fn with_latency(mut self) -> Self {
        self.simulate_latency = true;
        self
    }

    /// Set model name for metadata
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model_name = model.into();
        self
    }
}

/// Response generation configuration
#[derive(Debug, Clone)]
pub enum ResponseConfig {
    /// Return a fixed response
    Fixed(String),
    /// Echo back the last user message with a prefix
    Echo,
    /// Generate lorem ipsum text with target token count
    Lorem { target_tokens: usize },
    /// Return responses from a sequence (cycles when exhausted)
    Sequence(Vec<String>),
    /// Empty response (useful for tool-only responses)
    Empty,
}

/// Tool call configuration
#[derive(Debug, Clone)]
pub enum ToolCallConfig {
    /// Always return these tool calls
    Fixed(Vec<ToolCall>),
    /// Return tool calls from a sequence (cycles when exhausted)
    Sequence(Vec<Vec<ToolCall>>),
    /// Conditionally return tool calls based on message content
    Conditional {
        /// Patterns to match against user message
        patterns: Vec<ToolCallPattern>,
    },
}

/// Pattern for conditional tool calls
#[derive(Debug, Clone)]
pub struct ToolCallPattern {
    /// Substring to match in user message
    pub contains: String,
    /// Tool calls to return when pattern matches
    pub tool_calls: Vec<ToolCall>,
}

impl ToolCallPattern {
    pub fn new(contains: impl Into<String>, tool_calls: Vec<ToolCall>) -> Self {
        Self {
            contains: contains.into(),
            tool_calls,
        }
    }
}

// ============================================================================
// Driver Implementation
// ============================================================================

/// LLM Simulator Driver for testing
///
/// This driver generates simulated responses based on configuration.
/// It's intended for unit and integration tests where you need
/// deterministic or configurable LLM behavior.
///
/// # Example
///
/// ```ignore
/// use everruns_core::llmsim_driver::{LlmSimDriver, LlmSimConfig};
///
/// // Simple fixed response
/// let driver = LlmSimDriver::new(LlmSimConfig::fixed("Hello!"));
///
/// // With tool calls
/// let driver = LlmSimDriver::new(
///     LlmSimConfig::fixed("Let me check that for you.")
///         .with_tool_calls(vec![ToolCall { ... }])
/// );
///
/// // Sequence of responses for multi-turn tests
/// let driver = LlmSimDriver::new(
///     LlmSimConfig::sequence(vec![
///         "First response".to_string(),
///         "Second response".to_string(),
///     ])
/// );
/// ```
#[derive(Clone)]
pub struct LlmSimDriver {
    config: LlmSimConfig,
    /// Counter for sequence-based responses
    response_counter: Arc<AtomicUsize>,
    /// Counter for sequence-based tool calls
    tool_call_counter: Arc<AtomicUsize>,
}

impl LlmSimDriver {
    /// Create a new driver with the given configuration
    pub fn new(config: LlmSimConfig) -> Self {
        Self {
            config,
            response_counter: Arc::new(AtomicUsize::new(0)),
            tool_call_counter: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Create a driver with default configuration (fixed response)
    pub fn default_driver() -> Self {
        Self::new(LlmSimConfig::default())
    }

    /// Generate response text based on configuration
    fn generate_response(&self, messages: &[LlmMessage]) -> String {
        match &self.config.response {
            ResponseConfig::Fixed(text) => text.clone(),

            ResponseConfig::Echo => {
                // Find last user message and echo it
                let last_user = messages
                    .iter()
                    .rev()
                    .find(|m| m.role == LlmMessageRole::User)
                    .map(|m| m.content_as_text())
                    .unwrap_or_default();
                format!("Echo: {}", last_user)
            }

            ResponseConfig::Lorem { target_tokens } => {
                let generator = LoremGenerator::new(*target_tokens);
                let request = self.to_chat_request(messages);
                generator.generate(&request)
            }

            ResponseConfig::Sequence(responses) => {
                if responses.is_empty() {
                    return String::new();
                }
                let idx = self.response_counter.fetch_add(1, Ordering::SeqCst);
                responses[idx % responses.len()].clone()
            }

            ResponseConfig::Empty => String::new(),
        }
    }

    /// Get tool calls based on configuration
    fn get_tool_calls(&self, messages: &[LlmMessage]) -> Option<Vec<ToolCall>> {
        match &self.config.tool_calls {
            None => None,

            Some(ToolCallConfig::Fixed(calls)) => {
                if calls.is_empty() {
                    None
                } else {
                    Some(calls.clone())
                }
            }

            Some(ToolCallConfig::Sequence(sequences)) => {
                if sequences.is_empty() {
                    return None;
                }
                let idx = self.tool_call_counter.fetch_add(1, Ordering::SeqCst);
                let calls = &sequences[idx % sequences.len()];
                if calls.is_empty() {
                    None
                } else {
                    Some(calls.clone())
                }
            }

            Some(ToolCallConfig::Conditional { patterns }) => {
                // Find last user message
                let last_user = messages
                    .iter()
                    .rev()
                    .find(|m| m.role == LlmMessageRole::User)
                    .map(|m| m.content_as_text())
                    .unwrap_or_default();

                // Check patterns
                for pattern in patterns {
                    if last_user.contains(&pattern.contains) {
                        return if pattern.tool_calls.is_empty() {
                            None
                        } else {
                            Some(pattern.tool_calls.clone())
                        };
                    }
                }
                None
            }
        }
    }

    /// Convert LlmMessage to llmsim ChatCompletionRequest
    fn to_chat_request(&self, messages: &[LlmMessage]) -> ChatCompletionRequest {
        let sim_messages: Vec<Message> = messages
            .iter()
            .map(|m| {
                let role = match m.role {
                    LlmMessageRole::System => Role::System,
                    LlmMessageRole::User => Role::User,
                    LlmMessageRole::Assistant => Role::Assistant,
                    LlmMessageRole::Tool => Role::Tool,
                };
                Message {
                    role,
                    content: Some(m.content_as_text()),
                    name: None,
                    tool_calls: None,
                    tool_call_id: m.tool_call_id.clone(),
                }
            })
            .collect();

        ChatCompletionRequest {
            model: self.config.model_name.clone(),
            messages: sim_messages,
            temperature: None,
            top_p: None,
            n: None,
            max_tokens: None,
            max_completion_tokens: None,
            stream: true,
            stop: None,
            presence_penalty: None,
            frequency_penalty: None,
            logit_bias: None,
            user: None,
            tools: None,
            tool_choice: None,
            seed: None,
            response_format: None,
        }
    }

    /// Get latency profile if enabled
    fn get_latency_profile(&self) -> LatencyProfile {
        if self.config.simulate_latency {
            // Use fast profile for tests - just enough to simulate streaming
            LatencyProfile::fast()
        } else {
            LatencyProfile::instant()
        }
    }

    /// Estimate token count for text
    fn estimate_tokens(text: &str) -> u32 {
        // Simple estimation: ~4 chars per token
        (text.len() / 4).max(1) as u32
    }
}

#[async_trait]
impl LlmDriver for LlmSimDriver {
    async fn chat_completion_stream(
        &self,
        messages: Vec<LlmMessage>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponseStream> {
        let response_text = self.generate_response(&messages);
        let tool_calls = self.get_tool_calls(&messages);
        let latency_profile = self.get_latency_profile();
        let model_name = config.model.clone();

        // Calculate token estimates
        let prompt_tokens: u32 = messages
            .iter()
            .map(|m| Self::estimate_tokens(&m.content_as_text()))
            .sum();
        let completion_tokens = Self::estimate_tokens(&response_text);

        // Build stream events
        let mut events = Vec::new();

        // Simulate time-to-first-token if latency enabled
        if self.config.simulate_latency {
            let ttft = latency_profile.sample_ttft();
            tokio::time::sleep(ttft).await;
        }

        // Stream text in chunks (word by word for realistic streaming)
        if !response_text.is_empty() {
            let words: Vec<&str> = response_text.split_whitespace().collect();
            for (i, word) in words.iter().enumerate() {
                let delta = if i == 0 {
                    word.to_string()
                } else {
                    format!(" {}", word)
                };
                events.push(Ok(LlmStreamEvent::TextDelta(delta)));
            }
        }

        // Emit tool calls if present
        if let Some(calls) = tool_calls {
            events.push(Ok(LlmStreamEvent::ToolCalls(calls)));
        }

        // Final done event with metadata
        events.push(Ok(LlmStreamEvent::Done(LlmCompletionMetadata {
            total_tokens: Some(prompt_tokens + completion_tokens),
            prompt_tokens: Some(prompt_tokens),
            completion_tokens: Some(completion_tokens),
            model: Some(model_name),
            finish_reason: Some("stop".to_string()),
        })));

        Ok(Box::pin(stream::iter(events)))
    }
}

impl std::fmt::Debug for LlmSimDriver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LlmSimDriver")
            .field("model", &self.config.model_name)
            .field("simulate_latency", &self.config.simulate_latency)
            .finish()
    }
}

// ============================================================================
// Driver Registration
// ============================================================================

/// Register the LlmSim driver with the driver registry
///
/// This registers a driver for the `LlmSim` provider type.
/// The driver is created with a default configuration; for custom configs,
/// create the driver directly using `LlmSimDriver::new()`.
///
/// # Example
///
/// ```ignore
/// use everruns_core::DriverRegistry;
/// use everruns_core::llmsim_driver::register_driver;
///
/// let mut registry = DriverRegistry::new();
/// register_driver(&mut registry);
/// ```
pub fn register_driver(registry: &mut DriverRegistry) {
    registry.register(ProviderType::LlmSim, |_api_key, _base_url| {
        // Default driver - tests can create custom drivers directly
        Box::new(LlmSimDriver::default_driver()) as BoxedLlmDriver
    });
}

/// Create a LlmSim driver with custom configuration
///
/// This is the preferred way to create a driver in tests.
/// Unlike `register_driver`, this gives you full control over the config.
///
/// # Example
///
/// ```ignore
/// use everruns_core::llmsim_driver::{create_driver, LlmSimConfig};
///
/// let driver = create_driver(
///     LlmSimConfig::fixed("I'll help you with that!")
///         .with_tool_calls(vec![...])
/// );
/// ```
pub fn create_driver(config: LlmSimConfig) -> BoxedLlmDriver {
    Box::new(LlmSimDriver::new(config))
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    fn make_config() -> LlmCallConfig {
        LlmCallConfig {
            model: "test-model".to_string(),
            temperature: None,
            max_tokens: None,
            tools: vec![],
            reasoning_effort: None,
        }
    }

    fn user_message(content: &str) -> LlmMessage {
        LlmMessage::text(LlmMessageRole::User, content)
    }

    fn system_message(content: &str) -> LlmMessage {
        LlmMessage::text(LlmMessageRole::System, content)
    }

    #[tokio::test]
    async fn test_fixed_response() {
        let driver = LlmSimDriver::new(LlmSimConfig::fixed("Hello, world!"));
        let messages = vec![user_message("Hi there")];

        let response = driver
            .chat_completion(messages, &make_config())
            .await
            .unwrap();

        assert_eq!(response.text, "Hello, world!");
        assert!(response.tool_calls.is_none());
    }

    #[tokio::test]
    async fn test_echo_response() {
        let driver = LlmSimDriver::new(LlmSimConfig::echo());
        let messages = vec![
            system_message("You are a helpful assistant"),
            user_message("What is 2+2?"),
        ];

        let response = driver
            .chat_completion(messages, &make_config())
            .await
            .unwrap();

        assert_eq!(response.text, "Echo: What is 2+2?");
    }

    #[tokio::test]
    async fn test_sequence_response() {
        let driver = LlmSimDriver::new(LlmSimConfig::sequence(vec![
            "First".to_string(),
            "Second".to_string(),
            "Third".to_string(),
        ]));

        let messages = vec![user_message("test")];

        // First call
        let r1 = driver
            .chat_completion(messages.clone(), &make_config())
            .await
            .unwrap();
        assert_eq!(r1.text, "First");

        // Second call
        let r2 = driver
            .chat_completion(messages.clone(), &make_config())
            .await
            .unwrap();
        assert_eq!(r2.text, "Second");

        // Third call
        let r3 = driver
            .chat_completion(messages.clone(), &make_config())
            .await
            .unwrap();
        assert_eq!(r3.text, "Third");

        // Fourth call - cycles back to first
        let r4 = driver
            .chat_completion(messages.clone(), &make_config())
            .await
            .unwrap();
        assert_eq!(r4.text, "First");
    }

    #[tokio::test]
    async fn test_lorem_response() {
        let driver = LlmSimDriver::new(LlmSimConfig::lorem(50));
        let messages = vec![user_message("Generate text")];

        let response = driver
            .chat_completion(messages, &make_config())
            .await
            .unwrap();

        // Lorem response should have content
        assert!(!response.text.is_empty());
        // Should have multiple words
        assert!(response.text.split_whitespace().count() > 5);
    }

    #[tokio::test]
    async fn test_fixed_tool_calls() {
        let tool_call = ToolCall {
            id: "call_123".to_string(),
            name: "get_weather".to_string(),
            arguments: serde_json::json!({"city": "NYC"}),
        };

        let driver = LlmSimDriver::new(
            LlmSimConfig::fixed("Let me check the weather.")
                .with_tool_calls(vec![tool_call.clone()]),
        );

        let messages = vec![user_message("What's the weather?")];
        let response = driver
            .chat_completion(messages, &make_config())
            .await
            .unwrap();

        assert_eq!(response.text, "Let me check the weather.");
        let calls = response.tool_calls.expect("Expected tool calls");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "get_weather");
        assert_eq!(calls[0].id, "call_123");
    }

    #[tokio::test]
    async fn test_tool_call_sequence() {
        let call1 = ToolCall {
            id: "call_1".to_string(),
            name: "search".to_string(),
            arguments: serde_json::json!({"q": "rust"}),
        };
        let call2 = ToolCall {
            id: "call_2".to_string(),
            name: "fetch".to_string(),
            arguments: serde_json::json!({"url": "https://example.com"}),
        };

        let driver = LlmSimDriver::new(
            LlmSimConfig::fixed("Processing...").with_tool_call_sequence(vec![
                vec![call1.clone()],
                vec![call2.clone()],
                vec![],
            ]),
        );

        let messages = vec![user_message("test")];

        // First call - should get search
        let r1 = driver
            .chat_completion(messages.clone(), &make_config())
            .await
            .unwrap();
        let calls1 = r1.tool_calls.expect("Expected tool calls");
        assert_eq!(calls1[0].name, "search");

        // Second call - should get fetch
        let r2 = driver
            .chat_completion(messages.clone(), &make_config())
            .await
            .unwrap();
        let calls2 = r2.tool_calls.expect("Expected tool calls");
        assert_eq!(calls2[0].name, "fetch");

        // Third call - no tool calls
        let r3 = driver
            .chat_completion(messages.clone(), &make_config())
            .await
            .unwrap();
        assert!(r3.tool_calls.is_none());
    }

    #[tokio::test]
    async fn test_conditional_tool_calls() {
        let weather_call = ToolCall {
            id: "call_w".to_string(),
            name: "get_weather".to_string(),
            arguments: serde_json::json!({}),
        };
        let search_call = ToolCall {
            id: "call_s".to_string(),
            name: "search".to_string(),
            arguments: serde_json::json!({}),
        };

        let config = LlmSimConfig {
            response: ResponseConfig::Fixed("Response".to_string()),
            tool_calls: Some(ToolCallConfig::Conditional {
                patterns: vec![
                    ToolCallPattern::new("weather", vec![weather_call]),
                    ToolCallPattern::new("search", vec![search_call]),
                ],
            }),
            simulate_latency: false,
            model_name: "test".to_string(),
        };

        let driver = LlmSimDriver::new(config);

        // Weather query - should trigger weather tool
        let r1 = driver
            .chat_completion(vec![user_message("What's the weather?")], &make_config())
            .await
            .unwrap();
        let calls1 = r1.tool_calls.expect("Expected weather tool");
        assert_eq!(calls1[0].name, "get_weather");

        // Search query - should trigger search tool
        let r2 = driver
            .chat_completion(vec![user_message("search for rust")], &make_config())
            .await
            .unwrap();
        let calls2 = r2.tool_calls.expect("Expected search tool");
        assert_eq!(calls2[0].name, "search");

        // No matching pattern - no tool calls
        let r3 = driver
            .chat_completion(vec![user_message("hello world")], &make_config())
            .await
            .unwrap();
        assert!(r3.tool_calls.is_none());
    }

    #[tokio::test]
    async fn test_streaming() {
        let driver = LlmSimDriver::new(LlmSimConfig::fixed("Hello world test"));
        let messages = vec![user_message("test")];

        let mut stream = driver
            .chat_completion_stream(messages, &make_config())
            .await
            .unwrap();

        let mut text_parts = Vec::new();
        let mut got_done = false;

        while let Some(event) = stream.next().await {
            match event.unwrap() {
                LlmStreamEvent::TextDelta(text) => text_parts.push(text),
                LlmStreamEvent::Done(meta) => {
                    got_done = true;
                    assert!(meta.total_tokens.is_some());
                    assert!(meta.model.is_some());
                }
                _ => {}
            }
        }

        assert!(got_done);
        // Should stream word by word
        assert_eq!(text_parts.len(), 3); // "Hello", " world", " test"
        assert_eq!(text_parts.join(""), "Hello world test");
    }

    #[tokio::test]
    async fn test_metadata() {
        let driver = LlmSimDriver::new(LlmSimConfig::fixed("Hi").with_model("custom-model"));
        let messages = vec![user_message("test")];

        let mut config = make_config();
        config.model = "request-model".to_string();

        let response = driver.chat_completion(messages, &config).await.unwrap();

        // Model should come from the request config
        assert_eq!(response.metadata.model, Some("request-model".to_string()));
        assert!(response.metadata.prompt_tokens.is_some());
        assert!(response.metadata.completion_tokens.is_some());
    }

    #[tokio::test]
    async fn test_register_driver() {
        let mut registry = DriverRegistry::new();
        register_driver(&mut registry);

        assert!(registry.has_driver(&ProviderType::LlmSim));

        // Creating a driver should work (with any API key since it's simulated)
        let config = crate::llm_driver_registry::ProviderConfig::new(ProviderType::LlmSim)
            .with_api_key("fake-key");
        let driver = registry.create_driver(&config);
        assert!(driver.is_ok());
    }

    #[tokio::test]
    async fn test_empty_response() {
        let config = LlmSimConfig {
            response: ResponseConfig::Empty,
            tool_calls: None,
            simulate_latency: false,
            model_name: "test".to_string(),
        };

        let driver = LlmSimDriver::new(config);
        let messages = vec![user_message("test")];

        let response = driver
            .chat_completion(messages, &make_config())
            .await
            .unwrap();

        assert!(response.text.is_empty());
    }

    #[test]
    fn test_driver_debug() {
        let driver = LlmSimDriver::new(LlmSimConfig::fixed("test").with_latency());
        let debug = format!("{:?}", driver);

        assert!(debug.contains("LlmSimDriver"));
        assert!(debug.contains("simulate_latency"));
    }

    #[test]
    fn test_default_config() {
        let config = LlmSimConfig::default();
        assert!(matches!(config.response, ResponseConfig::Fixed(_)));
        assert!(config.tool_calls.is_none());
        assert!(!config.simulate_latency);
    }

    #[test]
    fn test_config_builder() {
        let tool_call = ToolCall {
            id: "call_1".to_string(),
            name: "get_weather".to_string(),
            arguments: serde_json::json!({"city": "NYC"}),
        };

        let config = LlmSimConfig::fixed("Result")
            .with_tool_calls(vec![tool_call.clone()])
            .with_latency()
            .with_model("gpt-4");

        assert!(config.tool_calls.is_some());
        assert!(config.simulate_latency);
        assert_eq!(config.model_name, "gpt-4");
    }
}

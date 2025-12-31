// LLM Driver Abstractions
//
// This module encapsulates all abstractions needed to interact with LLM Providers:
// - LlmDriver trait and types for provider-agnostic LLM interactions
// - Driver factory for creating drivers based on provider type and configuration
// - Message types for LLM calls
//
// Supports both simple text content and multipart content (text, images, audio).
//
// IMPORTANT: API keys must be provided from the database. The factory does NOT read
// from environment variables. Keys should be decrypted and passed via ProviderConfig.

use crate::anthropic::AnthropicLlmDriver;
use crate::config::AgentConfig;
use crate::error::{AgentLoopError, Result};
use crate::openai::OpenAILlmDriver;
use crate::tool_types::{ToolCall, ToolDefinition};
use async_trait::async_trait;
use futures::Stream;
use std::pin::Pin;

// ============================================================================
// LlmDriver Trait
// ============================================================================

/// Type alias for the LLM response stream
pub type LlmResponseStream = Pin<Box<dyn Stream<Item = Result<LlmStreamEvent>> + Send>>;

/// Events emitted during LLM streaming
#[derive(Debug, Clone)]
pub enum LlmStreamEvent {
    /// Text delta (incremental content)
    TextDelta(String),
    /// Tool calls from the LLM
    ToolCalls(Vec<ToolCall>),
    /// Streaming completed
    Done(LlmCompletionMetadata),
    /// Error during streaming
    Error(String),
}

/// Metadata about LLM completion
#[derive(Debug, Clone, Default)]
pub struct LlmCompletionMetadata {
    /// Total tokens used
    pub total_tokens: Option<u32>,
    /// Prompt tokens
    pub prompt_tokens: Option<u32>,
    /// Completion tokens
    pub completion_tokens: Option<u32>,
    /// Model used
    pub model: Option<String>,
    /// Finish reason
    pub finish_reason: Option<String>,
}

/// Trait for LLM drivers
///
/// Implementations handle provider-specific API calls and response parsing.
#[async_trait]
pub trait LlmDriver: Send + Sync {
    /// Call the LLM with streaming response
    async fn chat_completion_stream(
        &self,
        messages: Vec<LlmMessage>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponseStream>;

    /// Call the LLM without streaming (convenience method)
    async fn chat_completion(
        &self,
        messages: Vec<LlmMessage>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponse> {
        use futures::StreamExt;

        let mut stream = self.chat_completion_stream(messages, config).await?;
        let mut text = String::new();
        let mut tool_calls = Vec::new();
        let mut metadata = LlmCompletionMetadata::default();

        while let Some(event) = stream.next().await {
            match event? {
                LlmStreamEvent::TextDelta(delta) => text.push_str(&delta),
                LlmStreamEvent::ToolCalls(calls) => tool_calls = calls,
                LlmStreamEvent::Done(meta) => metadata = meta,
                LlmStreamEvent::Error(err) => return Err(crate::error::AgentLoopError::llm(err)),
            }
        }

        Ok(LlmResponse {
            text,
            tool_calls: if tool_calls.is_empty() {
                None
            } else {
                Some(tool_calls)
            },
            metadata,
        })
    }
}

/// Implement LlmDriver for Box<dyn LlmDriver> to allow dynamic dispatch
#[async_trait]
impl LlmDriver for Box<dyn LlmDriver> {
    async fn chat_completion_stream(
        &self,
        messages: Vec<LlmMessage>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponseStream> {
        (**self).chat_completion_stream(messages, config).await
    }

    async fn chat_completion(
        &self,
        messages: Vec<LlmMessage>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponse> {
        (**self).chat_completion(messages, config).await
    }
}

// ============================================================================
// Message Types
// ============================================================================

/// Message format for LLM calls (provider-agnostic)
#[derive(Debug, Clone)]
pub struct LlmMessage {
    pub role: LlmMessageRole,
    pub content: LlmMessageContent,
    pub tool_calls: Option<Vec<ToolCall>>,
    pub tool_call_id: Option<String>,
}

impl LlmMessage {
    /// Create a message with text content
    pub fn text(role: LlmMessageRole, content: impl Into<String>) -> Self {
        Self {
            role,
            content: LlmMessageContent::Text(content.into()),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    /// Create a message with content parts (text, images, audio)
    pub fn parts(role: LlmMessageRole, parts: Vec<LlmContentPart>) -> Self {
        Self {
            role,
            content: LlmMessageContent::Parts(parts),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    /// Get content as plain text string (for simple cases)
    pub fn content_as_text(&self) -> String {
        self.content.to_text()
    }
}

/// Message content - either a simple string or array of content parts
#[derive(Debug, Clone)]
pub enum LlmMessageContent {
    /// Simple text content
    Text(String),
    /// Array of content parts (text, images, audio)
    Parts(Vec<LlmContentPart>),
}

impl LlmMessageContent {
    /// Convert to plain text (concatenates text parts, ignores media)
    pub fn to_text(&self) -> String {
        match self {
            LlmMessageContent::Text(s) => s.clone(),
            LlmMessageContent::Parts(parts) => parts
                .iter()
                .filter_map(|p| match p {
                    LlmContentPart::Text { text } => Some(text.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join(""),
        }
    }

    /// Check if content is simple text
    pub fn is_text(&self) -> bool {
        matches!(self, LlmMessageContent::Text(_))
    }

    /// Check if content has multiple parts
    pub fn is_parts(&self) -> bool {
        matches!(self, LlmMessageContent::Parts(_))
    }
}

impl From<String> for LlmMessageContent {
    fn from(s: String) -> Self {
        LlmMessageContent::Text(s)
    }
}

impl From<&str> for LlmMessageContent {
    fn from(s: &str) -> Self {
        LlmMessageContent::Text(s.to_string())
    }
}

/// A single content part within a message
#[derive(Debug, Clone)]
pub enum LlmContentPart {
    /// Text content
    Text { text: String },
    /// Image content (base64 data URL or HTTP URL)
    Image { url: String },
    /// Audio content (base64 data URL)
    Audio { url: String },
}

impl LlmContentPart {
    /// Create a text content part
    pub fn text(text: impl Into<String>) -> Self {
        LlmContentPart::Text { text: text.into() }
    }

    /// Create an image content part from URL (can be data URL or HTTP URL)
    pub fn image(url: impl Into<String>) -> Self {
        LlmContentPart::Image { url: url.into() }
    }

    /// Create an audio content part from URL (typically a data URL)
    pub fn audio(url: impl Into<String>) -> Self {
        LlmContentPart::Audio { url: url.into() }
    }
}

/// Message role for LLM calls
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LlmMessageRole {
    System,
    User,
    Assistant,
    Tool,
}

// ============================================================================
// Configuration and Response Types
// ============================================================================

/// Configuration for an LLM call
#[derive(Debug, Clone)]
pub struct LlmCallConfig {
    pub model: String,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub tools: Vec<ToolDefinition>,
    /// Reasoning effort level (for models that support it: low, medium, high)
    pub reasoning_effort: Option<String>,
}

impl From<&AgentConfig> for LlmCallConfig {
    fn from(config: &AgentConfig) -> Self {
        Self {
            model: config.model.clone(),
            temperature: config.temperature,
            max_tokens: config.max_tokens,
            tools: config.tools.clone(),
            reasoning_effort: None, // Set by CallModelAtom from user message controls
        }
    }
}

/// Response from an LLM call (non-streaming)
#[derive(Debug, Clone)]
pub struct LlmResponse {
    pub text: String,
    pub tool_calls: Option<Vec<ToolCall>>,
    pub metadata: LlmCompletionMetadata,
}

/// Builder for LlmCallConfig with fluent API
///
/// Use `from(&config)` to start building from an AgentConfig, then chain
/// methods like `reasoning_effort()`, `temperature()`, etc. Call `build()`
/// to get the final config.
///
/// # Example
///
/// ```ignore
/// use everruns_core::llm::LlmCallConfigBuilder;
/// use everruns_core::config::AgentConfig;
///
/// let agent_config = AgentConfig::new("You are helpful", "gpt-4o");
/// let llm_config = LlmCallConfigBuilder::from(&agent_config)
///     .reasoning_effort("high")
///     .temperature(0.7)
///     .build();
/// ```
pub struct LlmCallConfigBuilder {
    config: LlmCallConfig,
}

impl LlmCallConfigBuilder {
    /// Start building from an AgentConfig
    pub fn from(config: &AgentConfig) -> Self {
        Self {
            config: LlmCallConfig::from(config),
        }
    }

    /// Set reasoning effort level (for models that support it: low, medium, high)
    pub fn reasoning_effort(mut self, effort: impl Into<String>) -> Self {
        self.config.reasoning_effort = Some(effort.into());
        self
    }

    /// Set the model
    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.config.model = model.into();
        self
    }

    /// Set temperature
    pub fn temperature(mut self, temp: f32) -> Self {
        self.config.temperature = Some(temp);
        self
    }

    /// Set max tokens
    pub fn max_tokens(mut self, tokens: u32) -> Self {
        self.config.max_tokens = Some(tokens);
        self
    }

    /// Set tools
    pub fn tools(mut self, tools: Vec<ToolDefinition>) -> Self {
        self.config.tools = tools;
        self
    }

    /// Build the configuration
    pub fn build(self) -> LlmCallConfig {
        self.config
    }
}

// ============================================================================
// Conversion from Message
// ============================================================================

impl From<&crate::message::Message> for LlmMessage {
    fn from(msg: &crate::message::Message) -> Self {
        let role = match msg.role {
            crate::message::MessageRole::System => LlmMessageRole::System,
            crate::message::MessageRole::User => LlmMessageRole::User,
            crate::message::MessageRole::Assistant => LlmMessageRole::Assistant,
            crate::message::MessageRole::ToolResult => LlmMessageRole::Tool,
        };

        // Convert tool calls from ContentPart format to ToolCall format
        let tool_calls: Vec<ToolCall> = msg
            .tool_calls()
            .into_iter()
            .map(|tc| ToolCall {
                id: tc.id.clone(),
                name: tc.name.clone(),
                arguments: tc.arguments.clone(),
            })
            .collect();

        LlmMessage {
            role,
            content: LlmMessageContent::Text(msg.content_to_llm_string()),
            tool_calls: if tool_calls.is_empty() {
                None
            } else {
                Some(tool_calls)
            },
            tool_call_id: msg.tool_call_id().map(|s| s.to_string()),
        }
    }
}

// ============================================================================
// Driver Factory
// ============================================================================

/// Provider type enumeration matching the database/contracts
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderType {
    OpenAI,
    Anthropic,
    AzureOpenAI,
}

impl std::str::FromStr for ProviderType {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "openai" => Ok(ProviderType::OpenAI),
            "anthropic" => Ok(ProviderType::Anthropic),
            "azure_openai" => Ok(ProviderType::AzureOpenAI),
            _ => Err(format!("Unknown provider type: {}", s)),
        }
    }
}

impl std::fmt::Display for ProviderType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProviderType::OpenAI => write!(f, "openai"),
            ProviderType::Anthropic => write!(f, "anthropic"),
            ProviderType::AzureOpenAI => write!(f, "azure_openai"),
        }
    }
}

/// Configuration for creating an LLM provider
#[derive(Debug, Clone)]
pub struct ProviderConfig {
    /// Type of provider
    pub provider_type: ProviderType,
    /// API key for authentication
    pub api_key: Option<String>,
    /// Base URL override (optional)
    pub base_url: Option<String>,
}

impl ProviderConfig {
    /// Create a new provider config
    pub fn new(provider_type: ProviderType) -> Self {
        Self {
            provider_type,
            api_key: None,
            base_url: None,
        }
    }

    /// Set the API key
    pub fn with_api_key(mut self, api_key: impl Into<String>) -> Self {
        self.api_key = Some(api_key.into());
        self
    }

    /// Set the base URL
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = Some(base_url.into());
        self
    }
}

/// Boxed LLM driver for dynamic dispatch
pub type BoxedLlmDriver = Box<dyn LlmDriver>;

/// Create an LLM driver based on configuration
///
/// API keys must be provided in the config. This function does NOT fall back to
/// environment variables. Keys should be decrypted from the database and passed here.
pub fn create_driver(config: &ProviderConfig) -> Result<BoxedLlmDriver> {
    // API key is required - it should be decrypted from the database
    let api_key = config.api_key.as_ref().ok_or_else(|| {
        AgentLoopError::llm("API key is required. Configure the API key in provider settings.")
    })?;

    match config.provider_type {
        ProviderType::OpenAI | ProviderType::AzureOpenAI => {
            let driver = match &config.base_url {
                Some(url) => OpenAILlmDriver::with_base_url(api_key, url),
                None => OpenAILlmDriver::new(api_key),
            };
            Ok(Box::new(driver))
        }
        ProviderType::Anthropic => {
            let driver = match &config.base_url {
                Some(url) => AnthropicLlmDriver::with_base_url(api_key, url),
                None => AnthropicLlmDriver::new(api_key),
            };
            Ok(Box::new(driver))
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_llm_call_config_builder_from_agent_config() {
        let agent_config = AgentConfig::new("You are helpful", "gpt-4o");
        let llm_config = LlmCallConfigBuilder::from(&agent_config).build();

        assert_eq!(llm_config.model, "gpt-4o");
        assert!(llm_config.reasoning_effort.is_none());
        assert!(llm_config.temperature.is_none());
        assert!(llm_config.max_tokens.is_none());
        assert!(llm_config.tools.is_empty());
    }

    #[test]
    fn test_llm_call_config_builder_with_reasoning_effort() {
        let agent_config = AgentConfig::new("You are helpful", "gpt-4o");
        let llm_config = LlmCallConfigBuilder::from(&agent_config)
            .reasoning_effort("high")
            .build();

        assert_eq!(llm_config.reasoning_effort, Some("high".to_string()));
    }

    #[test]
    fn test_llm_call_config_builder_with_all_options() {
        let agent_config = AgentConfig::new("You are helpful", "gpt-4o");
        let llm_config = LlmCallConfigBuilder::from(&agent_config)
            .model("claude-3-opus")
            .reasoning_effort("medium")
            .temperature(0.7)
            .max_tokens(1000)
            .build();

        assert_eq!(llm_config.model, "claude-3-opus");
        assert_eq!(llm_config.reasoning_effort, Some("medium".to_string()));
        assert_eq!(llm_config.temperature, Some(0.7));
        assert_eq!(llm_config.max_tokens, Some(1000));
    }

    #[test]
    fn test_provider_type_parsing() {
        assert_eq!(
            "openai".parse::<ProviderType>().unwrap(),
            ProviderType::OpenAI
        );
        assert_eq!(
            "anthropic".parse::<ProviderType>().unwrap(),
            ProviderType::Anthropic
        );
        assert_eq!(
            "azure_openai".parse::<ProviderType>().unwrap(),
            ProviderType::AzureOpenAI
        );
        // Ollama and Custom are no longer supported
        assert!("ollama".parse::<ProviderType>().is_err());
        assert!("custom".parse::<ProviderType>().is_err());
    }

    #[test]
    fn test_provider_type_display() {
        assert_eq!(ProviderType::OpenAI.to_string(), "openai");
        assert_eq!(ProviderType::Anthropic.to_string(), "anthropic");
        assert_eq!(ProviderType::AzureOpenAI.to_string(), "azure_openai");
    }

    #[test]
    fn test_provider_config_builder() {
        let config = ProviderConfig::new(ProviderType::Anthropic)
            .with_api_key("test-key")
            .with_base_url("https://custom.api.com");

        assert_eq!(config.provider_type, ProviderType::Anthropic);
        assert_eq!(config.api_key, Some("test-key".to_string()));
        assert_eq!(config.base_url, Some("https://custom.api.com".to_string()));
    }

    #[test]
    fn test_create_driver_requires_api_key() {
        // Driver without API key should fail
        let config = ProviderConfig::new(ProviderType::OpenAI);
        let result = create_driver(&config);
        assert!(result.is_err());

        // Driver with API key should succeed
        let config_with_key = ProviderConfig::new(ProviderType::OpenAI).with_api_key("test-key");
        let result = create_driver(&config_with_key);
        assert!(result.is_ok());
    }
}

// LLM Driver Abstractions
//
// This module encapsulates all abstractions needed to interact with LLM Providers:
// - LlmDriver trait and types for provider-agnostic LLM interactions
// - DriverRegistry for dynamic driver registration at startup
// - Message types for LLM calls
//
// Supports both simple text content and multipart content (text, images, audio).
//
// IMPORTANT: API keys must be provided from the database. The registry does NOT read
// from environment variables. Keys should be decrypted and passed via ProviderConfig.
//
// Design: Dependency inversion - provider crates (everruns-anthropic, everruns-openai)
// depend on core and register their drivers at startup. Core has no knowledge of
// specific provider implementations.

use crate::error::{AgentLoopError, Result};
use crate::runtime_agent::RuntimeAgent;
use crate::tool_types::{ToolCall, ToolDefinition};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::Stream;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;

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
    /// Thinking delta (incremental reasoning content from extended thinking models)
    ThinkingDelta(String),
    /// Cryptographic signature for thinking content (Anthropic Claude)
    /// Emitted when a thinking block completes, before the Done event
    ThinkingSignature(String),
    /// Tool calls from the LLM
    ToolCalls(Vec<ToolCall>),
    /// Streaming completed
    Done(LlmCompletionMetadata),
    /// Error during streaming
    Error(String),
}

/// Model information discovered from a provider's list_models API
///
/// Represents a model available from a provider. Used for dynamic model discovery
/// to sync available models from provider APIs into the database.
#[derive(Debug, Clone)]
pub struct DiscoveredModel {
    /// Model identifier (e.g., "gpt-5.2", "claude-opus-4-5-20251101")
    pub model_id: String,
    /// Human-readable display name (if provided by API)
    pub display_name: Option<String>,
    /// When the model was created/released
    pub created_at: Option<DateTime<Utc>>,
    /// Owner or organization (e.g., "openai", "system")
    pub owned_by: Option<String>,
}

/// Metadata about LLM completion
///
/// Contains token usage and completion information from the LLM response.
/// Cache token fields are provider-specific:
/// - OpenAI: `cache_read_tokens` from prompt_tokens_details.cached_tokens
/// - Anthropic: `cache_read_tokens` from cache_read_input_tokens,
///   `cache_creation_tokens` from cache_creation_input_tokens
#[derive(Debug, Clone, Default)]
pub struct LlmCompletionMetadata {
    /// Total tokens used
    pub total_tokens: Option<u32>,
    /// Prompt tokens
    pub prompt_tokens: Option<u32>,
    /// Completion tokens
    pub completion_tokens: Option<u32>,
    /// Tokens read from cache (reduces cost)
    pub cache_read_tokens: Option<u32>,
    /// Tokens written to cache (Anthropic-specific)
    pub cache_creation_tokens: Option<u32>,
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
        let mut thinking = String::new();
        let mut thinking_signature: Option<String> = None;
        let mut tool_calls = Vec::new();
        let mut metadata = LlmCompletionMetadata::default();

        while let Some(event) = stream.next().await {
            match event? {
                LlmStreamEvent::TextDelta(delta) => text.push_str(&delta),
                LlmStreamEvent::ThinkingDelta(delta) => thinking.push_str(&delta),
                LlmStreamEvent::ThinkingSignature(sig) => thinking_signature = Some(sig),
                LlmStreamEvent::ToolCalls(calls) => tool_calls = calls,
                LlmStreamEvent::Done(meta) => metadata = meta,
                LlmStreamEvent::Error(err) => return Err(crate::error::AgentLoopError::llm(err)),
            }
        }

        Ok(LlmResponse {
            text,
            thinking: if thinking.is_empty() {
                None
            } else {
                Some(thinking)
            },
            thinking_signature,
            tool_calls: if tool_calls.is_empty() {
                None
            } else {
                Some(tool_calls)
            },
            metadata,
        })
    }

    /// List available models from the provider
    ///
    /// Returns `Ok(Some(models))` if the provider supports model listing,
    /// or `Ok(None)` if not supported (e.g., custom endpoints, proxies).
    ///
    /// Implementations should filter to chat/completion models only,
    /// excluding embedding models, TTS, whisper, etc.
    async fn list_models(&self) -> Result<Option<Vec<DiscoveredModel>>> {
        // Default: not supported. Providers override if they support listing.
        Ok(None)
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

    async fn list_models(&self) -> Result<Option<Vec<DiscoveredModel>>> {
        (**self).list_models().await
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
    /// Thinking content from extended thinking models (Anthropic Claude)
    /// Must be included in subsequent API calls when thinking is enabled
    pub thinking: Option<String>,
    /// Cryptographic signature for thinking content (Anthropic Claude)
    /// Required when sending thinking back in subsequent API calls
    pub thinking_signature: Option<String>,
}

impl LlmMessage {
    /// Create a message with text content
    pub fn text(role: LlmMessageRole, content: impl Into<String>) -> Self {
        Self {
            role,
            content: LlmMessageContent::Text(content.into()),
            tool_calls: None,
            tool_call_id: None,
            thinking: None,
            thinking_signature: None,
        }
    }

    /// Create a message with content parts (text, images, audio)
    pub fn parts(role: LlmMessageRole, parts: Vec<LlmContentPart>) -> Self {
        Self {
            role,
            content: LlmMessageContent::Parts(parts),
            tool_calls: None,
            tool_call_id: None,
            thinking: None,
            thinking_signature: None,
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
    /// Metadata to send with the API request for tracking and debugging.
    /// Keys and values are strings. Both OpenAI and Anthropic support metadata fields.
    /// Typically includes: session_id, agent_id, org_id, turn_id, exec_id.
    pub metadata: HashMap<String, String>,
}

impl From<&RuntimeAgent> for LlmCallConfig {
    fn from(runtime_agent: &RuntimeAgent) -> Self {
        Self {
            model: runtime_agent.model.clone(),
            temperature: runtime_agent.temperature,
            max_tokens: runtime_agent.max_tokens,
            tools: runtime_agent.tools.clone(),
            reasoning_effort: None, // Set by ReasonAtom from user message controls
            metadata: HashMap::new(), // Set by ReasonAtom with session/agent context
        }
    }
}

/// Response from an LLM call (non-streaming)
#[derive(Debug, Clone)]
pub struct LlmResponse {
    pub text: String,
    /// Thinking content from extended thinking models (e.g., Claude with thinking enabled)
    pub thinking: Option<String>,
    /// Cryptographic signature for thinking content (Anthropic Claude)
    pub thinking_signature: Option<String>,
    pub tool_calls: Option<Vec<ToolCall>>,
    pub metadata: LlmCompletionMetadata,
}

/// Builder for LlmCallConfig with fluent API
///
/// Use `from(&runtime_agent)` to start building from a RuntimeAgent, then chain
/// methods like `reasoning_effort()`, `temperature()`, etc. Call `build()`
/// to get the final config.
///
/// # Example
///
/// ```ignore
/// use everruns_core::llm::LlmCallConfigBuilder;
/// use everruns_core::runtime_agent::RuntimeAgent;
///
/// let runtime_agent = RuntimeAgent::new("You are helpful", "gpt-4o");
/// let llm_config = LlmCallConfigBuilder::from(&runtime_agent)
///     .reasoning_effort("high")
///     .temperature(0.7)
///     .build();
/// ```
pub struct LlmCallConfigBuilder {
    config: LlmCallConfig,
}

impl LlmCallConfigBuilder {
    /// Start building from a RuntimeAgent
    pub fn from(runtime_agent: &RuntimeAgent) -> Self {
        Self {
            config: LlmCallConfig::from(runtime_agent),
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

    /// Set metadata for API tracking
    ///
    /// This metadata is sent to the LLM provider for tracking and debugging.
    /// Typically includes session_id, agent_id, org_id, turn_id, exec_id.
    pub fn metadata(mut self, metadata: HashMap<String, String>) -> Self {
        self.config.metadata = metadata;
        self
    }

    /// Add a single metadata key-value pair
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.config.metadata.insert(key.into(), value.into());
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
    /// Convert a Message to LlmMessage (text-only, images become placeholders)
    ///
    /// This conversion is suitable for messages without images or when image
    /// resolution is not available. For multimodal messages, use
    /// `LlmMessage::from_message_with_images()` instead.
    fn from(msg: &crate::message::Message) -> Self {
        let role = match msg.role {
            crate::message::MessageRole::System => LlmMessageRole::System,
            crate::message::MessageRole::User => LlmMessageRole::User,
            crate::message::MessageRole::Agent => LlmMessageRole::Assistant,
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
            thinking: msg.thinking.clone(),
            thinking_signature: msg.thinking_signature.clone(),
        }
    }
}

// ============================================================================
// Message Conversion with Images
// ============================================================================

use crate::traits::ResolvedImage;
use uuid::Uuid;

impl LlmMessage {
    /// Convert a Message to LlmMessage with resolved images
    ///
    /// This method handles multimodal messages by converting:
    /// - `text` content parts → `LlmContentPart::Text`
    /// - `image` content parts → `LlmContentPart::Image` (data URL)
    /// - `image_file` content parts → `LlmContentPart::Image` (resolved to data URL)
    /// - `tool_call` content parts → extracted to `tool_calls` field
    /// - `tool_result` content parts → text representation
    ///
    /// # Provider-specific formatting
    ///
    /// The `LlmContentPart::Image` uses data URLs which are converted by each provider:
    /// - **OpenAI**: `{ "type": "image_url", "image_url": { "url": "data:..." } }`
    /// - **Anthropic**: `{ "type": "image", "source": { "type": "base64", ... } }`
    ///
    /// # Arguments
    ///
    /// * `msg` - The message to convert
    /// * `resolved_images` - Pre-resolved images keyed by image_id
    pub fn from_message_with_images(
        msg: &crate::message::Message,
        resolved_images: &HashMap<Uuid, ResolvedImage>,
    ) -> Self {
        use crate::message::{ContentPart, MessageRole};

        let role = match msg.role {
            MessageRole::System => LlmMessageRole::System,
            MessageRole::User => LlmMessageRole::User,
            MessageRole::Agent => LlmMessageRole::Assistant,
            MessageRole::ToolResult => LlmMessageRole::Tool,
        };

        // Convert content parts to LlmContentParts
        let mut parts: Vec<LlmContentPart> = Vec::new();
        let mut tool_calls: Vec<ToolCall> = Vec::new();

        for part in &msg.content {
            match part {
                ContentPart::Text(t) => {
                    parts.push(LlmContentPart::Text {
                        text: t.text.clone(),
                    });
                }
                ContentPart::Image(img) => {
                    // Convert inline image to data URL
                    if let Some(url) = &img.url {
                        parts.push(LlmContentPart::Image { url: url.clone() });
                    } else if let (Some(base64), Some(media_type)) = (&img.base64, &img.media_type)
                    {
                        let data_url = format!("data:{};base64,{}", media_type, base64);
                        parts.push(LlmContentPart::Image { url: data_url });
                    }
                }
                ContentPart::ImageFile(img_file) => {
                    // Resolve image_file to actual image data
                    if let Some(resolved) = resolved_images.get(&img_file.image_id.uuid()) {
                        parts.push(LlmContentPart::Image {
                            url: resolved.to_data_url(),
                        });
                    } else {
                        // Image not found - add placeholder text
                        parts.push(LlmContentPart::Text {
                            text: format!("[Image not found: {}]", img_file.image_id),
                        });
                    }
                }
                ContentPart::ToolCall(tc) => {
                    // Extract tool calls to separate field (don't include in content)
                    tool_calls.push(ToolCall {
                        id: tc.id.clone(),
                        name: tc.name.clone(),
                        arguments: tc.arguments.clone(),
                    });
                }
                ContentPart::ToolResult(tr) => {
                    // Convert tool result to text representation
                    let text = if let Some(err) = &tr.error {
                        format!("Tool error: {}", err)
                    } else if let Some(res) = &tr.result {
                        serde_json::to_string(res).unwrap_or_else(|_| "{}".to_string())
                    } else {
                        "{}".to_string()
                    };
                    parts.push(LlmContentPart::Text { text });
                }
            }
        }

        // Determine content format
        let content = if parts.len() == 1 && matches!(&parts[0], LlmContentPart::Text { .. }) {
            // Single text part - use simple Text format
            if let LlmContentPart::Text { text } = &parts[0] {
                LlmMessageContent::Text(text.clone())
            } else {
                LlmMessageContent::Parts(parts)
            }
        } else if parts.is_empty() {
            // No content parts - use empty text
            LlmMessageContent::Text(String::new())
        } else {
            // Multiple parts or non-text - use Parts format
            LlmMessageContent::Parts(parts)
        };

        LlmMessage {
            role,
            content,
            tool_calls: if tool_calls.is_empty() {
                None
            } else {
                Some(tool_calls)
            },
            tool_call_id: msg.tool_call_id().map(|s| s.to_string()),
            thinking: msg.thinking.clone(),
            thinking_signature: msg.thinking_signature.clone(),
        }
    }

    /// Check if a message contains image_file references that need resolution
    pub fn message_has_image_files(msg: &crate::message::Message) -> bool {
        msg.content.iter().any(|p| p.is_image_file())
    }

    /// Extract all image_file IDs from a message
    pub fn extract_image_file_ids(msg: &crate::message::Message) -> Vec<Uuid> {
        msg.content
            .iter()
            .filter_map(|p| match p {
                crate::message::ContentPart::ImageFile(f) => Some(f.image_id.uuid()),
                _ => None,
            })
            .collect()
    }
}

// ============================================================================
// Driver Factory Types
// ============================================================================

/// Provider type enumeration matching the database/contracts
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ProviderType {
    /// OpenAI using Open Responses API (https://www.openresponses.org/)
    /// This is the recommended API for new projects.
    OpenAI,
    /// OpenAI using Chat Completions API (for backward compatibility)
    /// Use this if you need the legacy /v1/chat/completions endpoint.
    OpenAICompletions,
    Anthropic,
    /// LLM simulator for testing (uses llmsim crate)
    LlmSim,
}

impl std::str::FromStr for ProviderType {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "openai" => Ok(ProviderType::OpenAI),
            "openai_completions" => Ok(ProviderType::OpenAICompletions),
            "anthropic" => Ok(ProviderType::Anthropic),
            "llmsim" => Ok(ProviderType::LlmSim),
            _ => Err(format!("Unknown provider type: {}", s)),
        }
    }
}

impl std::fmt::Display for ProviderType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProviderType::OpenAI => write!(f, "openai"),
            ProviderType::OpenAICompletions => write!(f, "openai_completions"),
            ProviderType::Anthropic => write!(f, "anthropic"),
            ProviderType::LlmSim => write!(f, "llmsim"),
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

// ============================================================================
// Driver Registry
// ============================================================================

/// Factory function type for creating LLM drivers
///
/// Takes api_key and optional base_url, returns a boxed driver
pub type DriverFactory = Arc<dyn Fn(&str, Option<&str>) -> BoxedLlmDriver + Send + Sync>;

/// Registry for LLM drivers
///
/// Enables dependency inversion: provider crates (everruns-anthropic, everruns-openai)
/// register their drivers at startup. The core has no direct knowledge of implementations.
///
/// # Example
///
/// ```ignore
/// use everruns_core::llm_drivers::{DriverRegistry, ProviderType};
/// use everruns_anthropic::register_driver;
/// use everruns_openai::register_driver as register_openai;
///
/// let mut registry = DriverRegistry::new();
/// everruns_anthropic::register_driver(&mut registry);
/// everruns_openai::register_driver(&mut registry);
///
/// // Later, create a driver from config
/// let driver = registry.create_driver(&config)?;
/// ```
#[derive(Clone, Default)]
pub struct DriverRegistry {
    factories: HashMap<ProviderType, DriverFactory>,
}

impl DriverRegistry {
    /// Create a new empty registry
    pub fn new() -> Self {
        Self {
            factories: HashMap::new(),
        }
    }

    /// Register a driver factory for a provider type
    pub fn register<F>(&mut self, provider_type: ProviderType, factory: F)
    where
        F: Fn(&str, Option<&str>) -> BoxedLlmDriver + Send + Sync + 'static,
    {
        self.factories.insert(provider_type, Arc::new(factory));
    }

    /// Create an LLM driver based on configuration
    ///
    /// API keys must be provided in the config for real providers. This function does NOT fall back to
    /// environment variables. Keys should be decrypted from the database and passed here.
    /// Exception: LlmSim provider does not require an API key.
    ///
    /// Returns `DriverNotRegistered` error if no driver is registered for the provider type.
    pub fn create_driver(&self, config: &ProviderConfig) -> Result<BoxedLlmDriver> {
        // API key is required for real providers, but not for LlmSim (testing)
        let api_key = if config.provider_type == ProviderType::LlmSim {
            // LlmSim doesn't need a real API key
            config.api_key.as_deref().unwrap_or("")
        } else {
            config.api_key.as_ref().ok_or_else(|| {
                AgentLoopError::llm(
                    "API key is required. Configure the API key in provider settings.",
                )
            })?
        };

        // Look up the factory for this provider type
        let factory = self.factories.get(&config.provider_type).ok_or_else(|| {
            AgentLoopError::driver_not_registered(config.provider_type.to_string())
        })?;

        // Create the driver using the factory
        Ok(factory(api_key, config.base_url.as_deref()))
    }

    /// Check if a driver is registered for a provider type
    pub fn has_driver(&self, provider_type: &ProviderType) -> bool {
        self.factories.contains_key(provider_type)
    }

    /// Get the list of registered provider types
    pub fn registered_providers(&self) -> Vec<ProviderType> {
        self.factories.keys().cloned().collect()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_llm_call_config_builder_from_runtime_agent() {
        let runtime_agent = RuntimeAgent::new("You are helpful", "gpt-4o");
        let llm_config = LlmCallConfigBuilder::from(&runtime_agent).build();

        assert_eq!(llm_config.model, "gpt-4o");
        assert!(llm_config.reasoning_effort.is_none());
        assert!(llm_config.temperature.is_none());
        assert!(llm_config.max_tokens.is_none());
        assert!(llm_config.tools.is_empty());
        assert!(llm_config.metadata.is_empty());
    }

    #[test]
    fn test_llm_call_config_builder_with_metadata() {
        let runtime_agent = RuntimeAgent::new("You are helpful", "gpt-4o");
        let llm_config = LlmCallConfigBuilder::from(&runtime_agent)
            .with_metadata("session_id", "session_abc123")
            .with_metadata("agent_id", "agent_xyz789")
            .build();

        assert_eq!(
            llm_config.metadata.get("session_id"),
            Some(&"session_abc123".to_string())
        );
        assert_eq!(
            llm_config.metadata.get("agent_id"),
            Some(&"agent_xyz789".to_string())
        );
    }

    #[test]
    fn test_llm_call_config_builder_with_metadata_hashmap() {
        let runtime_agent = RuntimeAgent::new("You are helpful", "gpt-4o");
        let mut metadata = HashMap::new();
        metadata.insert("key1".to_string(), "value1".to_string());
        metadata.insert("key2".to_string(), "value2".to_string());

        let llm_config = LlmCallConfigBuilder::from(&runtime_agent)
            .metadata(metadata)
            .build();

        assert_eq!(llm_config.metadata.get("key1"), Some(&"value1".to_string()));
        assert_eq!(llm_config.metadata.get("key2"), Some(&"value2".to_string()));
    }

    #[test]
    fn test_llm_call_config_builder_with_reasoning_effort() {
        let runtime_agent = RuntimeAgent::new("You are helpful", "gpt-4o");
        let llm_config = LlmCallConfigBuilder::from(&runtime_agent)
            .reasoning_effort("high")
            .build();

        assert_eq!(llm_config.reasoning_effort, Some("high".to_string()));
    }

    #[test]
    fn test_llm_call_config_builder_with_all_options() {
        let runtime_agent = RuntimeAgent::new("You are helpful", "gpt-4o");
        let llm_config = LlmCallConfigBuilder::from(&runtime_agent)
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
            "openai_completions".parse::<ProviderType>().unwrap(),
            ProviderType::OpenAICompletions
        );
        assert_eq!(
            "anthropic".parse::<ProviderType>().unwrap(),
            ProviderType::Anthropic
        );
        // Azure, Ollama and Custom are no longer supported
        assert!("azure_openai".parse::<ProviderType>().is_err());
        assert!("ollama".parse::<ProviderType>().is_err());
        assert!("custom".parse::<ProviderType>().is_err());
    }

    #[test]
    fn test_provider_type_display() {
        assert_eq!(ProviderType::OpenAI.to_string(), "openai");
        assert_eq!(
            ProviderType::OpenAICompletions.to_string(),
            "openai_completions"
        );
        assert_eq!(ProviderType::Anthropic.to_string(), "anthropic");
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
    fn test_driver_registry_requires_api_key() {
        // Register a mock factory
        let mut registry = DriverRegistry::new();
        registry.register(ProviderType::OpenAI, |_api_key, _base_url| {
            // Return a mock driver - just need something that compiles
            struct MockDriver;
            #[async_trait]
            impl LlmDriver for MockDriver {
                async fn chat_completion_stream(
                    &self,
                    _messages: Vec<LlmMessage>,
                    _config: &LlmCallConfig,
                ) -> Result<LlmResponseStream> {
                    unimplemented!()
                }
            }
            Box::new(MockDriver)
        });

        // Driver without API key should fail
        let config = ProviderConfig::new(ProviderType::OpenAI);
        let result = registry.create_driver(&config);
        assert!(result.is_err());

        // Driver with API key should succeed
        let config_with_key = ProviderConfig::new(ProviderType::OpenAI).with_api_key("test-key");
        let result = registry.create_driver(&config_with_key);
        assert!(result.is_ok());
    }

    #[test]
    fn test_driver_registry_returns_error_for_unregistered_provider() {
        let registry = DriverRegistry::new();
        let config = ProviderConfig::new(ProviderType::Anthropic).with_api_key("test-key");

        let result = registry.create_driver(&config);

        // Should fail with DriverNotRegistered error
        if let Err(AgentLoopError::DriverNotRegistered(provider)) = result {
            assert_eq!(provider, "anthropic");
        } else {
            panic!("Expected DriverNotRegistered error");
        }
    }

    #[test]
    fn test_driver_registry_registration() {
        let mut registry = DriverRegistry::new();

        assert!(!registry.has_driver(&ProviderType::OpenAI));
        assert!(!registry.has_driver(&ProviderType::Anthropic));

        registry.register(ProviderType::OpenAI, |_, _| {
            struct MockDriver;
            #[async_trait]
            impl LlmDriver for MockDriver {
                async fn chat_completion_stream(
                    &self,
                    _messages: Vec<LlmMessage>,
                    _config: &LlmCallConfig,
                ) -> Result<LlmResponseStream> {
                    unimplemented!()
                }
            }
            Box::new(MockDriver)
        });

        assert!(registry.has_driver(&ProviderType::OpenAI));
        assert!(!registry.has_driver(&ProviderType::Anthropic));
    }

    // ========================================================================
    // Image resolution tests
    // ========================================================================

    use crate::{ContentPart, ImageFileContentPart, Message, MessageRole, TextContentPart};

    #[test]
    fn test_message_has_image_files_with_image_file() {
        let message = Message {
            id: uuid::Uuid::new_v4().into(),
            role: MessageRole::User,
            content: vec![
                ContentPart::Text(TextContentPart {
                    text: "Look at this image".to_string(),
                }),
                ContentPart::ImageFile(ImageFileContentPart {
                    image_id: uuid::Uuid::new_v4().into(),
                    filename: Some("test.png".to_string()),
                }),
            ],
            thinking: None,
            thinking_signature: None,
            controls: None,
            metadata: None,
            created_at: chrono::Utc::now(),
        };

        assert!(LlmMessage::message_has_image_files(&message));
    }

    #[test]
    fn test_message_has_image_files_without_image_file() {
        let message = Message {
            id: uuid::Uuid::new_v4().into(),
            role: MessageRole::User,
            content: vec![ContentPart::Text(TextContentPart {
                text: "Just text".to_string(),
            })],
            thinking: None,
            thinking_signature: None,
            controls: None,
            metadata: None,
            created_at: chrono::Utc::now(),
        };

        assert!(!LlmMessage::message_has_image_files(&message));
    }

    #[test]
    fn test_extract_image_file_ids() {
        let id1 = uuid::Uuid::new_v4();
        let id2 = uuid::Uuid::new_v4();

        let message = Message {
            id: uuid::Uuid::new_v4().into(),
            role: MessageRole::User,
            content: vec![
                ContentPart::Text(TextContentPart {
                    text: "Look at these images".to_string(),
                }),
                ContentPart::ImageFile(ImageFileContentPart {
                    image_id: id1.into(),
                    filename: Some("test1.png".to_string()),
                }),
                ContentPart::ImageFile(ImageFileContentPart {
                    image_id: id2.into(),
                    filename: Some("test2.png".to_string()),
                }),
            ],
            thinking: None,
            thinking_signature: None,
            controls: None,
            metadata: None,
            created_at: chrono::Utc::now(),
        };

        let ids = LlmMessage::extract_image_file_ids(&message);
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&id1));
        assert!(ids.contains(&id2));
    }

    #[test]
    fn test_from_message_with_images_text_only() {
        let message = Message {
            id: uuid::Uuid::new_v4().into(),
            role: MessageRole::User,
            content: vec![ContentPart::Text(TextContentPart {
                text: "Hello".to_string(),
            })],
            thinking: None,
            thinking_signature: None,
            controls: None,
            metadata: None,
            created_at: chrono::Utc::now(),
        };

        let resolved = std::collections::HashMap::new();
        let llm_message = LlmMessage::from_message_with_images(&message, &resolved);

        assert_eq!(llm_message.role, LlmMessageRole::User);
        match llm_message.content {
            LlmMessageContent::Text(text) => assert_eq!(text, "Hello"),
            _ => panic!("Expected text content"),
        }
    }

    #[test]
    fn test_from_message_with_images_resolved_image() {
        let image_id = uuid::Uuid::new_v4();
        let message = Message {
            id: uuid::Uuid::new_v4().into(),
            role: MessageRole::User,
            content: vec![
                ContentPart::Text(TextContentPart {
                    text: "Look at this".to_string(),
                }),
                ContentPart::ImageFile(ImageFileContentPart {
                    image_id: image_id.into(),
                    filename: Some("test.png".to_string()),
                }),
            ],
            thinking: None,
            thinking_signature: None,
            controls: None,
            metadata: None,
            created_at: chrono::Utc::now(),
        };

        let mut resolved = std::collections::HashMap::new();
        resolved.insert(
            image_id,
            crate::ResolvedImage::new("base64data", "image/png"),
        );

        let llm_message = LlmMessage::from_message_with_images(&message, &resolved);

        match &llm_message.content {
            LlmMessageContent::Parts(parts) => {
                assert_eq!(parts.len(), 2);
                // First part should be text
                assert!(matches!(&parts[0], LlmContentPart::Text { .. }));
                // Second part should be resolved image
                if let LlmContentPart::Image { url } = &parts[1] {
                    assert!(url.starts_with("data:image/png;base64,"));
                } else {
                    panic!("Expected image content part");
                }
            }
            _ => panic!("Expected parts content"),
        }
    }

    #[test]
    fn test_from_message_with_images_unresolved_image() {
        let image_id = uuid::Uuid::new_v4();
        let message = Message {
            id: uuid::Uuid::new_v4().into(),
            role: MessageRole::User,
            content: vec![ContentPart::ImageFile(ImageFileContentPart {
                image_id: image_id.into(),
                filename: Some("missing.png".to_string()),
            })],
            thinking: None,
            thinking_signature: None,
            controls: None,
            metadata: None,
            created_at: chrono::Utc::now(),
        };

        // Empty resolved map - image not found
        let resolved = std::collections::HashMap::new();
        let llm_message = LlmMessage::from_message_with_images(&message, &resolved);

        // Should have placeholder text for missing image
        // When there's only one part, it may return Text directly instead of Parts
        match &llm_message.content {
            LlmMessageContent::Text(text) => {
                assert!(text.contains("Image not found"));
            }
            LlmMessageContent::Parts(parts) => {
                assert_eq!(parts.len(), 1);
                if let LlmContentPart::Text { text } = &parts[0] {
                    assert!(text.contains("Image not found"));
                } else {
                    panic!("Expected text placeholder for missing image");
                }
            }
        }
    }
}

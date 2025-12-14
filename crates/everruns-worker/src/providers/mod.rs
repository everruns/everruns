// LLM provider abstraction layer
//
// This module provides a provider-agnostic interface for LLM interactions.
// Providers (OpenAI, Anthropic, etc.) implement the LlmProvider trait.

use anyhow::Result;
use everruns_contracts::tools::{ToolCall, ToolDefinition};
use futures::Stream;
use serde::{Deserialize, Serialize};
use std::pin::Pin;

pub mod openai;
#[cfg(test)]
mod tests;

/// Type alias for boxed async stream of LLM events
pub type LlmStream = Pin<Box<dyn Stream<Item = Result<LlmStreamEvent>> + Send>>;

/// Provider-agnostic LLM provider trait
/// Implementations handle provider-specific API details
#[async_trait::async_trait]
pub trait LlmProvider: Send + Sync {
    /// Stream chat completion with the given messages and configuration
    async fn chat_completion_stream(
        &self,
        messages: Vec<ChatMessage>,
        config: &LlmConfig,
    ) -> Result<LlmStream>;
}

/// Provider-agnostic chat message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: MessageRole,
    pub content: String,
    /// Tool call results (for assistant messages with tool calls)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    /// Tool call ID (for tool result messages)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

/// Message role in conversation
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    System,
    User,
    Assistant,
    Tool,
}

/// Provider-agnostic LLM configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmConfig {
    /// Model identifier (provider-specific)
    pub model: String,
    /// Sampling temperature (0.0 - 2.0)
    pub temperature: Option<f32>,
    /// Maximum tokens to generate
    pub max_tokens: Option<u32>,
    /// System prompt (if not in messages)
    pub system_prompt: Option<String>,
    /// Available tools (for function calling)
    #[serde(default)]
    pub tools: Vec<ToolDefinition>,
}

/// Events emitted during LLM streaming
#[derive(Debug, Clone)]
pub enum LlmStreamEvent {
    /// Text delta (incremental content)
    TextDelta(String),
    /// Tool calls from the LLM
    ToolCalls(Vec<ToolCall>),
    /// Streaming completed successfully
    Done(CompletionMetadata),
    /// Error occurred during streaming
    Error(String),
}

/// Completion metadata returned on stream completion
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionMetadata {
    /// Total tokens used (if available)
    pub total_tokens: Option<u32>,
    /// Input tokens used (if available)
    pub prompt_tokens: Option<u32>,
    /// Output tokens generated (if available)
    pub completion_tokens: Option<u32>,
    /// Model used
    pub model: String,
    /// Finish reason
    pub finish_reason: Option<String>,
}

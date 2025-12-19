// Core traits for pluggable backends
//
// These traits allow the agent loop to be used with different backends:
// - In-memory implementations for examples and testing
// - Database implementations for production
// - Channel-based implementations for streaming

use async_trait::async_trait;
use everruns_contracts::tools::{ToolCall, ToolDefinition, ToolResult};
use futures::Stream;
use std::pin::Pin;
use uuid::Uuid;

use crate::config::AgentConfig;
use crate::error::Result;
use crate::events::LoopEvent;
use crate::message::ConversationMessage;

// ============================================================================
// EventEmitter - For streaming events during execution
// ============================================================================

/// Trait for emitting events during loop execution
///
/// Implementations can:
/// - Store events in a database
/// - Send events to a channel for SSE streaming
/// - Collect events in memory for testing
/// - Do nothing (no-op implementation)
#[async_trait]
pub trait EventEmitter: Send + Sync {
    /// Emit a single event
    async fn emit(&self, event: LoopEvent) -> Result<()>;

    /// Emit multiple events
    async fn emit_batch(&self, events: Vec<LoopEvent>) -> Result<()> {
        for event in events {
            self.emit(event).await?;
        }
        Ok(())
    }
}

// ============================================================================
// MessageStore - For persisting conversation messages
// ============================================================================

/// Trait for storing and retrieving conversation messages
///
/// Implementations can:
/// - Store messages in a database
/// - Keep messages in memory for testing
/// - Store messages in a file
#[async_trait]
pub trait MessageStore: Send + Sync {
    /// Store a message
    async fn store(&self, session_id: Uuid, message: ConversationMessage) -> Result<()>;

    /// Store multiple messages
    async fn store_batch(
        &self,
        session_id: Uuid,
        messages: Vec<ConversationMessage>,
    ) -> Result<()> {
        for message in messages {
            self.store(session_id, message).await?;
        }
        Ok(())
    }

    /// Load all messages for a session
    async fn load(&self, session_id: Uuid) -> Result<Vec<ConversationMessage>>;

    /// Load messages with pagination
    async fn load_page(
        &self,
        session_id: Uuid,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<ConversationMessage>> {
        let all = self.load(session_id).await?;
        Ok(all.into_iter().skip(offset).take(limit).collect())
    }

    /// Count messages in a session
    async fn count(&self, session_id: Uuid) -> Result<usize> {
        Ok(self.load(session_id).await?.len())
    }
}

// ============================================================================
// LlmProvider - For calling LLM models
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

/// Trait for LLM providers
///
/// Implementations handle provider-specific API calls and response parsing.
#[async_trait]
pub trait LlmProvider: Send + Sync {
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

/// Message format for LLM calls (provider-agnostic)
#[derive(Debug, Clone)]
pub struct LlmMessage {
    pub role: LlmMessageRole,
    pub content: String,
    pub tool_calls: Option<Vec<ToolCall>>,
    pub tool_call_id: Option<String>,
}

/// Message role for LLM calls
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LlmMessageRole {
    System,
    User,
    Assistant,
    Tool,
}

/// Configuration for an LLM call
#[derive(Debug, Clone)]
pub struct LlmCallConfig {
    pub model: String,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub tools: Vec<ToolDefinition>,
}

impl From<&AgentConfig> for LlmCallConfig {
    fn from(config: &AgentConfig) -> Self {
        Self {
            model: config.model.clone(),
            temperature: config.temperature,
            max_tokens: config.max_tokens,
            tools: config.tools.clone(),
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

// ============================================================================
// ToolExecutor - For executing tool calls
// ============================================================================

/// Trait for executing tool calls
///
/// Implementations handle the actual tool execution:
/// - Webhook calls
/// - Built-in function execution
/// - Mock execution for testing
#[async_trait]
pub trait ToolExecutor: Send + Sync {
    /// Execute a single tool call
    async fn execute(&self, tool_call: &ToolCall, tool_def: &ToolDefinition) -> Result<ToolResult>;

    /// Execute multiple tool calls (default: sequential)
    async fn execute_batch(
        &self,
        tool_calls: &[ToolCall],
        tool_defs: &[ToolDefinition],
    ) -> Result<Vec<ToolResult>> {
        let mut results = Vec::with_capacity(tool_calls.len());

        // Build a map of tool names to definitions
        let tool_map: std::collections::HashMap<&str, &ToolDefinition> = tool_defs
            .iter()
            .map(|def| {
                let name = match def {
                    ToolDefinition::Webhook(w) => w.name.as_str(),
                    ToolDefinition::Builtin(b) => b.name.as_str(),
                };
                (name, def)
            })
            .collect();

        for tool_call in tool_calls {
            let tool_def = tool_map.get(tool_call.name.as_str()).ok_or_else(|| {
                crate::error::AgentLoopError::tool(format!(
                    "Tool definition not found: {}",
                    tool_call.name
                ))
            })?;

            results.push(self.execute(tool_call, tool_def).await?);
        }

        Ok(results)
    }

    /// Execute multiple tool calls in parallel
    async fn execute_parallel(
        &self,
        tool_calls: &[ToolCall],
        tool_defs: &[ToolDefinition],
    ) -> Result<Vec<ToolResult>>
    where
        Self: Sized,
    {
        use futures::future::join_all;

        // Build a map of tool names to definitions
        let tool_map: std::collections::HashMap<&str, &ToolDefinition> = tool_defs
            .iter()
            .map(|def| {
                let name = match def {
                    ToolDefinition::Webhook(w) => w.name.as_str(),
                    ToolDefinition::Builtin(b) => b.name.as_str(),
                };
                (name, def)
            })
            .collect();

        let futures: Vec<_> = tool_calls
            .iter()
            .map(|tool_call| async {
                let tool_def = tool_map.get(tool_call.name.as_str()).ok_or_else(|| {
                    crate::error::AgentLoopError::tool(format!(
                        "Tool definition not found: {}",
                        tool_call.name
                    ))
                })?;
                self.execute(tool_call, tool_def).await
            })
            .collect();

        let results = join_all(futures).await;
        results.into_iter().collect()
    }
}

// ============================================================================
// Conversion helpers
// ============================================================================

impl From<&ConversationMessage> for LlmMessage {
    fn from(msg: &ConversationMessage) -> Self {
        let role = match msg.role {
            crate::message::MessageRole::System => LlmMessageRole::System,
            crate::message::MessageRole::User => LlmMessageRole::User,
            crate::message::MessageRole::Assistant => LlmMessageRole::Assistant,
            crate::message::MessageRole::ToolCall => LlmMessageRole::Assistant,
            crate::message::MessageRole::ToolResult => LlmMessageRole::Tool,
        };

        LlmMessage {
            role,
            content: msg.content.to_llm_string(),
            tool_calls: msg.tool_calls.clone(),
            tool_call_id: msg.tool_call_id.clone(),
        }
    }
}

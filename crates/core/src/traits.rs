// Core traits for pluggable backends
//
// These traits allow the agent loop to be used with different backends:
// - In-memory implementations for examples and testing
// - Database implementations for production
// - Channel-based implementations for streaming

use crate::agent::Agent;
use crate::llm_models::LlmProviderType;
use crate::session_file::{FileInfo, FileStat, GrepMatch, SessionFile};
use crate::tool_types::{ToolCall, ToolDefinition, ToolResult};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

/// Build a map of tool names to definitions for efficient lookup
fn build_tool_map(tool_defs: &[ToolDefinition]) -> HashMap<&str, &ToolDefinition> {
    tool_defs.iter().map(|def| (def.name(), def)).collect()
}

use crate::error::Result;

// ============================================================================
// AgentStore - For retrieving agent configurations
// ============================================================================

/// Trait for retrieving agent configurations
///
/// Implementations can:
/// - Load agents from a database
/// - Keep agents in memory for testing
/// - Load agents from a configuration file
#[async_trait]
pub trait AgentStore: Send + Sync {
    /// Get an agent by ID
    async fn get_agent(&self, agent_id: Uuid) -> Result<Option<Agent>>;
}

// ============================================================================
// SessionStore - For retrieving session information
// ============================================================================

use crate::session::Session;

/// Trait for retrieving session configurations
///
/// Implementations can:
/// - Load sessions from a database
/// - Keep sessions in memory for testing
#[async_trait]
pub trait SessionStore: Send + Sync {
    /// Get a session by ID
    async fn get_session(&self, session_id: Uuid) -> Result<Option<Session>>;
}

// ============================================================================
// LlmProviderStore - For retrieving LLM provider configurations
// ============================================================================

/// Model information with provider details needed for LLM calls
#[derive(Debug, Clone)]
pub struct ModelWithProvider {
    /// The model ID string to pass to the LLM API (e.g., "gpt-4o", "claude-3-opus")
    pub model: String,
    /// Provider type for factory selection
    pub provider_type: LlmProviderType,
    /// Decrypted API key (if configured)
    pub api_key: Option<String>,
    /// Optional base URL override
    pub base_url: Option<String>,
}

/// Trait for retrieving LLM provider and model configurations
///
/// This trait abstracts the database lookup and API key decryption needed
/// to create LLM providers at runtime.
///
/// Implementations can:
/// - Load from a database with encrypted API keys
/// - Use in-memory configurations for testing
/// - Load from environment variables for development
#[async_trait]
pub trait LlmProviderStore: Send + Sync {
    /// Get model with provider info by model UUID
    ///
    /// Returns the model string ID, provider type, decrypted API key, and base URL
    /// needed to create an LLM provider via the factory.
    async fn get_model_with_provider(&self, model_id: Uuid) -> Result<Option<ModelWithProvider>>;

    /// Get the default model with provider info
    ///
    /// Returns the system default model when an agent has no default_model_id set.
    async fn get_default_model(&self) -> Result<Option<ModelWithProvider>>;
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
    /// Execute a single tool call (without context)
    ///
    /// This is the legacy method that doesn't provide context to tools.
    /// Use `execute_with_context` when context is available.
    async fn execute(&self, tool_call: &ToolCall, tool_def: &ToolDefinition) -> Result<ToolResult>;

    /// Execute a single tool call with context
    ///
    /// This method provides runtime context to tools that need it (like filesystem tools).
    /// The default implementation delegates to `execute()`.
    async fn execute_with_context(
        &self,
        tool_call: &ToolCall,
        tool_def: &ToolDefinition,
        _context: &ToolContext,
    ) -> Result<ToolResult> {
        // Default: delegate to execute(), ignoring context
        self.execute(tool_call, tool_def).await
    }

    /// Execute multiple tool calls (default: sequential)
    async fn execute_batch(
        &self,
        tool_calls: &[ToolCall],
        tool_defs: &[ToolDefinition],
    ) -> Result<Vec<ToolResult>> {
        let mut results = Vec::with_capacity(tool_calls.len());

        let tool_map = build_tool_map(tool_defs);

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

        let tool_map = build_tool_map(tool_defs);

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
// SessionFileStore - For session filesystem operations
// ============================================================================

/// Trait for session filesystem operations
///
/// This trait abstracts file operations for tools that need to interact with
/// the session's virtual filesystem. Implementations can:
/// - Store files in a database (production)
/// - Use an in-memory filesystem for testing
#[async_trait]
pub trait SessionFileStore: Send + Sync {
    /// Read a file by path
    async fn read_file(&self, session_id: Uuid, path: &str) -> Result<Option<SessionFile>>;

    /// Write/create a file
    async fn write_file(
        &self,
        session_id: Uuid,
        path: &str,
        content: &str,
        encoding: &str,
    ) -> Result<SessionFile>;

    /// Delete a file or directory
    async fn delete_file(&self, session_id: Uuid, path: &str, recursive: bool) -> Result<bool>;

    /// List files in a directory
    async fn list_directory(&self, session_id: Uuid, path: &str) -> Result<Vec<FileInfo>>;

    /// Get file metadata
    async fn stat_file(&self, session_id: Uuid, path: &str) -> Result<Option<FileStat>>;

    /// Search files by pattern (grep)
    async fn grep_files(
        &self,
        session_id: Uuid,
        pattern: &str,
        path_pattern: Option<&str>,
    ) -> Result<Vec<GrepMatch>>;

    /// Create a directory
    async fn create_directory(&self, session_id: Uuid, path: &str) -> Result<FileInfo>;
}

// ============================================================================
// SessionStorageStore - For session key/value and secret storage
// ============================================================================

/// Info about a stored key (without its value)
#[derive(Debug, Clone)]
pub struct KeyInfo {
    pub key: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Info about a stored secret (without its value)
#[derive(Debug, Clone)]
pub struct SecretInfo {
    pub name: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Trait for session key/value and secret storage operations
///
/// This trait abstracts storage operations for tools that need to persist
/// data within a session. Implementations can:
/// - Store data in a database (production)
/// - Use in-memory storage for testing
///
/// Key/value storage is for general data that doesn't need encryption.
/// Secret storage is for sensitive data that is encrypted at rest.
#[async_trait]
pub trait SessionStorageStore: Send + Sync {
    // Key/Value operations (plain text)

    /// Set a key/value pair (creates or updates)
    async fn set_value(&self, session_id: Uuid, key: &str, value: &str) -> Result<()>;

    /// Get a value by key
    async fn get_value(&self, session_id: Uuid, key: &str) -> Result<Option<String>>;

    /// Delete a key/value pair
    async fn delete_value(&self, session_id: Uuid, key: &str) -> Result<bool>;

    /// List all keys in a session
    async fn list_keys(&self, session_id: Uuid) -> Result<Vec<KeyInfo>>;

    // Secret operations (encrypted)

    /// Set a secret (creates or updates, value is encrypted before storage)
    async fn set_secret(&self, session_id: Uuid, name: &str, value: &str) -> Result<()>;

    /// Get a secret by name (value is decrypted before returning)
    async fn get_secret(&self, session_id: Uuid, name: &str) -> Result<Option<String>>;

    /// Delete a secret
    async fn delete_secret(&self, session_id: Uuid, name: &str) -> Result<bool>;

    /// List all secret names in a session (without values)
    async fn list_secrets(&self, session_id: Uuid) -> Result<Vec<SecretInfo>>;
}

// ============================================================================
// ToolContext - Runtime context for tool execution
// ============================================================================

/// Runtime context provided to tools during execution.
///
/// This context contains:
/// - Session ID for scoping operations
/// - Optional stores for tools that need external access
///
/// Tools that need context-aware execution (like filesystem tools) can use
/// the `execute_with_context` method on the Tool trait.
#[derive(Clone)]
pub struct ToolContext {
    /// The session ID for the current execution
    pub session_id: Uuid,

    /// Optional file store for filesystem operations
    pub file_store: Option<Arc<dyn SessionFileStore>>,

    /// Optional storage store for key/value and secret storage
    pub storage_store: Option<Arc<dyn SessionStorageStore>>,
}

impl ToolContext {
    /// Create a new tool context with just a session ID
    pub fn new(session_id: Uuid) -> Self {
        Self {
            session_id,
            file_store: None,
            storage_store: None,
        }
    }

    /// Create a context with a file store
    pub fn with_file_store(session_id: Uuid, file_store: Arc<dyn SessionFileStore>) -> Self {
        Self {
            session_id,
            file_store: Some(file_store),
            storage_store: None,
        }
    }

    /// Create a context with a storage store
    pub fn with_storage_store(
        session_id: Uuid,
        storage_store: Arc<dyn SessionStorageStore>,
    ) -> Self {
        Self {
            session_id,
            file_store: None,
            storage_store: Some(storage_store),
        }
    }

    /// Create a context with both file store and storage store
    pub fn with_stores(
        session_id: Uuid,
        file_store: Arc<dyn SessionFileStore>,
        storage_store: Arc<dyn SessionStorageStore>,
    ) -> Self {
        Self {
            session_id,
            file_store: Some(file_store),
            storage_store: Some(storage_store),
        }
    }
}

impl std::fmt::Debug for ToolContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolContext")
            .field("session_id", &self.session_id)
            .field("file_store", &self.file_store.is_some())
            .field("storage_store", &self.storage_store.is_some())
            .finish()
    }
}

// ============================================================================
// EventEmitter - For emitting events
// ============================================================================

use crate::events::{Event, EventRequest};

/// Trait for emitting events following the standard event protocol
///
/// Implementations can:
/// - Store events in a database
/// - Keep events in memory for testing
/// - Stream events via SSE/WebSocket
/// - Log events for debugging
///
/// Events follow a consistent schema: id, type, ts, context, data.
/// See specs/events.md for the full event protocol specification.
#[async_trait]
pub trait EventEmitter: Send + Sync {
    /// Emit an event request
    ///
    /// Takes an EventRequest (without id/sequence) and returns the stored Event
    /// with id and sequence assigned by the storage layer.
    async fn emit(&self, request: EventRequest) -> Result<Event>;
}

/// No-op event emitter for when event emission is not needed
///
/// This is useful for testing or when event observability is disabled.
#[derive(Debug, Clone, Default)]
pub struct NoopEventEmitter;

#[async_trait]
impl EventEmitter for NoopEventEmitter {
    async fn emit(&self, request: EventRequest) -> Result<Event> {
        // Return a dummy event with sequence 0
        Ok(request.into_event(uuid::Uuid::now_v7(), 0))
    }
}

// Note: EventListener trait has been moved to event_listeners.rs module.
// Use `everruns_core::EventListener` or `everruns_core::event_listeners::EventListener`.

// ============================================================================
// ImageResolver - For resolving image_file content to actual image data
// ============================================================================

/// Resolved image data for LLM consumption
///
/// This struct contains the actual image data in a format suitable for
/// sending to LLM providers. Both OpenAI and Anthropic accept base64-encoded
/// images with media type information.
#[derive(Debug, Clone)]
pub struct ResolvedImage {
    /// Base64-encoded image data (without data URL prefix)
    pub base64: String,
    /// MIME type (e.g., "image/png", "image/jpeg")
    pub media_type: String,
}

impl ResolvedImage {
    /// Create a new resolved image
    pub fn new(base64: impl Into<String>, media_type: impl Into<String>) -> Self {
        Self {
            base64: base64.into(),
            media_type: media_type.into(),
        }
    }

    /// Convert to a data URL suitable for OpenAI Vision API
    ///
    /// Format: `data:{media_type};base64,{base64_data}`
    pub fn to_data_url(&self) -> String {
        format!("data:{};base64,{}", self.media_type, self.base64)
    }
}

/// Trait for resolving image_file content parts to actual image data
///
/// When building LLM messages, `image_file` content parts contain only
/// a reference (UUID) to an uploaded image. This trait allows resolving
/// those references to actual image data.
///
/// # Provider-specific formatting
///
/// The resolved image data is then converted to provider-specific formats:
///
/// **OpenAI Vision:**
/// ```json
/// {
///   "type": "image_url",
///   "image_url": { "url": "data:image/png;base64,..." }
/// }
/// ```
///
/// **Anthropic Vision:**
/// ```json
/// {
///   "type": "image",
///   "source": { "type": "base64", "media_type": "image/png", "data": "..." }
/// }
/// ```
///
/// # Implementation notes
///
/// Implementations should:
/// - Fetch image data from storage (database, S3, etc.)
/// - Return base64-encoded data with media type
/// - Handle missing images gracefully (return None)
#[async_trait]
pub trait ImageResolver: Send + Sync {
    /// Resolve an image_file reference to actual image data
    ///
    /// Returns `None` if the image is not found.
    async fn resolve_image(&self, image_id: Uuid) -> Result<Option<ResolvedImage>>;
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolved_image_new() {
        let image = ResolvedImage::new("SGVsbG8=", "image/png");
        assert_eq!(image.base64, "SGVsbG8=");
        assert_eq!(image.media_type, "image/png");
    }

    #[test]
    fn test_resolved_image_to_data_url() {
        let image = ResolvedImage::new("SGVsbG8=", "image/png");
        let data_url = image.to_data_url();
        assert_eq!(data_url, "data:image/png;base64,SGVsbG8=");
    }

    #[test]
    fn test_resolved_image_jpeg() {
        let image = ResolvedImage::new("base64data", "image/jpeg");
        let data_url = image.to_data_url();
        assert!(data_url.starts_with("data:image/jpeg;base64,"));
    }
}

// Core traits for pluggable backends
//
// These traits allow the agent loop to be used with different backends:
// - In-memory implementations for examples and testing
// - Database implementations for production
// - Channel-based implementations for streaming

use crate::agent::Agent;
use crate::harness::Harness;
use crate::llm_models::LlmProviderType;
use crate::session_file::{FileInfo, FileStat, GrepMatch, SessionFile};
use crate::tool_types::{ToolCall, ToolDefinition, ToolResult};
use crate::typed_id::{AgentId, HarnessId, ModelId, SessionId};
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
    async fn get_agent(&self, agent_id: AgentId) -> Result<Option<Agent>>;
}

// ============================================================================
// HarnessStore - For retrieving harness configurations
// ============================================================================

/// Trait for retrieving harness configurations
///
/// Implementations can:
/// - Load harnesses from a database
/// - Keep harnesses in memory for testing
#[async_trait]
pub trait HarnessStore: Send + Sync {
    /// Get a harness by ID
    async fn get_harness(&self, harness_id: HarnessId) -> Result<Option<Harness>>;
}

// ============================================================================
// SessionStore - For retrieving session information
// ============================================================================

use crate::leased_resource::{LeasedResource, UpsertLeasedResource};
use crate::session::Session;

/// Trait for retrieving session configurations
///
/// Implementations can:
/// - Load sessions from a database
/// - Keep sessions in memory for testing
#[async_trait]
pub trait SessionStore: Send + Sync {
    /// Get a session by ID
    async fn get_session(&self, session_id: SessionId) -> Result<Option<Session>>;
}

/// Trait for updating mutable session metadata.
#[async_trait]
pub trait SessionMutator: Send + Sync {
    /// Update a session's human-readable title.
    async fn update_session_title(&self, session_id: SessionId, title: String) -> Result<Session>;
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
    /// Get model with provider info by model ID
    ///
    /// Returns the model string ID, provider type, decrypted API key, and base URL
    /// needed to create an LLM provider via the factory.
    async fn get_model_with_provider(&self, model_id: ModelId)
    -> Result<Option<ModelWithProvider>>;

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
    async fn read_file(&self, session_id: SessionId, path: &str) -> Result<Option<SessionFile>>;

    /// Write/create a file
    async fn write_file(
        &self,
        session_id: SessionId,
        path: &str,
        content: &str,
        encoding: &str,
    ) -> Result<SessionFile>;

    /// Write a file only if its current content snapshot still matches.
    ///
    /// Implementations backed by transactional storage should override this
    /// with an atomic compare-and-set update.
    async fn write_file_if_content_matches(
        &self,
        session_id: SessionId,
        path: &str,
        expected_content: &str,
        expected_encoding: &str,
        content: &str,
        encoding: &str,
    ) -> Result<Option<SessionFile>> {
        let Some(existing) = self.read_file(session_id, path).await? else {
            return Ok(None);
        };

        if existing.is_directory {
            return Ok(None);
        }

        let current_content = existing.content.unwrap_or_default();
        if current_content != expected_content || existing.encoding != expected_encoding {
            return Ok(None);
        }

        self.write_file(session_id, path, content, encoding)
            .await
            .map(Some)
    }

    /// Delete a file or directory
    async fn delete_file(&self, session_id: SessionId, path: &str, recursive: bool)
    -> Result<bool>;

    /// List files in a directory
    async fn list_directory(&self, session_id: SessionId, path: &str) -> Result<Vec<FileInfo>>;

    /// Get file metadata
    async fn stat_file(&self, session_id: SessionId, path: &str) -> Result<Option<FileStat>>;

    /// Search files by pattern (grep)
    async fn grep_files(
        &self,
        session_id: SessionId,
        pattern: &str,
        path_pattern: Option<&str>,
    ) -> Result<Vec<GrepMatch>>;

    /// Create a directory
    async fn create_directory(&self, session_id: SessionId, path: &str) -> Result<FileInfo>;
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
    async fn set_value(&self, session_id: SessionId, key: &str, value: &str) -> Result<()>;

    /// Get a value by key
    async fn get_value(&self, session_id: SessionId, key: &str) -> Result<Option<String>>;

    /// Delete a key/value pair
    async fn delete_value(&self, session_id: SessionId, key: &str) -> Result<bool>;

    /// List all keys in a session
    async fn list_keys(&self, session_id: SessionId) -> Result<Vec<KeyInfo>>;

    // Secret operations (encrypted)

    /// Set a secret (creates or updates, value is encrypted before storage)
    async fn set_secret(&self, session_id: SessionId, name: &str, value: &str) -> Result<()>;

    /// Get a secret by name (value is decrypted before returning)
    async fn get_secret(&self, session_id: SessionId, name: &str) -> Result<Option<String>>;

    /// Delete a secret
    async fn delete_secret(&self, session_id: SessionId, name: &str) -> Result<bool>;

    /// List all secret names in a session (without values)
    async fn list_secrets(&self, session_id: SessionId) -> Result<Vec<SecretInfo>>;
}

// ============================================================================
// SessionScheduleStore - For session-scoped schedule operations
// ============================================================================

use crate::session_schedule::SessionSchedule;
use crate::typed_id::ScheduleId;

/// Trait for session schedule CRUD operations.
///
/// Used by scheduling tools to create, cancel, and list schedules.
#[async_trait]
pub trait SessionScheduleStore: Send + Sync {
    /// Create a new schedule for a session.
    async fn create_schedule(
        &self,
        session_id: SessionId,
        description: String,
        cron_expression: Option<String>,
        scheduled_at: Option<chrono::DateTime<chrono::Utc>>,
        timezone: String,
    ) -> Result<SessionSchedule>;

    /// Cancel (disable) a schedule.
    async fn cancel_schedule(
        &self,
        session_id: SessionId,
        schedule_id: ScheduleId,
    ) -> Result<SessionSchedule>;

    /// List schedules for a session.
    async fn list_schedules(&self, session_id: SessionId) -> Result<Vec<SessionSchedule>>;

    /// Count active (enabled) schedules for a session.
    async fn count_active_schedules(&self, session_id: SessionId) -> Result<u32>;
}

// ============================================================================
// LeasedResourceStore - For lifecycle-managed external resources
// ============================================================================

/// Trait for session-scoped leased resource operations.
///
/// Tools use this store to register or refresh leases when they create or use
/// external provider resources. Cleanup workers operate through control-plane
/// storage APIs directly so they can claim work across organizations.
#[async_trait]
pub trait LeasedResourceStore: Send + Sync {
    /// Create or refresh a leased resource for a session.
    ///
    /// Implementations must treat this as an idempotent upsert keyed by the
    /// provider-specific resource identity so repeated tool usage extends the
    /// same lease instead of creating duplicate rows.
    async fn upsert_resource(&self, input: UpsertLeasedResource) -> Result<LeasedResource>;

    /// Mark a leased resource as explicitly released.
    ///
    /// This is the fast path for explicit user intent such as "close browser"
    /// or "delete sandbox". It should transition the resource to `released`
    /// without waiting for the durable cleanup worker to observe lease expiry.
    async fn release_resource(
        &self,
        session_id: SessionId,
        provider: &str,
        resource_type: &str,
        external_id: &str,
    ) -> Result<Option<LeasedResource>>;

    /// List leased resources currently associated with a session.
    ///
    /// Session surfaces use this for visibility. Released resources remain
    /// visible so operators can inspect cleanup outcomes and failure history.
    async fn list_resources(&self, session_id: SessionId) -> Result<Vec<LeasedResource>>;
}

// ============================================================================
// ToolContext - Runtime context for tool execution
// ============================================================================

/// Type alias for the session SQL DB store trait object.
pub type SessionSqlDbStoreRef = Arc<dyn crate::session_sqldb::SessionSqlDbStore>;

/// Resolves user connection tokens (e.g. GitHub) lazily at tool execution time.
///
/// Instead of eagerly injecting tokens at session creation, tools call this
/// resolver when they need a token. If the user hasn't connected, returns None.
#[async_trait]
pub trait UserConnectionResolver: Send + Sync {
    /// Get a decrypted connection token for the given provider.
    /// Returns None if the user has no connection for this provider.
    async fn get_connection_token(
        &self,
        session_id: SessionId,
        provider: &str,
    ) -> Result<Option<String>>;

    /// Resolve the user ID of the connection used for a session/provider pair.
    ///
    /// This is used by leased resources to bind cleanup to the same provider
    /// identity that created the remote resource.
    async fn get_connection_user(
        &self,
        _session_id: SessionId,
        _provider: &str,
    ) -> Result<Option<Uuid>> {
        Ok(None)
    }

    /// Resolve a provider token for a specific user.
    ///
    /// Cleanup workers use this to avoid "first org member wins" behavior when
    /// cleaning resources created by a specific provider connection owner.
    async fn get_connection_token_for_user(
        &self,
        _user_id: Uuid,
        _provider: &str,
    ) -> Result<Option<String>> {
        Ok(None)
    }

    /// Get provider-specific metadata stored alongside the connection.
    /// Returns None if no metadata is stored or no connection exists.
    async fn get_connection_metadata(
        &self,
        _session_id: SessionId,
        _provider: &str,
    ) -> Result<Option<serde_json::Value>> {
        Ok(None)
    }
}

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
    pub session_id: SessionId,

    /// Optional file store for filesystem operations
    pub file_store: Option<Arc<dyn SessionFileStore>>,

    /// Optional storage store for key/value and secret storage
    pub storage_store: Option<Arc<dyn SessionStorageStore>>,

    /// Optional session SQL database store
    pub sqldb_store: Option<SessionSqlDbStoreRef>,

    /// Optional message retriever for tools that need conversation history access
    pub message_retriever: Option<Arc<dyn crate::message_retriever::MessageRetriever>>,

    /// Optional session store for tools that need session metadata access.
    pub session_store: Option<Arc<dyn SessionStore>>,

    /// Optional session mutator for tools that need to update session metadata.
    pub session_mutator: Option<Arc<dyn SessionMutator>>,

    /// Optional agent store for tools that need agent metadata access.
    pub agent_store: Option<Arc<dyn AgentStore>>,

    /// Optional resolver for user connection tokens (lazy GitHub token lookup, etc.)
    pub connection_resolver: Option<Arc<dyn UserConnectionResolver>>,

    /// Optional session schedule store for scheduling tools.
    pub schedule_store: Option<Arc<dyn SessionScheduleStore>>,

    /// Optional platform store for org-level management tools.
    pub platform_store: Option<Arc<dyn crate::platform_store::PlatformStore>>,
    /// Optional leased resource store for lifecycle-managed provider resources.
    pub leased_resource_store: Option<Arc<dyn LeasedResourceStore>>,

    /// Optional event emitter for tools that need to stream progress updates.
    /// When set, tools can emit `tool.progress` events during execution.
    pub event_emitter: Option<Arc<dyn EventEmitter>>,

    /// Event context for correlating progress events with the current tool call.
    /// Set by ActAtom when constructing the ToolContext.
    pub event_context: Option<crate::events::EventContext>,

    /// The tool call ID for the current execution (set by ActAtom).
    /// Used by tools to emit correlated progress events.
    pub tool_call_id: Option<String>,
    /// Optional capability registry for blueprint lookups.
    pub capability_registry: Option<crate::capabilities::CapabilityRegistry>,

    /// Optional memory store backend for persistent cross-session memory.
    pub memory_store: Option<Arc<dyn crate::memory_store::MemoryStoreBackend>>,

    /// Optional org ID for org-scoped operations (memory stores, etc.).
    pub org_id: Option<crate::typed_id::OrgId>,

    /// Merged network access list (harness ∩ agent ∩ session).
    /// When set, tools that make HTTP requests must check URLs against this list.
    pub network_access: Option<crate::network_access::NetworkAccessList>,
}

impl ToolContext {
    /// Create a new tool context with just a session ID
    pub fn new(session_id: SessionId) -> Self {
        Self {
            session_id,
            file_store: None,
            storage_store: None,
            sqldb_store: None,
            message_retriever: None,
            session_store: None,
            session_mutator: None,
            agent_store: None,
            connection_resolver: None,
            schedule_store: None,
            platform_store: None,
            leased_resource_store: None,
            event_emitter: None,
            event_context: None,
            tool_call_id: None,
            capability_registry: None,
            memory_store: None,
            org_id: None,
            network_access: None,
        }
    }

    /// Create a context with a file store
    pub fn with_file_store(session_id: SessionId, file_store: Arc<dyn SessionFileStore>) -> Self {
        Self {
            session_id,
            file_store: Some(file_store),
            storage_store: None,
            sqldb_store: None,
            message_retriever: None,
            session_store: None,
            session_mutator: None,
            agent_store: None,
            connection_resolver: None,
            schedule_store: None,
            platform_store: None,
            leased_resource_store: None,
            event_emitter: None,
            event_context: None,
            tool_call_id: None,
            capability_registry: None,
            memory_store: None,
            org_id: None,
            network_access: None,
        }
    }

    /// Create a context with a storage store
    pub fn with_storage_store(
        session_id: SessionId,
        storage_store: Arc<dyn SessionStorageStore>,
    ) -> Self {
        Self {
            session_id,
            file_store: None,
            storage_store: Some(storage_store),
            sqldb_store: None,
            message_retriever: None,
            session_store: None,
            session_mutator: None,
            agent_store: None,
            connection_resolver: None,
            schedule_store: None,
            platform_store: None,
            leased_resource_store: None,
            event_emitter: None,
            event_context: None,
            tool_call_id: None,
            capability_registry: None,
            memory_store: None,
            org_id: None,
            network_access: None,
        }
    }

    /// Create a context with both file store and storage store
    pub fn with_stores(
        session_id: SessionId,
        file_store: Arc<dyn SessionFileStore>,
        storage_store: Arc<dyn SessionStorageStore>,
    ) -> Self {
        Self {
            session_id,
            file_store: Some(file_store),
            storage_store: Some(storage_store),
            sqldb_store: None,
            message_retriever: None,
            session_store: None,
            session_mutator: None,
            agent_store: None,
            connection_resolver: None,
            schedule_store: None,
            platform_store: None,
            leased_resource_store: None,
            event_emitter: None,
            event_context: None,
            tool_call_id: None,
            capability_registry: None,
            memory_store: None,
            org_id: None,
            network_access: None,
        }
    }

    /// Add a SQL database store to this context
    pub fn with_sqldb_store(mut self, sqldb_store: SessionSqlDbStoreRef) -> Self {
        self.sqldb_store = Some(sqldb_store);
        self
    }

    /// Add a message retriever to this context
    pub fn with_message_retriever(
        mut self,
        retriever: Arc<dyn crate::message_retriever::MessageRetriever>,
    ) -> Self {
        self.message_retriever = Some(retriever);
        self
    }

    /// Add a session store to this context.
    pub fn with_session_store(mut self, store: Arc<dyn SessionStore>) -> Self {
        self.session_store = Some(store);
        self
    }

    /// Add a session mutator to this context.
    pub fn with_session_mutator(mut self, mutator: Arc<dyn SessionMutator>) -> Self {
        self.session_mutator = Some(mutator);
        self
    }

    /// Add an agent store to this context.
    pub fn with_agent_store(mut self, store: Arc<dyn AgentStore>) -> Self {
        self.agent_store = Some(store);
        self
    }

    /// Add a connection resolver to this context
    pub fn with_connection_resolver(mut self, resolver: Arc<dyn UserConnectionResolver>) -> Self {
        self.connection_resolver = Some(resolver);
        self
    }

    /// Add a session schedule store to this context.
    pub fn with_schedule_store(mut self, store: Arc<dyn SessionScheduleStore>) -> Self {
        self.schedule_store = Some(store);
        self
    }

    /// Add a platform store to this context.
    pub fn with_platform_store(
        mut self,
        store: Arc<dyn crate::platform_store::PlatformStore>,
    ) -> Self {
        self.platform_store = Some(store);
        self
    }

    /// Add a leased resource store to this context.
    pub fn with_leased_resource_store(mut self, store: Arc<dyn LeasedResourceStore>) -> Self {
        self.leased_resource_store = Some(store);
        self
    }

    /// Add a memory store backend for persistent cross-session memory.
    pub fn with_memory_store(
        mut self,
        store: Arc<dyn crate::memory_store::MemoryStoreBackend>,
    ) -> Self {
        self.memory_store = Some(store);
        self
    }

    /// Set org ID for org-scoped operations.
    pub fn with_org_id(mut self, org_id: crate::typed_id::OrgId) -> Self {
        self.org_id = Some(org_id);
        self
    }

    /// Set the merged network access list for URL filtering.
    pub fn with_network_access(
        mut self,
        network_access: Option<crate::network_access::NetworkAccessList>,
    ) -> Self {
        self.network_access = network_access;
        self
    }

    /// Emit a `tool.progress` event if an event emitter and context are available.
    ///
    /// This is a best-effort helper: failures are logged but not propagated,
    /// so tools never fail just because a progress event couldn't be sent.
    pub async fn emit_progress(&self, tool_name: &str, message: &str) {
        let (Some(emitter), Some(ctx), Some(call_id)) =
            (&self.event_emitter, &self.event_context, &self.tool_call_id)
        else {
            return;
        };
        if let Err(e) = emitter
            .emit(EventRequest::new(
                self.session_id,
                ctx.clone(),
                crate::events::ToolProgressData {
                    tool_call_id: call_id.clone(),
                    tool_name: tool_name.to_string(),
                    message: message.to_string(),
                    display_name: None,
                },
            ))
            .await
        {
            tracing::debug!(
                tool_call_id = call_id,
                tool_name,
                error = %e,
                "Failed to emit tool.progress event"
            );
        }
    }

    /// Emit a `tool.output.delta` event if an event emitter and context are available.
    ///
    /// Streams incremental output chunks (e.g., stdout/stderr lines) for live
    /// rendering in UI and CLI. Best-effort: failures are logged, not propagated.
    pub async fn emit_tool_output(&self, tool_name: &str, delta: &str, stream: &str) {
        let (Some(emitter), Some(ctx), Some(call_id)) =
            (&self.event_emitter, &self.event_context, &self.tool_call_id)
        else {
            return;
        };
        if let Err(e) = emitter
            .emit(EventRequest::new(
                self.session_id,
                ctx.clone(),
                crate::events::ToolOutputDeltaData {
                    tool_call_id: call_id.clone(),
                    tool_name: tool_name.to_string(),
                    delta: delta.to_string(),
                    stream: stream.to_string(),
                },
            ))
            .await
        {
            tracing::debug!(
                tool_call_id = call_id,
                tool_name,
                error = %e,
                "Failed to emit tool.output.delta event"
            );
        }
    }
}

impl std::fmt::Debug for ToolContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolContext")
            .field("session_id", &self.session_id)
            .field("file_store", &self.file_store.is_some())
            .field("storage_store", &self.storage_store.is_some())
            .field("sqldb_store", &self.sqldb_store.is_some())
            .field("message_retriever", &self.message_retriever.is_some())
            .field("session_store", &self.session_store.is_some())
            .field("session_mutator", &self.session_mutator.is_some())
            .field("agent_store", &self.agent_store.is_some())
            .field("connection_resolver", &self.connection_resolver.is_some())
            .field("schedule_store", &self.schedule_store.is_some())
            .field("platform_store", &self.platform_store.is_some())
            .field(
                "leased_resource_store",
                &self.leased_resource_store.is_some(),
            )
            .field("event_emitter", &self.event_emitter.is_some())
            .field("memory_store", &self.memory_store.is_some())
            .field("org_id", &self.org_id)
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

/// Blanket impl: `Arc<E>` delegates to the inner emitter.
#[async_trait]
impl<E: EventEmitter + ?Sized> EventEmitter for Arc<E> {
    async fn emit(&self, request: EventRequest) -> Result<Event> {
        (**self).emit(request).await
    }
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
        Ok(request.into_event(crate::typed_id::EventId::new(), 0))
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

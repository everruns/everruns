// Core traits for pluggable backends
//
// These traits allow the agent loop to be used with different backends:
// - In-memory implementations for examples and testing
// - Database implementations for production
// - Channel-based implementations for streaming

use crate::agent::Agent;
use crate::harness::Harness;
use crate::provider::DriverId;
use crate::session_file::{FileInfo, FileStat, GrepMatch, InitialFile, SessionFile};
use crate::tool_types::{ToolCall, ToolDefinition, ToolResult};
use crate::typed_id::{AgentId, HarnessId, ImageId, ModelId, SessionId, WorkspaceId};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::any::{Any, TypeId};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use uuid::Uuid;

fn workspace_display_path(path: &str) -> String {
    if path == "/" {
        "/workspace".to_string()
    } else if path.starts_with('/') {
        format!("/workspace{path}")
    } else {
        format!("/workspace/{path}")
    }
}

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

#[async_trait]
impl<T: AgentStore + ?Sized> AgentStore for std::sync::Arc<T> {
    async fn get_agent(&self, agent_id: AgentId) -> Result<Option<Agent>> {
        (**self).get_agent(agent_id).await
    }
}

// ============================================================================
// HarnessStore - For retrieving harness configurations
// ============================================================================

/// Trait for retrieving harness configurations
///
/// Implementations can:
/// - Load harnesses from a database
/// - Keep harnesses in memory for testing
///
/// Returns the harness inheritance chain (root-to-leaf) so the caller
/// can fold each harness as an `AgentConfigOverlay`. DB-backed stores
/// return the raw chain; gRPC-backed stores may return a single
/// pre-merged harness (functionally equivalent when folded).
#[async_trait]
pub trait HarnessStore: Send + Sync {
    /// Get the harness inheritance chain, root-to-leaf.
    ///
    /// Returns `Ok(vec![])` if the harness does not exist.
    /// A harness with no parent returns a single-element vec.
    async fn get_harness_chain(&self, harness_id: HarnessId) -> Result<Vec<Harness>>;
}

#[async_trait]
impl<T: HarnessStore + ?Sized> HarnessStore for std::sync::Arc<T> {
    async fn get_harness_chain(&self, harness_id: HarnessId) -> Result<Vec<Harness>> {
        (**self).get_harness_chain(harness_id).await
    }
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

#[async_trait]
impl<T: SessionStore + ?Sized> SessionStore for std::sync::Arc<T> {
    async fn get_session(&self, session_id: SessionId) -> Result<Option<Session>> {
        (**self).get_session(session_id).await
    }
}

/// Trait for updating mutable session metadata.
#[async_trait]
pub trait SessionMutator: Send + Sync {
    /// Update a session's human-readable title.
    async fn update_session_title(&self, session_id: SessionId, title: String) -> Result<Session>;
}

#[async_trait]
impl<T: SessionMutator + ?Sized> SessionMutator for std::sync::Arc<T> {
    async fn update_session_title(&self, session_id: SessionId, title: String) -> Result<Session> {
        (**self).update_session_title(session_id, title).await
    }
}

// ============================================================================
// ProviderStore - For retrieving LLM provider configurations
// ============================================================================

/// Model information with provider details needed for LLM calls
#[derive(Debug, Clone)]
pub struct ResolvedModel {
    /// The model ID string to pass to the LLM API (e.g., "gpt-4o", "claude-3-opus")
    pub model: String,
    /// Provider type for factory selection
    pub provider_type: DriverId,
    /// Decrypted API key (if configured)
    pub api_key: Option<String>,
    /// Optional base URL override
    pub base_url: Option<String>,
    /// Extra provider-specific metadata (OAuth tokens, account ids, etc.).
    /// Used by embedder-defined providers that authenticate without an API key.
    pub provider_metadata: Option<crate::driver_registry::ProviderMetadata>,
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
pub trait ProviderStore: Send + Sync {
    /// Get model with provider info by model ID
    ///
    /// Returns the model string ID, provider type, decrypted API key, and base URL
    /// needed to create an LLM provider via the factory.
    async fn get_resolved_model(&self, model_id: ModelId) -> Result<Option<ResolvedModel>>;

    /// Get the default model with provider info
    ///
    /// Returns the system default model when an agent has no default_model_id set.
    async fn get_default_model(&self) -> Result<Option<ResolvedModel>>;
}

#[async_trait]
impl<T: ProviderStore + ?Sized> ProviderStore for std::sync::Arc<T> {
    async fn get_resolved_model(&self, model_id: ModelId) -> Result<Option<ResolvedModel>> {
        (**self).get_resolved_model(model_id).await
    }

    async fn get_default_model(&self) -> Result<Option<ResolvedModel>> {
        (**self).get_default_model().await
    }
}

// ============================================================================
// ImageArtifactStore - For durable image persistence from tools
// ============================================================================

/// Metadata for a stored image artifact.
#[derive(Debug, Clone)]
pub struct StoredImageInfo {
    pub id: ImageId,
    pub filename: String,
    pub content_type: String,
    pub size_bytes: i64,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

/// Stored image artifact with binary data.
#[derive(Debug, Clone)]
pub struct StoredImage {
    pub info: StoredImageInfo,
    pub data: Vec<u8>,
}

/// Input for creating a stored image artifact.
#[derive(Debug, Clone)]
pub struct CreateStoredImage {
    pub filename: String,
    pub content_type: String,
    pub data: Vec<u8>,
    pub metadata: serde_json::Value,
}

#[async_trait]
pub trait ImageArtifactStore: Send + Sync {
    /// Persist an image artifact and return its durable metadata.
    async fn create_image(&self, input: CreateStoredImage) -> Result<StoredImageInfo>;

    /// Load a stored image artifact including bytes.
    async fn get_image(&self, image_id: ImageId) -> Result<Option<StoredImage>>;

    /// Load stored image metadata without binary data.
    async fn get_image_info(&self, image_id: ImageId) -> Result<Option<StoredImageInfo>>;
}

// ============================================================================
// ProviderCredentialStore - For tool-side provider credential resolution
// ============================================================================

/// Provider credentials resolved for tool-side API clients.
#[derive(Debug, Clone)]
pub struct ProviderCredentials {
    pub api_key: String,
    pub base_url: Option<String>,
}

#[async_trait]
pub trait ProviderCredentialStore: Send + Sync {
    /// Resolve default credentials for a provider type (for example `openai`).
    ///
    /// Implementations may apply environment fallbacks internally, but tools
    /// should never read provider env vars directly.
    async fn get_default_provider_credentials(
        &self,
        provider_type: &str,
    ) -> Result<Option<ProviderCredentials>>;
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

/// Delegating impl so callers can hold a `ToolExecutor` as a trait object
/// (e.g. to choose between a plain registry and an MCP-routing composite at
/// runtime without monomorphizing the consumer).
#[async_trait]
impl ToolExecutor for std::sync::Arc<dyn ToolExecutor> {
    async fn execute(&self, tool_call: &ToolCall, tool_def: &ToolDefinition) -> Result<ToolResult> {
        (**self).execute(tool_call, tool_def).await
    }

    async fn execute_with_context(
        &self,
        tool_call: &ToolCall,
        tool_def: &ToolDefinition,
        context: &ToolContext,
    ) -> Result<ToolResult> {
        (**self)
            .execute_with_context(tool_call, tool_def, context)
            .await
    }

    async fn execute_batch(
        &self,
        tool_calls: &[ToolCall],
        tool_defs: &[ToolDefinition],
    ) -> Result<Vec<ToolResult>> {
        (**self).execute_batch(tool_calls, tool_defs).await
    }
}

// ============================================================================
// SessionFileSystem - For session filesystem operations
// ============================================================================

/// Trait for session filesystem operations
///
/// This trait abstracts the session filesystem contract for tools and hosts.
/// Implementations can:
/// - Store files in a database (production)
/// - Use an in-memory filesystem for testing
/// - Project files onto real disk or object storage
#[async_trait]
pub trait SessionFileSystem: Send + Sync {
    /// Human-facing root path for this filesystem.
    ///
    /// `/workspace` remains the stable agent namespace, but embedded runtimes
    /// backed by a host directory can expose the real root here so shared
    /// capabilities can avoid misleading users about where files live.
    fn display_root(&self) -> String {
        "/workspace".to_string()
    }

    /// Convert a canonical session path into a human-facing path.
    fn display_path(&self, path: &str) -> String {
        workspace_display_path(path)
    }

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

    /// Seed a starter file into a session workspace.
    async fn seed_initial_file(&self, session_id: SessionId, file: &InitialFile) -> Result<()> {
        if file.is_readonly {
            return Err(crate::error::AgentLoopError::store(
                "read-only initial files require a SessionFileSystem-specific seed implementation",
            ));
        }
        self.write_file(session_id, &file.path, &file.content, &file.encoding)
            .await?;
        Ok(())
    }
}

/// A [`SessionFileSystem`] decorator that pins every operation to a fixed
/// workspace key, ignoring the per-call `session_id`.
///
/// Used to re-key file I/O for a session attached to a shared workspace (where
/// `workspace.id != session.id`): wrap the session's file store once with the
/// session's `workspace_id`, and all downstream capability/tool access then
/// addresses the attached workspace rather than the session's own keyspace. For
/// the default 1:1 session the key equals the session id, so the wrapper is a
/// transparent pass-through. See `specs/workspace.md`.
pub struct WorkspaceScopedFileSystem {
    inner: Arc<dyn SessionFileSystem>,
    key: SessionId,
}

impl WorkspaceScopedFileSystem {
    /// Wrap `inner`, pinning all operations to `workspace_id`'s key.
    pub fn wrap(
        inner: Arc<dyn SessionFileSystem>,
        workspace_id: WorkspaceId,
    ) -> Arc<dyn SessionFileSystem> {
        Arc::new(Self {
            inner,
            key: SessionId::from_uuid(workspace_id.uuid()),
        })
    }
}

#[async_trait]
impl SessionFileSystem for WorkspaceScopedFileSystem {
    async fn read_file(&self, _session_id: SessionId, path: &str) -> Result<Option<SessionFile>> {
        self.inner.read_file(self.key, path).await
    }
    async fn write_file(
        &self,
        _session_id: SessionId,
        path: &str,
        content: &str,
        encoding: &str,
    ) -> Result<SessionFile> {
        self.inner
            .write_file(self.key, path, content, encoding)
            .await
    }
    async fn write_file_if_content_matches(
        &self,
        _session_id: SessionId,
        path: &str,
        expected_content: &str,
        expected_encoding: &str,
        content: &str,
        encoding: &str,
    ) -> Result<Option<SessionFile>> {
        self.inner
            .write_file_if_content_matches(
                self.key,
                path,
                expected_content,
                expected_encoding,
                content,
                encoding,
            )
            .await
    }
    async fn delete_file(
        &self,
        _session_id: SessionId,
        path: &str,
        recursive: bool,
    ) -> Result<bool> {
        self.inner.delete_file(self.key, path, recursive).await
    }
    async fn list_directory(&self, _session_id: SessionId, path: &str) -> Result<Vec<FileInfo>> {
        self.inner.list_directory(self.key, path).await
    }
    async fn stat_file(&self, _session_id: SessionId, path: &str) -> Result<Option<FileStat>> {
        self.inner.stat_file(self.key, path).await
    }
    async fn grep_files(
        &self,
        _session_id: SessionId,
        pattern: &str,
        path_pattern: Option<&str>,
    ) -> Result<Vec<GrepMatch>> {
        self.inner.grep_files(self.key, pattern, path_pattern).await
    }
    async fn create_directory(&self, _session_id: SessionId, path: &str) -> Result<FileInfo> {
        self.inner.create_directory(self.key, path).await
    }
    async fn seed_initial_file(&self, _session_id: SessionId, file: &InitialFile) -> Result<()> {
        self.inner.seed_initial_file(self.key, file).await
    }

    fn display_root(&self) -> String {
        self.inner.display_root()
    }

    fn display_path(&self, path: &str) -> String {
        self.inner.display_path(path)
    }
}

#[async_trait]
impl<T: SessionFileSystem + ?Sized> SessionFileSystem for std::sync::Arc<T> {
    fn display_root(&self) -> String {
        (**self).display_root()
    }

    fn display_path(&self, path: &str) -> String {
        (**self).display_path(path)
    }

    async fn read_file(&self, session_id: SessionId, path: &str) -> Result<Option<SessionFile>> {
        (**self).read_file(session_id, path).await
    }

    async fn write_file(
        &self,
        session_id: SessionId,
        path: &str,
        content: &str,
        encoding: &str,
    ) -> Result<SessionFile> {
        (**self)
            .write_file(session_id, path, content, encoding)
            .await
    }

    async fn write_file_if_content_matches(
        &self,
        session_id: SessionId,
        path: &str,
        expected_content: &str,
        expected_encoding: &str,
        content: &str,
        encoding: &str,
    ) -> Result<Option<SessionFile>> {
        (**self)
            .write_file_if_content_matches(
                session_id,
                path,
                expected_content,
                expected_encoding,
                content,
                encoding,
            )
            .await
    }

    async fn delete_file(
        &self,
        session_id: SessionId,
        path: &str,
        recursive: bool,
    ) -> Result<bool> {
        (**self).delete_file(session_id, path, recursive).await
    }

    async fn list_directory(&self, session_id: SessionId, path: &str) -> Result<Vec<FileInfo>> {
        (**self).list_directory(session_id, path).await
    }

    async fn stat_file(&self, session_id: SessionId, path: &str) -> Result<Option<FileStat>> {
        (**self).stat_file(session_id, path).await
    }

    async fn grep_files(
        &self,
        session_id: SessionId,
        pattern: &str,
        path_pattern: Option<&str>,
    ) -> Result<Vec<GrepMatch>> {
        (**self).grep_files(session_id, pattern, path_pattern).await
    }

    async fn create_directory(&self, session_id: SessionId, path: &str) -> Result<FileInfo> {
        (**self).create_directory(session_id, path).await
    }

    async fn seed_initial_file(&self, session_id: SessionId, file: &InitialFile) -> Result<()> {
        (**self).seed_initial_file(session_id, file).await
    }
}

/// Backward-compatible alias for the old session filesystem trait name.
pub use SessionFileSystem as SessionFileStore;

/// Host-supplied values used by platform file-system factories.
///
/// The context is intentionally type-erased so `everruns-core` can own the
/// platform contract without depending on server-only types such as
/// `StorageBackend` or future object-storage clients.
#[derive(Clone, Default)]
pub struct SessionFileSystemFactoryContext {
    values: Arc<HashMap<TypeId, Arc<dyn Any + Send + Sync>>>,
}

impl SessionFileSystemFactoryContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with<T: Any + Send + Sync>(mut self, value: Arc<T>) -> Self {
        let values = Arc::make_mut(&mut self.values);
        values.insert(TypeId::of::<T>(), value);
        self
    }

    pub fn get<T: Any + Send + Sync>(&self) -> Option<Arc<T>> {
        self.values
            .get(&TypeId::of::<T>())
            .and_then(|value| value.clone().downcast::<T>().ok())
    }
}

/// Factory for deployment-selected session filesystem implementations.
#[async_trait]
pub trait SessionFileSystemFactory: Send + Sync {
    /// Human-readable factory name for diagnostics.
    fn name(&self) -> &'static str {
        "SessionFileSystemFactory"
    }

    /// Whether this factory intentionally leaves filesystem selection to the
    /// runtime default.
    fn is_disabled(&self) -> bool {
        false
    }

    /// Resolve a live filesystem from host-provided dependencies.
    async fn create_session_file_system(
        &self,
        context: SessionFileSystemFactoryContext,
    ) -> Result<Arc<dyn SessionFileSystem>>;
}

/// Default factory used when a platform does not configure session files.
#[derive(Debug, Clone, Default)]
pub struct DisabledSessionFileSystemFactory;

#[async_trait]
impl SessionFileSystemFactory for DisabledSessionFileSystemFactory {
    fn name(&self) -> &'static str {
        "DisabledSessionFileSystemFactory"
    }

    fn is_disabled(&self) -> bool {
        true
    }

    async fn create_session_file_system(
        &self,
        _context: SessionFileSystemFactoryContext,
    ) -> Result<Arc<dyn SessionFileSystem>> {
        Err(crate::error::AgentLoopError::config(
            "session filesystem is disabled",
        ))
    }
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
// SessionResourceRegistry - Generic session-scoped resource registry
// ============================================================================

/// Generic registry of resources active alongside a session.
///
/// Capabilities register resources here (sandboxes, subagents, browser sessions).
/// Agents query it ("what's running?"), infrastructure scans it for cleanup.
/// See `specs/session-resources.md`.
#[async_trait]
pub trait SessionResourceRegistry: Send + Sync {
    /// Register a resource (or update if resource_id already exists for this session).
    async fn register(
        &self,
        entry: crate::session_resource::RegisterSessionResource,
    ) -> Result<crate::session_resource::SessionResourceEntry>;

    /// Update the status of a registered resource.
    async fn update_status(
        &self,
        session_id: SessionId,
        resource_id: &str,
        status: crate::session_resource::SessionResourceStatus,
    ) -> Result<Option<crate::session_resource::SessionResourceEntry>>;

    /// Get a specific resource by ID.
    async fn get(
        &self,
        session_id: SessionId,
        resource_id: &str,
    ) -> Result<Option<crate::session_resource::SessionResourceEntry>>;

    /// List resources for a session, optionally filtered.
    async fn list(
        &self,
        session_id: SessionId,
        filter: Option<&crate::session_resource::SessionResourceFilter>,
    ) -> Result<Vec<crate::session_resource::SessionResourceEntry>>;

    /// Remove a resource from the registry.
    async fn deregister(&self, session_id: SessionId, resource_id: &str) -> Result<bool>;
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

// ============================================================================
// BudgetChecker - For querying budget status from tools
// ============================================================================

/// Trait for checking budget status from within tool execution.
///
/// Implemented by gRPC adapters (worker → server) and direct adapters (in-process).
/// Used by the `check_budget` tool to return real budget data to agents.
/// The org_id is captured at construction time by the implementing adapter.
#[async_trait]
pub trait BudgetChecker: Send + Sync {
    /// Check all budgets for a session and return a tool-friendly response.
    async fn check_budgets(&self, session_id: &str) -> Result<crate::budget::BudgetToolResponse>;
}

// ============================================================================
// PaymentAuthority - For capability-internal machine payments
// ============================================================================

/// Internal authority for paid capability operations.
///
/// Capabilities call this with fixed, typed requests. The model never receives a
/// generic paid HTTP tool, wallet credentials, or payment payloads.
#[async_trait]
pub trait PaymentAuthority: Send + Sync {
    async fn execute_machine_payment(
        &self,
        session_id: SessionId,
        request: crate::payment::MachinePaymentRequest,
    ) -> Result<crate::payment::MachinePaymentResponse>;
}

// OutboundToolRateLimiter - Per-org outbound tool-call rate limiting (TM-TOOL-009)
// ============================================================================

/// Per-org gate on outbound tool execution.
///
/// Returns `true` if the call is within the per-org budget, `false` if the
/// org has exceeded its outbound tool rate limit for this window.
/// Implementations must be fail-open: Valkey/backend errors should return `true`
/// rather than blocking legitimate tool calls.
#[async_trait]
pub trait OutboundToolRateLimiter: Send + Sync {
    /// Key by the public org UUID (keyed string representation).
    async fn check_org(&self, org_id: &crate::typed_id::OrgId) -> bool;
}

// ============================================================================
// DurableToolResultStore — per-tool-call idempotency (EVE-530)
// ============================================================================

/// Result of a claim attempt on the per-tool-call idempotency store.
#[derive(Debug)]
pub enum ToolCallClaimResult {
    /// First claim for this (turn_id, tool_call_id); caller should execute the tool.
    /// `claim_token` must be passed to `settle_tool_call` to verify ownership.
    Claimed { claim_token: uuid::Uuid },
    /// A prior run already settled this call; replay the stored result.
    AlreadySettled {
        result_json: serde_json::Value,
        args_fingerprint: String,
    },
    /// A prior run started but never settled. For `AtMostOnce` tools the
    /// caller should NOT re-execute; for `Pure`/`Idempotent` tools the caller
    /// may re-execute and then try to settle (the settle CAS will be a no-op if
    /// a different claimer wins first).
    AlreadyRunning { args_fingerprint: String },
    /// A settled row exists but its `args_fingerprint` does not match the
    /// current call — this is a determinism violation (workflow replay with
    /// different inputs). The workflow should be failed loudly.
    DeterminismViolation {
        stored_fingerprint: String,
        current_fingerprint: String,
    },
}

/// Read-only status of a tool call in durable storage (EVE-533).
#[derive(Debug, Clone)]
pub enum DurableToolCallStatus {
    /// Tool completed successfully or with an error; result is stored.
    Settled { result_json: serde_json::Value },
    /// Tool was settled with `interrupted` status; result may contain error details.
    Interrupted {
        result_json: Option<serde_json::Value>,
    },
    /// A claim exists but the tool never finished.
    Running,
}

/// Durable per-tool-call idempotency store (EVE-530).
///
/// Implements the claim/settle CAS that prevents double-execution of
/// `AtMostOnce` tools on worker reclaim/replay.
#[async_trait]
pub trait DurableToolResultStore: Send + Sync + 'static {
    /// Atomically claim `(turn_id, tool_call_id)` before tool dispatch.
    ///
    /// - Inserts a `running` row if none exists → `Claimed`.
    /// - Finds an existing `settled` row → `AlreadySettled`.
    /// - Finds an existing `running` row → `AlreadyRunning`.
    /// - Finds a `settled` row with a mismatched `args_fingerprint`
    ///   (determinism violation) → `DeterminismViolation`.
    async fn try_claim_tool_call(
        &self,
        turn_id: &str,
        tool_call_id: &str,
        tool_name: &str,
        args_fingerprint: &str,
    ) -> Result<ToolCallClaimResult>;

    /// Settle a previously claimed tool call with its result.
    ///
    /// `claim_token` must match the token returned by `try_claim_tool_call`.
    /// Returns `Ok(true)` if the row was updated, `Ok(false)` if the claim
    /// token no longer matches (ownership lost — treat as a warning).
    async fn settle_tool_call(
        &self,
        turn_id: &str,
        tool_call_id: &str,
        result_json: serde_json::Value,
        status: &str,
        claim_token: uuid::Uuid,
    ) -> Result<bool>;

    /// Read-only lookup of a tool call's current status in durable storage (EVE-533).
    ///
    /// Used by transcript repair to decide whether to replay a stored result or
    /// synthesize an interrupted placeholder. Returns `None` if no row exists.
    async fn get_tool_call_status(
        &self,
        turn_id: &str,
        tool_call_id: &str,
    ) -> Result<Option<DurableToolCallStatus>>;
}

/// No-op implementation — used when no durable store is configured (dev/test).
/// Every call is treated as a fresh first execution; no replay or ownership checks.
pub struct NoopDurableToolResultStore;

#[async_trait]
impl DurableToolResultStore for NoopDurableToolResultStore {
    async fn try_claim_tool_call(
        &self,
        _turn_id: &str,
        _tool_call_id: &str,
        _tool_name: &str,
        _args_fingerprint: &str,
    ) -> Result<ToolCallClaimResult> {
        Ok(ToolCallClaimResult::Claimed {
            claim_token: uuid::Uuid::new_v4(),
        })
    }

    async fn settle_tool_call(
        &self,
        _turn_id: &str,
        _tool_call_id: &str,
        _result_json: serde_json::Value,
        _status: &str,
        _claim_token: uuid::Uuid,
    ) -> Result<bool> {
        Ok(true)
    }

    async fn get_tool_call_status(
        &self,
        _turn_id: &str,
        _tool_call_id: &str,
    ) -> Result<Option<DurableToolCallStatus>> {
        Ok(None)
    }
}

// ============================================================================
// StreamHeartbeater — per-stream liveness signal for Reason activity (EVE-531)
// ============================================================================

/// Progress snapshot carried in each stream heartbeat.
#[derive(Debug, Clone)]
pub struct StreamProgress {
    /// Accumulated text + thinking length (characters) at the time of heartbeat.
    pub accumulated_len: usize,
    /// Wall-clock time of the most recent received token (Unix seconds).
    pub last_delta_at: u64,
}

/// Heartbeater the Reason streaming loop calls on delta batches and a keepalive
/// timer, signalling that the provider connection is alive.
///
/// Implementations bridge to the durable-execution layer (e.g. gRPC).
/// The no-op is used in dev/test where no durable store is present.
#[async_trait]
pub trait StreamHeartbeater: Send + Sync {
    /// Signal stream liveness with current progress.
    ///
    /// Must be best-effort: errors must not propagate to the caller.
    /// Cancel-safety is critical — if the worker dies the heartbeat stops
    /// and the existing task-level reclaim takes over.
    async fn heartbeat(&self, progress: StreamProgress);
}

/// No-op heartbeater — treats every stream as perpetually alive (dev/test).
pub struct NoopStreamHeartbeater;

#[async_trait]
impl StreamHeartbeater for NoopStreamHeartbeater {
    async fn heartbeat(&self, _progress: StreamProgress) {}
}

// ============================================================================
// PartialStreamStore — partial-stream recovery for Reason activity (EVE-532)
// ============================================================================

/// State of a partially-streamed assistant message detected in the event log.
#[derive(Debug, Clone)]
pub struct PartialStreamState {
    /// Accumulated text from the last `output.message.delta` for the turn.
    /// Empty when `output.message.started` was emitted but no delta arrived.
    pub accumulated: String,
}

/// Consults the persisted event log to detect whether a `reason` activity
/// was interrupted after `output.message.started` but before
/// `output.message.completed` or `output.message.replaced`.
///
/// Used by `ReasonAtom` on re-entry to apply the ContinuePartial recovery
/// policy (EVE-532): finalize the partial text without a second provider call,
/// or restart clean if the partial is unusable.
#[async_trait]
pub trait PartialStreamStore: Send + Sync {
    /// Return the partial-stream state for `(session_id, turn_id)` if an
    /// in-flight assistant message exists (started but not completed).
    async fn get_partial_stream(
        &self,
        session_id: SessionId,
        turn_id: &str,
    ) -> Result<Option<PartialStreamState>>;
}

/// No-op — always reports no partial stream (dev/test / in-memory mode).
pub struct NoopPartialStreamStore;

#[async_trait]
impl PartialStreamStore for NoopPartialStreamStore {
    async fn get_partial_stream(
        &self,
        _session_id: SessionId,
        _turn_id: &str,
    ) -> Result<Option<PartialStreamState>> {
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
    /// The workspace this session is attached to — the key for the virtual
    /// file store. For the default 1:1 session this equals
    /// `WorkspaceId::from_uuid(session_id.uuid())`; for a shared workspace it
    /// differs. File-system tools MUST key by this (via `workspace_fs_key`)
    /// rather than `session_id` so shared-workspace sessions read/write the
    /// attached workspace's files. See specs/workspace.md.
    pub workspace_id: WorkspaceId,

    /// Optional file store for filesystem operations
    pub file_store: Option<Arc<dyn SessionFileSystem>>,

    /// Optional storage store for key/value and secret storage
    pub storage_store: Option<Arc<dyn SessionStorageStore>>,

    /// Optional durable image artifact store for tool-side media persistence.
    pub image_store: Option<Arc<dyn ImageArtifactStore>>,

    /// Optional provider credential store for tool-side API clients.
    pub provider_credential_store: Option<Arc<dyn ProviderCredentialStore>>,

    /// Optional system utility LLM service for capability internals.
    pub utility_llm_service: Option<Arc<dyn crate::UtilityLlmService>>,

    /// Optional outbound egress service for HTTP/API traffic.
    pub egress_service: Option<Arc<dyn crate::EgressService>>,

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

    /// Optional hybrid retrieval over bound Knowledge Indexes for the
    /// `search_index` tool. Server-implemented; populated only on the server
    /// act path alongside `platform_store` / `connection_resolver`.
    pub knowledge_index_search: Option<Arc<dyn crate::vector_store::KnowledgeIndexSearch>>,

    /// Optional leased resource store for lifecycle-managed provider resources.
    pub leased_resource_store: Option<Arc<dyn LeasedResourceStore>>,

    /// Optional session resource registry — generic registry of active resources.
    pub session_resource_registry: Option<Arc<dyn SessionResourceRegistry>>,

    /// Optional session task registry — background work owned by the session
    /// (specs/session-tasks.md).
    pub session_task_registry: Option<Arc<dyn crate::session_task::SessionTaskRegistry>>,

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

    /// Optional registry of active built-in tools for meta-tools such as
    /// `spawn_background` that need to inspect or delegate to sibling tools.
    pub tool_registry: Option<Arc<crate::tools::ToolRegistry>>,

    /// Optional allowlist of tools visible to the model for this turn.
    /// Registry-introspecting tools must filter through this before returning
    /// sibling tool metadata, because the execution registry can be a superset.
    pub visible_tool_names: Option<Arc<HashSet<String>>>,

    /// Optional org ID for org-scoped operations.
    pub org_id: Option<crate::typed_id::OrgId>,

    /// Merged network access list (harness ∩ agent ∩ session).
    /// When set, tools that make HTTP requests must check URLs against this list.
    pub network_access: Option<crate::network_access::NetworkAccessList>,

    /// Resolved locale for localized tool behavior (BCP 47, e.g. `uk-UA`).
    /// When set, tools that support localization use this to produce
    /// locale-appropriate descriptions, error messages, and prompts.
    pub locale: Option<String>,

    /// Optional budget checker for the check_budget tool.
    pub budget_checker: Option<Arc<dyn BudgetChecker>>,

    /// Optional internal payment authority for paid capability tools.
    pub payment_authority: Option<Arc<dyn PaymentAuthority>>,

    /// Optional durable spawn handle store for subagent reattach (EVE-535).
    /// When set, `spawn_subagent` uses claim/settle to prevent duplicate spawning
    /// on parent worker reclaim.
    pub subagent_spawn_store: Option<Arc<dyn SubagentSpawnStore>>,
}

impl ToolContext {
    /// The virtual-file-store key for this execution, derived from the attached
    /// workspace. Carried through the `SessionFileSystem` trait's `SessionId`
    /// parameter (the store keys by `.uuid()`), so a shared-workspace session
    /// addresses the workspace's files rather than its own session-id keyspace.
    pub fn workspace_fs_key(&self) -> SessionId {
        SessionId::from_uuid(self.workspace_id.uuid())
    }

    /// Override the attached workspace (default is the 1:1 session-derived id).
    pub fn with_workspace_id(mut self, workspace_id: WorkspaceId) -> Self {
        self.workspace_id = workspace_id;
        self
    }

    /// Create a new tool context with just a session ID
    pub fn new(session_id: SessionId) -> Self {
        Self {
            session_id,
            workspace_id: WorkspaceId::from_uuid(session_id.uuid()),
            file_store: None,
            storage_store: None,
            image_store: None,
            provider_credential_store: None,
            utility_llm_service: None,
            egress_service: None,
            sqldb_store: None,
            message_retriever: None,
            session_store: None,
            session_mutator: None,
            agent_store: None,
            connection_resolver: None,
            schedule_store: None,
            platform_store: None,
            knowledge_index_search: None,
            leased_resource_store: None,
            session_resource_registry: None,
            session_task_registry: None,
            event_emitter: None,
            event_context: None,
            tool_call_id: None,
            capability_registry: None,
            tool_registry: None,
            visible_tool_names: None,
            org_id: None,
            network_access: None,
            locale: None,
            budget_checker: None,
            payment_authority: None,
            subagent_spawn_store: None,
        }
    }

    /// Create a context with a file store
    pub fn with_file_store(session_id: SessionId, file_store: Arc<dyn SessionFileSystem>) -> Self {
        Self {
            session_id,
            workspace_id: WorkspaceId::from_uuid(session_id.uuid()),
            file_store: Some(file_store),
            storage_store: None,
            image_store: None,
            provider_credential_store: None,
            utility_llm_service: None,
            egress_service: None,
            sqldb_store: None,
            message_retriever: None,
            session_store: None,
            session_mutator: None,
            agent_store: None,
            connection_resolver: None,
            schedule_store: None,
            platform_store: None,
            knowledge_index_search: None,
            leased_resource_store: None,
            session_resource_registry: None,
            session_task_registry: None,
            event_emitter: None,
            event_context: None,
            tool_call_id: None,
            capability_registry: None,
            tool_registry: None,
            visible_tool_names: None,
            org_id: None,
            network_access: None,
            locale: None,
            budget_checker: None,
            payment_authority: None,
            subagent_spawn_store: None,
        }
    }

    /// Create a context with a storage store
    pub fn with_storage_store(
        session_id: SessionId,
        storage_store: Arc<dyn SessionStorageStore>,
    ) -> Self {
        Self {
            session_id,
            workspace_id: WorkspaceId::from_uuid(session_id.uuid()),
            file_store: None,
            storage_store: Some(storage_store),
            image_store: None,
            provider_credential_store: None,
            utility_llm_service: None,
            egress_service: None,
            sqldb_store: None,
            message_retriever: None,
            session_store: None,
            session_mutator: None,
            agent_store: None,
            connection_resolver: None,
            schedule_store: None,
            platform_store: None,
            knowledge_index_search: None,
            leased_resource_store: None,
            session_resource_registry: None,
            session_task_registry: None,
            event_emitter: None,
            event_context: None,
            tool_call_id: None,
            capability_registry: None,
            tool_registry: None,
            visible_tool_names: None,
            org_id: None,
            network_access: None,
            locale: None,
            budget_checker: None,
            payment_authority: None,
            subagent_spawn_store: None,
        }
    }

    /// Create a context with both file store and storage store
    pub fn with_stores(
        session_id: SessionId,
        file_store: Arc<dyn SessionFileSystem>,
        storage_store: Arc<dyn SessionStorageStore>,
    ) -> Self {
        Self {
            session_id,
            workspace_id: WorkspaceId::from_uuid(session_id.uuid()),
            file_store: Some(file_store),
            storage_store: Some(storage_store),
            sqldb_store: None,
            image_store: None,
            provider_credential_store: None,
            utility_llm_service: None,
            egress_service: None,
            message_retriever: None,
            session_store: None,
            session_mutator: None,
            agent_store: None,
            connection_resolver: None,
            schedule_store: None,
            platform_store: None,
            knowledge_index_search: None,
            leased_resource_store: None,
            session_resource_registry: None,
            session_task_registry: None,
            event_emitter: None,
            event_context: None,
            tool_call_id: None,
            capability_registry: None,
            tool_registry: None,
            visible_tool_names: None,
            org_id: None,
            network_access: None,
            locale: None,
            budget_checker: None,
            payment_authority: None,
            subagent_spawn_store: None,
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

    /// Create a context with an image artifact store.
    pub fn with_image_store(
        session_id: SessionId,
        image_store: Arc<dyn ImageArtifactStore>,
    ) -> Self {
        Self {
            session_id,
            workspace_id: WorkspaceId::from_uuid(session_id.uuid()),
            file_store: None,
            storage_store: None,
            image_store: Some(image_store),
            provider_credential_store: None,
            utility_llm_service: None,
            egress_service: None,
            sqldb_store: None,
            message_retriever: None,
            session_store: None,
            session_mutator: None,
            agent_store: None,
            connection_resolver: None,
            schedule_store: None,
            platform_store: None,
            knowledge_index_search: None,
            leased_resource_store: None,
            session_resource_registry: None,
            session_task_registry: None,
            event_emitter: None,
            event_context: None,
            tool_call_id: None,
            capability_registry: None,
            tool_registry: None,
            visible_tool_names: None,
            org_id: None,
            network_access: None,
            locale: None,
            budget_checker: None,
            payment_authority: None,
            subagent_spawn_store: None,
        }
    }

    /// Set the provider credential store on this context.
    pub fn with_provider_credential_store(
        mut self,
        store: Arc<dyn ProviderCredentialStore>,
    ) -> Self {
        self.provider_credential_store = Some(store);
        self
    }

    /// Set the utility LLM service on this context.
    pub fn with_utility_llm_service(mut self, service: Arc<dyn crate::UtilityLlmService>) -> Self {
        self.utility_llm_service = Some(service);
        self
    }

    /// Set the outbound egress service on this context.
    pub fn with_egress_service(mut self, service: Arc<dyn crate::EgressService>) -> Self {
        self.egress_service = Some(service);
        self
    }

    /// Set the outbound egress service on this context when available.
    /// Preserves any already-set service when `service` is `None`.
    pub fn with_egress_service_opt(
        mut self,
        service: Option<Arc<dyn crate::EgressService>>,
    ) -> Self {
        if let Some(service) = service {
            self.egress_service = Some(service);
        }
        self
    }

    /// Set the session storage store on this context (builder method).
    pub fn with_storage_store_arc(mut self, store: Arc<dyn SessionStorageStore>) -> Self {
        self.storage_store = Some(store);
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

    /// Add a Knowledge Index search service to this context (for `search_index`).
    pub fn with_knowledge_index_search(
        mut self,
        search: Arc<dyn crate::vector_store::KnowledgeIndexSearch>,
    ) -> Self {
        self.knowledge_index_search = Some(search);
        self
    }

    /// Add a leased resource store to this context.
    pub fn with_leased_resource_store(mut self, store: Arc<dyn LeasedResourceStore>) -> Self {
        self.leased_resource_store = Some(store);
        self
    }

    /// Add a session resource registry to this context.
    pub fn with_session_resource_registry(
        mut self,
        registry: Arc<dyn SessionResourceRegistry>,
    ) -> Self {
        self.session_resource_registry = Some(registry);
        self
    }

    /// Add a session task registry to this context.
    pub fn with_session_task_registry(
        mut self,
        registry: Arc<dyn crate::session_task::SessionTaskRegistry>,
    ) -> Self {
        self.session_task_registry = Some(registry);
        self
    }

    /// Set org ID for org-scoped operations.
    pub fn with_org_id(mut self, org_id: crate::typed_id::OrgId) -> Self {
        self.org_id = Some(org_id);
        self
    }

    /// Set the active built-in tool registry on this context.
    pub fn with_tool_registry(mut self, registry: Arc<crate::tools::ToolRegistry>) -> Self {
        self.tool_registry = Some(registry);
        self
    }

    /// Set the tool names visible to the model in this turn.
    pub fn with_visible_tool_names(mut self, names: Arc<HashSet<String>>) -> Self {
        self.visible_tool_names = Some(names);
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

    /// Set the internal payment authority for paid capability operations.
    pub fn with_payment_authority(mut self, authority: Arc<dyn PaymentAuthority>) -> Self {
        self.payment_authority = Some(authority);
        self
    }

    /// Set the durable subagent spawn handle store (EVE-535).
    pub fn with_subagent_spawn_store(mut self, store: Arc<dyn SubagentSpawnStore>) -> Self {
        self.subagent_spawn_store = Some(store);
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
            .field("image_store", &self.image_store.is_some())
            .field(
                "provider_credential_store",
                &self.provider_credential_store.is_some(),
            )
            .field("utility_llm_service", &self.utility_llm_service.is_some())
            .field("egress_service", &self.egress_service.is_some())
            .field("sqldb_store", &self.sqldb_store.is_some())
            .field("message_retriever", &self.message_retriever.is_some())
            .field("session_store", &self.session_store.is_some())
            .field("session_mutator", &self.session_mutator.is_some())
            .field("agent_store", &self.agent_store.is_some())
            .field("connection_resolver", &self.connection_resolver.is_some())
            .field("schedule_store", &self.schedule_store.is_some())
            .field("platform_store", &self.platform_store.is_some())
            .field(
                "knowledge_index_search",
                &self.knowledge_index_search.is_some(),
            )
            .field(
                "leased_resource_store",
                &self.leased_resource_store.is_some(),
            )
            .field("event_emitter", &self.event_emitter.is_some())
            .field("tool_registry", &self.tool_registry.is_some())
            .field("payment_authority", &self.payment_authority.is_some())
            .field("subagent_spawn_store", &self.subagent_spawn_store.is_some())
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
// SubagentSpawnStore — durable spawn handles for subagent reattach (EVE-535)
// ============================================================================

/// Result of attempting to claim a subagent spawn slot.
#[derive(Debug)]
pub enum SpawnClaimResult {
    /// First claim — child session does not yet exist.
    /// Proceed to create the child, then call `register_child_session`.
    Claimed {
        spawn_handle_id: uuid::Uuid,
        claim_token: uuid::Uuid,
    },
    /// Row exists but `child_session_id` was never registered (crash between
    /// claim and `register_child_session`). Re-create the child and call
    /// `register_child_session` — same flow as `Claimed`.
    ClaimedPendingChild {
        spawn_handle_id: uuid::Uuid,
        claim_token: uuid::Uuid,
    },
    /// Child session was created and is still running.
    /// Reattach: wait for the existing child and settle with the stored claim_token.
    AlreadyRunning {
        child_session_id: crate::typed_id::SessionId,
        /// Stored claim token — must be used for `settle_spawn` on this replay.
        claim_token: uuid::Uuid,
    },
    /// Child already finished on a previous execution.
    /// Fast-path: return the stored result immediately without waiting.
    AlreadySettled {
        child_session_id: crate::typed_id::SessionId,
        /// The `wait_for_idle` return value from the original execution.
        terminal_status: String,
        terminal_result: String,
    },
}

/// Durable spawn handle store for subagent idempotency (EVE-535).
///
/// Maps `(parent_session_id, tool_call_id) → child_session_id` so that when
/// a parent's `act` is reclaimed mid-`wait_for_idle`, the tool can reattach
/// to the existing child instead of spawning a duplicate.
///
/// Lifecycle: claim → register_child_session → settle_spawn.
#[async_trait]
pub trait SubagentSpawnStore: Send + Sync + 'static {
    /// Attempt to claim a spawn slot for `(parent_session_id, tool_call_id)`.
    ///
    /// Does NOT accept `child_session_id` — the child session does not exist yet.
    /// Call `register_child_session` with the actual child ID after creating it.
    async fn try_claim_spawn(
        &self,
        parent_session_id: crate::typed_id::SessionId,
        tool_call_id: &str,
        claim_token: uuid::Uuid,
    ) -> Result<SpawnClaimResult>;

    /// Register the actual child session ID after it has been created.
    ///
    /// Must be called after `try_claim_spawn` returns `Claimed` or
    /// `ClaimedPendingChild`, before waiting for the child to complete.
    async fn register_child_session(
        &self,
        spawn_handle_id: uuid::Uuid,
        claim_token: uuid::Uuid,
        child_session_id: crate::typed_id::SessionId,
    ) -> Result<()>;

    /// Record the terminal result once the child has completed.
    ///
    /// `claim_token` must match the stored token. `terminal_status` is the
    /// `wait_for_idle` return value ("idle", "error", "timeout", etc.) and
    /// `terminal_result` is the last agent message.
    async fn settle_spawn(
        &self,
        parent_session_id: crate::typed_id::SessionId,
        tool_call_id: &str,
        claim_token: uuid::Uuid,
        terminal_status: &str,
        terminal_result: &str,
    ) -> Result<()>;
}

/// Blanket impl: `Arc<S>` delegates to the inner store.
#[async_trait]
impl<S: SubagentSpawnStore + ?Sized> SubagentSpawnStore for Arc<S> {
    async fn try_claim_spawn(
        &self,
        parent_session_id: crate::typed_id::SessionId,
        tool_call_id: &str,
        claim_token: uuid::Uuid,
    ) -> Result<SpawnClaimResult> {
        (**self)
            .try_claim_spawn(parent_session_id, tool_call_id, claim_token)
            .await
    }

    async fn register_child_session(
        &self,
        spawn_handle_id: uuid::Uuid,
        claim_token: uuid::Uuid,
        child_session_id: crate::typed_id::SessionId,
    ) -> Result<()> {
        (**self)
            .register_child_session(spawn_handle_id, claim_token, child_session_id)
            .await
    }

    async fn settle_spawn(
        &self,
        parent_session_id: crate::typed_id::SessionId,
        tool_call_id: &str,
        claim_token: uuid::Uuid,
        terminal_status: &str,
        terminal_result: &str,
    ) -> Result<()> {
        (**self)
            .settle_spawn(
                parent_session_id,
                tool_call_id,
                claim_token,
                terminal_status,
                terminal_result,
            )
            .await
    }
}

/// No-op spawn store — used when no durable store is configured (dev/test).
///
/// Always claims (no dedup); settle and register are no-ops.
pub struct NoopSubagentSpawnStore;

#[async_trait]
impl SubagentSpawnStore for NoopSubagentSpawnStore {
    async fn try_claim_spawn(
        &self,
        _parent_session_id: crate::typed_id::SessionId,
        _tool_call_id: &str,
        claim_token: uuid::Uuid,
    ) -> Result<SpawnClaimResult> {
        Ok(SpawnClaimResult::Claimed {
            spawn_handle_id: uuid::Uuid::new_v4(),
            claim_token,
        })
    }

    async fn register_child_session(
        &self,
        _spawn_handle_id: uuid::Uuid,
        _claim_token: uuid::Uuid,
        _child_session_id: crate::typed_id::SessionId,
    ) -> Result<()> {
        Ok(())
    }

    async fn settle_spawn(
        &self,
        _parent_session_id: crate::typed_id::SessionId,
        _tool_call_id: &str,
        _claim_token: uuid::Uuid,
        _terminal_status: &str,
        _terminal_result: &str,
    ) -> Result<()> {
        Ok(())
    }
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

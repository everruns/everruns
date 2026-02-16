// Worker adapters trait for unified worker implementation
//
// Decision: Single trait abstracts all data operations needed by activities
// Decision: Implementations for gRPC (external workers) and Direct (in-process)
//
// This allows a single Worker implementation to work with either backend.

use async_trait::async_trait;
use everruns_core::capabilities::CapabilityRegistry;
use everruns_core::error::Result;
use everruns_core::events::{Event, EventRequest};
use everruns_core::session_file::{FileInfo, FileStat, GrepMatch, SessionFile};
use everruns_core::traits::{ImageResolver, ResolvedImage};
use everruns_core::typed_id::{AgentId, HarnessId, MessageId, ModelId, SessionId};
use everruns_core::{
    Agent, DriverRegistry, Harness, LlmProviderType, Message, Session, ToolDefinition, ToolRegistry,
};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

use crate::mcp_executor::McpServerInfo;

// =============================================================================
// WorkerAdapters Trait
// =============================================================================

/// Unified adapter trait for worker data operations
///
/// This trait abstracts all data access operations needed by worker activities,
/// allowing the same activity logic to work with either:
/// - gRPC adapters (for external workers)
/// - Direct adapters (for in-process workers)
#[async_trait]
pub trait WorkerAdapters: Send + Sync + Clone + 'static {
    // =========================================================================
    // Agent Operations
    // =========================================================================

    /// Get agent by ID
    async fn get_agent(&self, org_id: i64, agent_id: Uuid) -> Result<Option<Agent>>;

    /// Get harness by ID
    async fn get_harness(&self, org_id: i64, harness_id: Uuid) -> Result<Option<Harness>>;

    // =========================================================================
    // Session Operations
    // =========================================================================

    /// Get session by ID
    async fn get_session(&self, org_id: i64, session_id: Uuid) -> Result<Option<Session>>;

    /// Set session status (started, active, idle)
    async fn set_session_status(
        &self,
        org_id: i64,
        session_id: Uuid,
        status: &str,
    ) -> Result<Session>;

    // =========================================================================
    // Message Operations
    // =========================================================================

    /// Get a specific message by ID
    async fn get_message(&self, session_id: Uuid, message_id: Uuid) -> Result<Option<Message>>;

    /// Load all messages for a session
    async fn load_messages(&self, session_id: Uuid) -> Result<Vec<Message>>;

    // =========================================================================
    // Event Operations
    // =========================================================================

    /// Emit an event
    async fn emit_event(&self, request: EventRequest) -> Result<Event>;

    // =========================================================================
    // LLM Provider Operations
    // =========================================================================

    /// Get model with provider configuration
    async fn get_model_with_provider(
        &self,
        org_id: i64,
        model_id: Uuid,
    ) -> Result<Option<ModelWithProvider>>;

    /// Get default model configuration
    async fn get_default_model(&self, org_id: i64) -> Result<Option<ModelWithProvider>>;

    // =========================================================================
    // Image Resolution Operations
    // =========================================================================

    /// Resolve a single image by ID
    async fn resolve_image(&self, image_id: Uuid) -> Result<Option<ResolvedImage>>;

    /// Resolve multiple images in a batch
    async fn resolve_images_batch(
        &self,
        image_ids: &[Uuid],
    ) -> Result<HashMap<Uuid, ResolvedImage>>;

    // =========================================================================
    // Session File Operations
    // =========================================================================

    /// Read a file from session filesystem
    async fn read_file(&self, session_id: Uuid, path: &str) -> Result<Option<SessionFile>>;

    /// Write a file to session filesystem
    async fn write_file(
        &self,
        session_id: Uuid,
        path: &str,
        content: &str,
        encoding: &str,
    ) -> Result<SessionFile>;

    /// Delete a file from session filesystem
    async fn delete_file(&self, session_id: Uuid, path: &str, recursive: bool) -> Result<bool>;

    /// List directory contents
    async fn list_directory(&self, session_id: Uuid, path: &str) -> Result<Vec<FileInfo>>;

    /// Get file stats
    async fn stat_file(&self, session_id: Uuid, path: &str) -> Result<Option<FileStat>>;

    /// Search files by pattern
    async fn grep_files(
        &self,
        session_id: Uuid,
        pattern: &str,
        path_pattern: Option<&str>,
    ) -> Result<Vec<GrepMatch>>;

    /// Create a directory
    async fn create_directory(&self, session_id: Uuid, path: &str) -> Result<FileInfo>;

    // =========================================================================
    // MCP Server Operations
    // =========================================================================

    /// Get MCP server info by name prefix (for MCP tool execution)
    async fn get_mcp_server_by_prefix(
        &self,
        org_id: i64,
        server_prefix: &str,
    ) -> Result<McpServerInfo>;

    // =========================================================================
    // Turn Context (batch operation for efficiency)
    // =========================================================================

    /// Load turn context in one batch call
    /// Returns agent, session, messages, model, and MCP tool definitions
    async fn load_turn_context(&self, org_id: i64, session_id: Uuid) -> Result<TurnContext>;

    // =========================================================================
    // Factory Methods for Core Types
    // =========================================================================

    /// Get the capability registry
    fn capability_registry(&self) -> CapabilityRegistry;

    /// Get the LLM driver registry
    fn driver_registry(&self) -> DriverRegistry;

    /// Create a tool registry with defaults and agent capabilities
    async fn build_tool_registry(&self, agent_id: Uuid) -> Result<ToolRegistry>;

    /// Get the session SQL database store (if available).
    /// Default returns None (not all backends support session SQL databases).
    fn sqldb_store(&self) -> Option<everruns_core::traits::SessionSqlDbStoreRef> {
        None
    }

    /// Get the session storage store for kv_store/secret_store tools (if available).
    /// Default returns None (not all backends support session storage).
    fn storage_store(&self) -> Option<Arc<dyn everruns_core::traits::SessionStorageStore>> {
        None
    }

    /// Get the user connection resolver for lazy token lookup (if available).
    /// Default returns None (not all backends support user connections).
    fn connection_resolver(
        &self,
    ) -> Option<Arc<dyn everruns_core::traits::UserConnectionResolver>> {
        None
    }
}

// =============================================================================
// Supporting Types
// =============================================================================

/// Model with provider configuration
#[derive(Debug, Clone)]
pub struct ModelWithProvider {
    pub model: String,
    pub provider_type: LlmProviderType,
    pub api_key: Option<String>,
    pub base_url: Option<String>,
}

/// Turn context loaded in one batched call
#[derive(Debug, Clone)]
pub struct TurnContext {
    pub agent: Option<Agent>,
    pub session: Session,
    pub messages: Vec<Message>,
    pub model: Option<ModelWithProvider>,
    /// MCP tool definitions pre-resolved from agent's MCP capabilities
    pub mcp_tool_definitions: Vec<ToolDefinition>,
}

// =============================================================================
// Adapter-based trait implementations for core traits
// =============================================================================

/// Adapter-based AgentStore implementation
pub struct AdapterAgentStore<A: WorkerAdapters> {
    adapters: A,
    org_id: i64,
}

impl<A: WorkerAdapters> AdapterAgentStore<A> {
    pub fn new(adapters: A, org_id: i64) -> Self {
        Self { adapters, org_id }
    }
}

#[async_trait]
impl<A: WorkerAdapters> everruns_core::traits::AgentStore for AdapterAgentStore<A> {
    async fn get_agent(&self, agent_id: AgentId) -> Result<Option<Agent>> {
        self.adapters.get_agent(self.org_id, agent_id.uuid()).await
    }
}

/// Adapter-based HarnessStore implementation
pub struct AdapterHarnessStore<A: WorkerAdapters> {
    adapters: A,
    org_id: i64,
}

impl<A: WorkerAdapters> AdapterHarnessStore<A> {
    pub fn new(adapters: A, org_id: i64) -> Self {
        Self { adapters, org_id }
    }
}

#[async_trait]
impl<A: WorkerAdapters> everruns_core::traits::HarnessStore for AdapterHarnessStore<A> {
    async fn get_harness(&self, harness_id: HarnessId) -> Result<Option<Harness>> {
        self.adapters
            .get_harness(self.org_id, harness_id.uuid())
            .await
    }
}

/// Adapter-based SessionStore implementation
pub struct AdapterSessionStore<A: WorkerAdapters> {
    adapters: A,
    org_id: i64,
}

impl<A: WorkerAdapters> AdapterSessionStore<A> {
    pub fn new(adapters: A, org_id: i64) -> Self {
        Self { adapters, org_id }
    }
}

#[async_trait]
impl<A: WorkerAdapters> everruns_core::traits::SessionStore for AdapterSessionStore<A> {
    async fn get_session(&self, session_id: SessionId) -> Result<Option<Session>> {
        self.adapters
            .get_session(self.org_id, session_id.uuid())
            .await
    }
}

/// Adapter-based MessageRetriever implementation
pub struct AdapterMessageRetriever<A: WorkerAdapters> {
    adapters: A,
}

impl<A: WorkerAdapters> AdapterMessageRetriever<A> {
    pub fn new(adapters: A) -> Self {
        Self { adapters }
    }
}

#[async_trait]
impl<A: WorkerAdapters> everruns_core::MessageRetriever for AdapterMessageRetriever<A> {
    async fn get(&self, session_id: SessionId, message_id: MessageId) -> Result<Option<Message>> {
        self.adapters
            .get_message(session_id.uuid(), message_id.uuid())
            .await
    }

    async fn load(&self, session_id: SessionId) -> Result<Vec<Message>> {
        self.adapters.load_messages(session_id.uuid()).await
    }
}

/// Adapter-based LlmProviderStore implementation
pub struct AdapterLlmProviderStore<A: WorkerAdapters> {
    adapters: A,
    org_id: i64,
}

impl<A: WorkerAdapters> AdapterLlmProviderStore<A> {
    pub fn new(adapters: A, org_id: i64) -> Self {
        Self { adapters, org_id }
    }
}

#[async_trait]
impl<A: WorkerAdapters> everruns_core::traits::LlmProviderStore for AdapterLlmProviderStore<A> {
    async fn get_model_with_provider(
        &self,
        model_id: ModelId,
    ) -> Result<Option<everruns_core::traits::ModelWithProvider>> {
        let result = self
            .adapters
            .get_model_with_provider(self.org_id, model_id.uuid())
            .await?;
        Ok(result.map(|m| everruns_core::traits::ModelWithProvider {
            model: m.model,
            provider_type: m.provider_type,
            api_key: m.api_key,
            base_url: m.base_url,
        }))
    }

    async fn get_default_model(&self) -> Result<Option<everruns_core::traits::ModelWithProvider>> {
        let result = self.adapters.get_default_model(self.org_id).await?;
        Ok(result.map(|m| everruns_core::traits::ModelWithProvider {
            model: m.model,
            provider_type: m.provider_type,
            api_key: m.api_key,
            base_url: m.base_url,
        }))
    }
}

/// Adapter-based EventEmitter implementation
pub struct AdapterEventEmitter<A: WorkerAdapters> {
    adapters: A,
}

impl<A: WorkerAdapters> AdapterEventEmitter<A> {
    pub fn new(adapters: A) -> Self {
        Self { adapters }
    }
}

#[async_trait]
impl<A: WorkerAdapters> everruns_core::traits::EventEmitter for AdapterEventEmitter<A> {
    async fn emit(&self, request: EventRequest) -> Result<Event> {
        self.adapters.emit_event(request).await
    }
}

/// Adapter-based ImageResolver implementation
pub struct AdapterImageResolver<A: WorkerAdapters> {
    adapters: A,
}

impl<A: WorkerAdapters> AdapterImageResolver<A> {
    pub fn new(adapters: A) -> Self {
        Self { adapters }
    }
}

#[async_trait]
impl<A: WorkerAdapters> ImageResolver for AdapterImageResolver<A> {
    async fn resolve_image(&self, image_id: Uuid) -> Result<Option<ResolvedImage>> {
        self.adapters.resolve_image(image_id).await
    }
}

/// Adapter-based SessionFileStore implementation
pub struct AdapterSessionFileStore<A: WorkerAdapters> {
    adapters: A,
}

impl<A: WorkerAdapters> AdapterSessionFileStore<A> {
    pub fn new(adapters: A) -> Self {
        Self { adapters }
    }
}

#[async_trait]
impl<A: WorkerAdapters> everruns_core::traits::SessionFileStore for AdapterSessionFileStore<A> {
    async fn read_file(&self, session_id: SessionId, path: &str) -> Result<Option<SessionFile>> {
        self.adapters.read_file(session_id.uuid(), path).await
    }

    async fn write_file(
        &self,
        session_id: SessionId,
        path: &str,
        content: &str,
        encoding: &str,
    ) -> Result<SessionFile> {
        self.adapters
            .write_file(session_id.uuid(), path, content, encoding)
            .await
    }

    async fn delete_file(
        &self,
        session_id: SessionId,
        path: &str,
        recursive: bool,
    ) -> Result<bool> {
        self.adapters
            .delete_file(session_id.uuid(), path, recursive)
            .await
    }

    async fn list_directory(&self, session_id: SessionId, path: &str) -> Result<Vec<FileInfo>> {
        self.adapters.list_directory(session_id.uuid(), path).await
    }

    async fn stat_file(&self, session_id: SessionId, path: &str) -> Result<Option<FileStat>> {
        self.adapters.stat_file(session_id.uuid(), path).await
    }

    async fn grep_files(
        &self,
        session_id: SessionId,
        pattern: &str,
        path_pattern: Option<&str>,
    ) -> Result<Vec<GrepMatch>> {
        self.adapters
            .grep_files(session_id.uuid(), pattern, path_pattern)
            .await
    }

    async fn create_directory(&self, session_id: SessionId, path: &str) -> Result<FileInfo> {
        self.adapters
            .create_directory(session_id.uuid(), path)
            .await
    }
}

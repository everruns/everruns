// Worker adapters trait for unified worker implementation
//
// Decision: Single trait abstracts all data operations needed by activities
// Decision: Implementations for gRPC (external workers) and Direct (in-process)
// Decision: 9 per-trait Adapter* wrappers consolidated into 2 (EVE-103):
//   - SessionAdapter<A>  (session-scoped, no org_id)
//   - OrgAdapter<A>      (org-scoped, carries org_id)
//
// This allows a single Worker implementation to work with either backend.

use async_trait::async_trait;
use everruns_core::capabilities::{
    CapabilityRegistry, SystemPromptContext, collect_capabilities, is_mcp_capability,
};
use everruns_core::error::Result;
use everruns_core::events::{Event, EventRequest};
use everruns_core::leased_resource::LeasedResource;
use everruns_core::session_file::{FileInfo, FileStat, GrepMatch, SessionFile};
use everruns_core::traits::{
    BudgetChecker, ImageResolver, LeasedResourceStore, ModelWithProvider, ResolvedImage,
};
use everruns_core::typed_id::{
    AgentId, HarnessId, LeasedResourceId, MessageId, ModelId, SessionId,
};
use everruns_core::{
    Agent, DriverRegistry, Harness, Message, Session, ToolDefinition, ToolRegistry,
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

    /// Set a session title.
    async fn set_session_title(
        &self,
        org_id: i64,
        session_id: Uuid,
        title: String,
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
    async fn resolve_image(&self, org_id: i64, image_id: Uuid) -> Result<Option<ResolvedImage>>;

    /// Resolve multiple images in a batch
    async fn resolve_images_batch(
        &self,
        org_id: i64,
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

    /// Write a file only if its current content snapshot still matches.
    async fn write_file_if_content_matches(
        &self,
        session_id: Uuid,
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
    async fn build_tool_registry(&self, org_id: i64, agent_id: Uuid) -> Result<ToolRegistry>;

    /// Create a tool registry with defaults and harness capabilities.
    ///
    /// Fallback for sessions without an agent — builds the registry from the
    /// harness's capability list so harness-provided tools (e.g. bash) are
    /// available. Without this, only `ToolRegistry::with_defaults()` would be
    /// used, which doesn't include capability-provided tools.
    async fn build_tool_registry_for_harness(
        &self,
        org_id: i64,
        harness_id: Uuid,
    ) -> Result<ToolRegistry> {
        let mut registry = ToolRegistry::with_defaults();
        if let Ok(Some(harness)) = self.get_harness(org_id, harness_id).await {
            let builtin_cap_ids: Vec<String> = harness
                .capabilities
                .iter()
                .map(|c| c.capability_id().to_string())
                .filter(|id| !is_mcp_capability(id))
                .collect();

            if !builtin_cap_ids.is_empty() {
                let cap_registry = self.capability_registry();
                let ctx = SystemPromptContext::without_file_store(everruns_core::SessionId::new());
                let collected = collect_capabilities(&builtin_cap_ids, &cap_registry, &ctx).await;
                for tool in collected.tools {
                    registry.register_boxed(tool);
                }
            }
        }
        Ok(registry)
    }

    /// Get the session SQL database store.
    fn sqldb_store(&self) -> everruns_core::traits::SessionSqlDbStoreRef;

    // =========================================================================
    // Required Store Accessors
    //
    // NO default implementations. Every WorkerAdapters impl MUST provide these.
    // This is enforced at compile time to prevent the class of bug where a new
    // adapter silently returns None and tools fail at runtime with misleading
    // errors (e.g. "API key not configured" when the real issue is a missing
    // gRPC endpoint).
    //
    // For stores with full gRPC support, the return type is non-optional.
    // For stores still missing gRPC RPCs, the return type is Option but there
    // is no default — implementors must explicitly return None or Some.
    // =========================================================================

    /// Get the session storage store for kv_store/secret_store tools.
    fn storage_store(&self) -> Arc<dyn everruns_core::traits::SessionStorageStore>;

    /// Get the platform store for org-level management tools.
    /// Takes org_id so the store is scoped to the current session's organization.
    fn platform_store(&self, org_id: i64) -> Arc<dyn everruns_core::platform_store::PlatformStore>;

    /// Get the user connection resolver for lazy token lookup.
    fn connection_resolver(&self) -> Arc<dyn everruns_core::traits::UserConnectionResolver>;

    /// Get the leased-resource store for tool-side registration/touch/release.
    fn leased_resource_store(&self) -> Arc<dyn LeasedResourceStore>;

    /// Get the session resource registry for generic resource tracking.
    /// Returns None when the registry is not available (e.g. gRPC workers
    /// without the registry RPC — follow-up work).
    fn session_resource_registry(
        &self,
    ) -> Option<Arc<dyn everruns_core::traits::SessionResourceRegistry>> {
        None
    }

    /// Get the session schedule store for scheduling tools.
    /// Takes org_id so the store is scoped to the current session's organization.
    fn schedule_store(&self, org_id: i64) -> Arc<dyn everruns_core::traits::SessionScheduleStore>;

    /// Get the budget checker for the current turn, if available.
    ///
    /// gRPC workers provide this through the control-plane API. Direct dev-mode
    /// workers may return `None` until they are wired to the server budget
    /// service.
    fn budget_checker(
        &self,
        _org_id: i64,
        _agent_id: Option<AgentId>,
    ) -> Option<Arc<dyn BudgetChecker>> {
        None
    }

    /// Claim due leased resources for cleanup work.
    ///
    /// This is the control-plane entry point used by the durable cleanup
    /// activity. Implementations must coordinate across workers so one claim
    /// wins per resource until the claim becomes stale.
    async fn claim_due_leased_resources(
        &self,
        limit: u32,
        stale_after_seconds: u32,
    ) -> Result<Vec<LeasedResource>>;

    /// Mark a claimed leased resource as released.
    ///
    /// `expected_cleanup_started_at` is a compare-and-set guard. Cleanup code
    /// must pass the timestamp from the original claim so a stale worker cannot
    /// overwrite a newer lease refresh or retry claim.
    async fn mark_leased_resource_released(
        &self,
        resource_id: LeasedResourceId,
        expected_cleanup_started_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<bool>;

    /// Mark a claimed leased resource cleanup as failed.
    ///
    /// Like `mark_leased_resource_released`, this is guarded by the claim
    /// timestamp to preserve correctness when multiple workers race or a lease
    /// is refreshed while cleanup is in flight.
    async fn mark_leased_resource_cleanup_failed(
        &self,
        resource_id: LeasedResourceId,
        expected_cleanup_started_at: chrono::DateTime<chrono::Utc>,
        retry_after_seconds: u32,
        error: &str,
    ) -> Result<bool>;
}

// =============================================================================
// Supporting Types
// =============================================================================

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
// Consolidated adapter wrappers (EVE-103)
//
// Two generic structs replace the former 9 per-trait Adapter* wrappers:
//   - SessionAdapter<A>  (session-scoped, no org_id)
//   - OrgAdapter<A>      (org-scoped, carries org_id)
//
// Type aliases preserve the old names at all call sites.
// =============================================================================

/// Session-scoped adapter: bridges WorkerAdapters → core traits that don't need org_id.
///
/// Implements: MessageRetriever, EventEmitter, SessionFileStore.
pub struct SessionAdapter<A: WorkerAdapters> {
    adapters: A,
}

impl<A: WorkerAdapters> SessionAdapter<A> {
    pub fn new(adapters: A) -> Self {
        Self { adapters }
    }
}

/// Org-scoped adapter: bridges WorkerAdapters → core traits that need org_id.
///
/// Implements: AgentStore, HarnessStore, SessionStore, SessionMutator,
/// LlmProviderStore, ImageResolver.
pub struct OrgAdapter<A: WorkerAdapters> {
    adapters: A,
    org_id: i64,
}

impl<A: WorkerAdapters> OrgAdapter<A> {
    pub fn new(adapters: A, org_id: i64) -> Self {
        Self { adapters, org_id }
    }
}

// Type aliases for backward compatibility at call sites
pub type AdapterAgentStore<A> = OrgAdapter<A>;
pub type AdapterHarnessStore<A> = OrgAdapter<A>;
pub type AdapterSessionStore<A> = OrgAdapter<A>;
pub type AdapterSessionMutator<A> = OrgAdapter<A>;
pub type AdapterLlmProviderStore<A> = OrgAdapter<A>;
pub type AdapterImageResolver<A> = OrgAdapter<A>;
pub type AdapterMessageRetriever<A> = SessionAdapter<A>;
pub type AdapterEventEmitter<A> = SessionAdapter<A>;
pub type AdapterSessionFileStore<A> = SessionAdapter<A>;

// --- Org-scoped trait impls ---

#[async_trait]
impl<A: WorkerAdapters> everruns_core::traits::AgentStore for OrgAdapter<A> {
    async fn get_agent(&self, agent_id: AgentId) -> Result<Option<Agent>> {
        self.adapters.get_agent(self.org_id, agent_id.uuid()).await
    }
}

#[async_trait]
impl<A: WorkerAdapters> everruns_core::traits::HarnessStore for OrgAdapter<A> {
    async fn get_harness_chain(&self, harness_id: HarnessId) -> Result<Vec<Harness>> {
        // WorkerAdapters returns a pre-merged harness; wrap as single-element chain
        Ok(self
            .adapters
            .get_harness(self.org_id, harness_id.uuid())
            .await?
            .into_iter()
            .collect())
    }
}

#[async_trait]
impl<A: WorkerAdapters> everruns_core::traits::SessionStore for OrgAdapter<A> {
    async fn get_session(&self, session_id: SessionId) -> Result<Option<Session>> {
        self.adapters
            .get_session(self.org_id, session_id.uuid())
            .await
    }
}

#[async_trait]
impl<A: WorkerAdapters> everruns_core::traits::SessionMutator for OrgAdapter<A> {
    async fn update_session_title(&self, session_id: SessionId, title: String) -> Result<Session> {
        self.adapters
            .set_session_title(self.org_id, session_id.uuid(), title)
            .await
    }
}

#[async_trait]
impl<A: WorkerAdapters> everruns_core::traits::LlmProviderStore for OrgAdapter<A> {
    async fn get_model_with_provider(
        &self,
        model_id: ModelId,
    ) -> Result<Option<ModelWithProvider>> {
        self.adapters
            .get_model_with_provider(self.org_id, model_id.uuid())
            .await
    }

    async fn get_default_model(&self) -> Result<Option<ModelWithProvider>> {
        self.adapters.get_default_model(self.org_id).await
    }
}

#[async_trait]
impl<A: WorkerAdapters> ImageResolver for OrgAdapter<A> {
    async fn resolve_image(&self, image_id: Uuid) -> Result<Option<ResolvedImage>> {
        self.adapters.resolve_image(self.org_id, image_id).await
    }
}

// --- Session-scoped trait impls ---

#[async_trait]
impl<A: WorkerAdapters> everruns_core::MessageRetriever for SessionAdapter<A> {
    async fn get(&self, session_id: SessionId, message_id: MessageId) -> Result<Option<Message>> {
        self.adapters
            .get_message(session_id.uuid(), message_id.uuid())
            .await
    }

    async fn load(&self, session_id: SessionId) -> Result<Vec<Message>> {
        self.adapters.load_messages(session_id.uuid()).await
    }
}

#[async_trait]
impl<A: WorkerAdapters> everruns_core::traits::EventEmitter for SessionAdapter<A> {
    async fn emit(&self, request: EventRequest) -> Result<Event> {
        self.adapters.emit_event(request).await
    }
}

#[async_trait]
impl<A: WorkerAdapters> everruns_core::traits::SessionFileStore for SessionAdapter<A> {
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

    async fn write_file_if_content_matches(
        &self,
        session_id: SessionId,
        path: &str,
        expected_content: &str,
        expected_encoding: &str,
        content: &str,
        encoding: &str,
    ) -> Result<Option<SessionFile>> {
        self.adapters
            .write_file_if_content_matches(
                session_id.uuid(),
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

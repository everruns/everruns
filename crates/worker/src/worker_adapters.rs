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
use everruns_core::capabilities::CapabilityRegistry;
use everruns_core::error::Result;
use everruns_core::events::{Event, EventRequest};
use everruns_core::leased_resource::LeasedResource;
use everruns_core::session_file::{
    FileInfo, FileStat, GrepMatch, GrepOptions, GrepSearchResult, SessionFile,
};
use everruns_core::traits::{
    BudgetChecker, ImageArtifactStore, ImageResolver, LeasedResourceStore, PaymentAuthority,
    ProviderCredentialStore, ResolvedImage, ResolvedModel, SessionCreationAuthority,
};
use everruns_core::typed_id::{
    AgentId, HarnessId, LeasedResourceId, MessageId, ModelId, SessionId,
};
use everruns_core::{
    AgentDefinition, DriverRegistry, EgressService, ExecutionSession, HarnessDefinition, Message,
    MessageHistory, MessageQuery, ToolDefinition, UtilityLlmService,
};
// EVE-877: the stored Agent record moved to `everruns-platform`. WorkerAdapters
// still transports it between control plane and worker; host/engine only ever
// see the projected `AgentDefinition` / resolved execution snapshot.
use everruns_platform::{Agent, Harness};
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

    /// Get the stored agent record by ID (platform-side transport; never
    /// handed to host execution — see `AgentStore for OrgAdapter`).
    async fn get_agent(&self, org_id: i64, agent_id: Uuid) -> Result<Option<Agent>>;

    /// Get harness by ID
    async fn get_harness(&self, org_id: i64, harness_id: Uuid) -> Result<Option<Harness>>;

    // =========================================================================
    // Session Operations
    // =========================================================================

    /// Get the portable execution view of a session by ID (EVE-882: the
    /// stored Session record stays on the control plane).
    async fn get_session(&self, org_id: i64, session_id: Uuid) -> Result<Option<ExecutionSession>>;

    /// Set session status (started, active, idle). Acknowledgement only —
    /// status mutation exposes no session record (EVE-882).
    async fn set_session_status(&self, org_id: i64, session_id: Uuid, status: &str) -> Result<()>;

    /// Set a session title, acknowledging with the refreshed portable
    /// execution view.
    async fn set_session_title(
        &self,
        org_id: i64,
        session_id: Uuid,
        title: String,
    ) -> Result<ExecutionSession>;

    // =========================================================================
    // Message Operations
    // =========================================================================

    /// Get a specific message by ID
    async fn get_message(&self, session_id: Uuid, message_id: Uuid) -> Result<Option<Message>>;

    /// Load all messages for a session
    async fn load_messages(&self, session_id: Uuid) -> Result<Vec<Message>>;

    async fn load_message_history(&self, query: MessageQuery) -> Result<MessageHistory> {
        Ok(MessageHistory {
            messages: self.load_messages(query.session_id.uuid()).await?,
            source_sequence: None,
        })
    }

    // =========================================================================
    // Event Operations
    // =========================================================================

    /// Emit an event
    async fn emit_event(&self, request: EventRequest) -> Result<Event>;

    // =========================================================================
    // LLM Provider Operations
    // =========================================================================

    /// Get model with provider configuration
    async fn get_resolved_model(
        &self,
        org_id: i64,
        model_id: Uuid,
    ) -> Result<Option<ResolvedModel>>;

    /// Get default model configuration
    async fn get_default_model(&self, org_id: i64) -> Result<Option<ResolvedModel>>;

    async fn get_provider_config(
        &self,
        org_id: i64,
        provider: &everruns_core::ProviderKey,
    ) -> Result<Option<everruns_core::ProviderConfig>>;

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

    async fn grep_files_with_options(
        &self,
        session_id: Uuid,
        pattern: &str,
        options: &GrepOptions,
    ) -> Result<GrepSearchResult> {
        if options.before_context != 0 || options.after_context != 0 {
            return Err(everruns_core::AgentLoopError::tool(
                "this worker adapter does not support grep context",
            ));
        }
        let matches = self
            .grep_files(session_id, pattern, options.path_pattern.as_deref())
            .await?;
        Ok(everruns_core::session_file::bound_grep_matches(
            matches, options,
        ))
    }

    /// Create a directory
    async fn create_directory(&self, session_id: Uuid, path: &str) -> Result<FileInfo>;

    // =========================================================================
    // MCP Server Operations
    // =========================================================================

    /// Get MCP server info by name prefix (for MCP tool execution)
    async fn get_mcp_server_by_prefix(
        &self,
        org_id: i64,
        session_id: Option<Uuid>,
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

    /// Get the session SQL database store.
    fn sqldb_store(&self) -> everruns_core::traits::SessionSqlDbStoreRef;

    fn compaction_checkpoint_store(
        &self,
    ) -> Option<Arc<dyn everruns_core::CompactionCheckpointStore>> {
        None
    }

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

    /// Get the image artifact store for tool-side image persistence.
    fn image_artifact_store(&self, org_id: i64) -> Arc<dyn ImageArtifactStore>;

    /// Get the provider credential store for tool-side provider auth lookup.
    fn provider_credential_store(&self, org_id: i64) -> Arc<dyn ProviderCredentialStore>;

    /// Get the system utility LLM service for capability internals.
    fn utility_llm_service(&self) -> Option<Arc<dyn UtilityLlmService>>;

    /// Get the outbound egress service for HTTP/API traffic.
    fn egress_service(&self) -> Option<Arc<dyn EgressService>>;

    /// Get the platform store for org-level management tools.
    /// Takes org_id so the store is scoped to the current session's organization.
    fn platform_store(
        &self,
        org_id: i64,
        session_id: SessionId,
    ) -> Arc<dyn everruns_platform::PlatformStore>;

    /// Get the user connection resolver for lazy token lookup.
    fn connection_resolver(&self) -> Arc<dyn everruns_core::traits::UserConnectionResolver>;

    /// Get the Knowledge Index search service for the `search_index` tool,
    /// scoped to the given org. Returns None when retrieval is not available
    /// (e.g. gRPC workers without a search RPC — follow-up work).
    fn knowledge_index_search(
        &self,
        _org_id: i64,
    ) -> Option<Arc<dyn everruns_platform::vector_store::KnowledgeIndexSearch>> {
        None
    }

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

    /// Get the session task registry for background work tracking.
    /// Returns None when the registry is not available (e.g. gRPC workers
    /// without the task RPCs — follow-up work).
    fn session_task_registry(
        &self,
    ) -> Option<Arc<dyn everruns_core::session_task::SessionTaskRegistry>> {
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

    /// Get the payment authority for paid internal capability tools, if available.
    fn payment_authority(
        &self,
        _org_id: i64,
        _agent_id: Option<AgentId>,
    ) -> Option<Arc<dyn PaymentAuthority>> {
        None
    }

    /// Get the authority for detached peer-session creation, scoped to the
    /// current session owner.
    fn session_creation_authority(
        &self,
        _org_id: i64,
        _session_id: SessionId,
    ) -> Option<Arc<dyn SessionCreationAuthority>> {
        None
    }

    /// Per-org outbound tool-call rate limiter (TM-TOOL-009).
    /// Default: `None` (no rate limiting — suitable for dev/worker environments
    /// that do not need the production limit).
    fn outbound_tool_rate_limiter(
        &self,
        _org_id: i64,
    ) -> Option<Arc<dyn everruns_core::OutboundToolRateLimiter>> {
        None
    }

    /// Per-turn durable tool result store for act-activity idempotency (EVE-530).
    /// Default: `None` (no durable idempotency — suitable for dev/test environments).
    fn durable_tool_result_store(&self) -> Option<Arc<dyn everruns_core::DurableToolResultStore>> {
        None
    }

    /// Durable subagent spawn handle store for reattach on reclaim (EVE-535).
    /// Default: `None` (no spawn dedup — suitable for dev/test environments).
    fn subagent_spawn_store(&self) -> Option<Arc<dyn everruns_core::SubagentSpawnStore>> {
        None
    }

    /// Knowledge store backing the `search_knowledge` tool. Default: `None`.
    fn knowledge_store(&self) -> Option<Arc<dyn everruns_platform::KnowledgeStore>> {
        None
    }

    /// Stream-liveness heartbeater for the Reason activity (EVE-531).
    /// Default: `None` (no heartbeats sent — durable workers supply one).
    fn stream_heartbeater(&self) -> Option<Arc<dyn everruns_core::StreamHeartbeater>> {
        None
    }

    /// Provider stall timeout for the Reason activity (EVE-531).
    /// Default: `None` (use built-in 120s default in ReasonAtom).
    fn provider_stall_timeout(&self) -> Option<std::time::Duration> {
        None
    }

    /// Invoke an app schedule channel when a durable schedule fires.
    async fn invoke_scheduled_app_channel(
        &self,
        org_id: i64,
        app_id: &str,
        channel_id: &str,
    ) -> Result<serde_json::Value>;

    /// Invoke an agent schedule trigger when a durable schedule fires (EVE-757).
    async fn invoke_agent_trigger(
        &self,
        org_id: i64,
        agent_id: &str,
        trigger_id: &str,
    ) -> Result<serde_json::Value>;

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

    // =========================================================================
    // Session task reaper (orphan reconciler)
    // =========================================================================

    /// Return (session_id, task_id) pairs for tasks whose worker heartbeat has
    /// gone stale. Tasks with NULL heartbeat_at are excluded (foreground tasks
    /// with no liveness probe are covered by EVE-535 spawn handles).
    async fn list_orphaned_session_task_ids(
        &self,
        stale_after: chrono::Duration,
        limit: i64,
    ) -> Result<Vec<(everruns_core::SessionId, String)>>;

    /// Session task registry for the reaper to call `update` through.
    /// Must include an event emitter so task.updated events fire on reap.
    fn reaper_session_task_registry(
        &self,
    ) -> std::sync::Arc<dyn everruns_core::session_task::SessionTaskRegistry>;

    /// Prune a bounded batch of terminal session tasks (succeeded/failed/
    /// canceled) whose `finished_at` is older than `ttl`, removing their
    /// `session_tasks` rows, `session_task_messages`, and `result_path`
    /// artifacts (EVE-580). Returns the number of tasks pruned this pass.
    ///
    /// Row deletion commits first; artifact (blob) deletion happens after, so a
    /// crash can at worst leak a dangling blob (reclaimed by blob GC) rather
    /// than leave a row pointing at a deleted artifact. `limit` bounds the work
    /// per pass; a backlog drains across successive reaper ticks.
    async fn prune_terminal_session_tasks(
        &self,
        ttl: chrono::Duration,
        limit: i64,
    ) -> Result<usize>;
}

// =============================================================================
// Supporting Types
// =============================================================================

/// Turn context loaded in one batched call
#[derive(Debug, Clone)]
pub struct TurnContext {
    pub agent: Option<Agent>,
    pub session: ExecutionSession,
    pub messages: Vec<Message>,
    pub model: Option<ResolvedModel>,
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
/// Implements: MessageRetriever, EventEmitter, SessionFileSystem.
pub struct SessionAdapter<A: WorkerAdapters> {
    adapters: A,
    event_metadata: Option<serde_json::Map<String, serde_json::Value>>,
}

impl<A: WorkerAdapters> SessionAdapter<A> {
    pub fn new(adapters: A) -> Self {
        Self {
            adapters,
            event_metadata: None,
        }
    }

    pub fn with_event_metadata(
        mut self,
        metadata: Option<serde_json::Map<String, serde_json::Value>>,
    ) -> Self {
        self.event_metadata = metadata;
        self
    }
}

/// Org-scoped adapter: bridges WorkerAdapters → core traits that need org_id.
///
/// Implements: AgentStore, HarnessStore, SessionStore, SessionMutator,
/// ProviderStore, ImageResolver.
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
pub type AdapterProviderStore<A> = OrgAdapter<A>;
pub type AdapterImageResolver<A> = OrgAdapter<A>;
pub type AdapterMessageRetriever<A> = SessionAdapter<A>;
pub type AdapterEventEmitter<A> = SessionAdapter<A>;
pub type AdapterSessionFileStore<A> = SessionAdapter<A>;

// --- Org-scoped trait impls ---

#[async_trait]
impl<A: WorkerAdapters> everruns_core::traits::AgentStore for OrgAdapter<A> {
    async fn get_agent(&self, agent_id: AgentId) -> Result<Option<AgentDefinition>> {
        // Loading seam (EVE-877): project the stored record into the portable
        // execution definition; archived/deleted records fail here, before
        // host execution.
        self.adapters
            .get_agent(self.org_id, agent_id.uuid())
            .await?
            .map(|agent| agent.execution_definition())
            .transpose()
    }

    async fn get_agent_blocker(
        &self,
        agent_id: AgentId,
    ) -> Result<Option<everruns_core::DependencyBlocker>> {
        Ok(
            match self
                .adapters
                .get_agent(self.org_id, agent_id.uuid())
                .await?
            {
                Some(agent) => agent.dependency_blocker(),
                None => Some(everruns_core::DependencyBlocker::AgentDeleted),
            },
        )
    }
}

#[async_trait]
impl<A: WorkerAdapters> everruns_core::traits::HarnessStore for OrgAdapter<A> {
    async fn get_harness(&self, harness_id: HarnessId) -> Result<Option<HarnessDefinition>> {
        // Loading seam (EVE-881): WorkerAdapters transports the pre-merged
        // stored record; project it into the portable execution definition,
        // failing archived/deleted records here.
        self.adapters
            .get_harness(self.org_id, harness_id.uuid())
            .await?
            .map(|harness| harness.execution_definition())
            .transpose()
    }

    async fn get_harness_blocker(
        &self,
        harness_id: HarnessId,
    ) -> Result<Option<everruns_core::DependencyBlocker>> {
        Ok(
            match self
                .adapters
                .get_harness(self.org_id, harness_id.uuid())
                .await?
            {
                Some(harness) => harness.dependency_blocker(),
                None => Some(everruns_core::DependencyBlocker::HarnessDeleted),
            },
        )
    }
}

#[async_trait]
impl<A: WorkerAdapters> everruns_core::traits::SessionStore for OrgAdapter<A> {
    async fn get_session(&self, session_id: SessionId) -> Result<Option<ExecutionSession>> {
        self.adapters
            .get_session(self.org_id, session_id.uuid())
            .await
    }
}

#[async_trait]
impl<A: WorkerAdapters> everruns_core::traits::SessionMutator for OrgAdapter<A> {
    async fn update_session_title(
        &self,
        session_id: SessionId,
        title: String,
    ) -> Result<ExecutionSession> {
        self.adapters
            .set_session_title(self.org_id, session_id.uuid(), title)
            .await
    }
}

#[async_trait]
impl<A: WorkerAdapters> everruns_core::traits::ProviderStore for OrgAdapter<A> {
    async fn get_resolved_model(&self, model_id: ModelId) -> Result<Option<ResolvedModel>> {
        self.adapters
            .get_resolved_model(self.org_id, model_id.uuid())
            .await
    }

    async fn get_default_model(&self) -> Result<Option<ResolvedModel>> {
        self.adapters.get_default_model(self.org_id).await
    }

    async fn get_provider_config(
        &self,
        provider: &everruns_core::ProviderKey,
    ) -> Result<Option<everruns_core::ProviderConfig>> {
        self.adapters
            .get_provider_config(self.org_id, provider)
            .await
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

    async fn load_filtered(&self, query: everruns_core::MessageQuery) -> Result<Vec<Message>> {
        Ok(self.adapters.load_message_history(query).await?.messages)
    }

    async fn load_filtered_history(
        &self,
        query: everruns_core::MessageQuery,
    ) -> Result<everruns_core::MessageHistory> {
        self.adapters.load_message_history(query).await
    }
}

#[async_trait]
impl<A: WorkerAdapters> everruns_core::traits::EventEmitter for SessionAdapter<A> {
    async fn emit(&self, mut request: EventRequest) -> Result<Event> {
        if let Some(extra) = &self.event_metadata {
            let mut metadata = request
                .metadata
                .take()
                .and_then(|value| value.as_object().cloned())
                .unwrap_or_default();
            for (key, value) in extra {
                metadata.entry(key.clone()).or_insert_with(|| value.clone());
            }
            request.metadata = Some(serde_json::Value::Object(metadata));
        }
        self.adapters.emit_event(request).await
    }
}

#[async_trait]
impl<A: WorkerAdapters> everruns_core::traits::SessionFileSystem for SessionAdapter<A> {
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

    async fn grep_files_with_options(
        &self,
        session_id: SessionId,
        pattern: &str,
        options: &GrepOptions,
    ) -> Result<GrepSearchResult> {
        self.adapters
            .grep_files_with_options(session_id.uuid(), pattern, options)
            .await
    }

    async fn create_directory(&self, session_id: SessionId, path: &str) -> Result<FileInfo> {
        self.adapters
            .create_directory(session_id.uuid(), path)
            .await
    }

    fn is_mount_resolver(&self) -> bool {
        false
    }
}

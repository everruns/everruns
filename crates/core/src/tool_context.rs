//! Tool context assembly, extension services, and availability validation.

use crate::connection_services::{ProviderCredentialStore, UserConnectionResolver};
use crate::delegation_services::{
    SessionCreationAuthority, SubagentNestingPolicy, SubagentSpawnStore,
};
use crate::event_emitter::EventEmitter;
use crate::events::EventRequest;
use crate::execution_loading::{AgentStore, SessionStore};
use crate::image_services::ImageArtifactStore;
use crate::session_files::SessionFileSystem;
use crate::session_services::{
    LeasedResourceStore, SessionResourceRegistry, SessionScheduleStore, SessionStorageStore,
};
use crate::tool_execution::{BudgetChecker, PaymentAuthority};
use crate::typed_id::{SessionId, WorkspaceId};
use std::any::{Any, TypeId};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// A live, shared handle to the reasoning effort for the current turn (EVE-595).
///
/// Each internal LLM step within a single `run_turn` re-reads this handle when
/// building its provider request. A tool (or any in-turn actor) can call
/// [`ReasoningEffortHandle::set`] mid-turn so that *subsequent* LLM steps in the
/// same turn use the new effort, without waiting for the next turn.
///
/// When the handle holds `None`, the engine reason phase falls back to
/// the effort resolved from the latest user message's `controls` — so callers
/// that never set an override see no behavior change.
#[derive(Clone, Default)]
pub struct ReasoningEffortHandle {
    inner: Arc<std::sync::RwLock<Option<everruns_provider::model::ReasoningEffort>>>,
}

impl ReasoningEffortHandle {
    /// Create an empty handle (no override).
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a handle pre-seeded with an effort override.
    pub fn with_effort(effort: everruns_provider::model::ReasoningEffort) -> Self {
        Self {
            inner: Arc::new(std::sync::RwLock::new(Some(effort))),
        }
    }

    /// Set (or replace) the override effort. Subsequent LLM steps in the same
    /// turn pick this up. Pass `None` to clear the override and fall back to the
    /// message-derived effort.
    pub fn set(&self, effort: Option<everruns_provider::model::ReasoningEffort>) {
        // Recover from a poisoned lock so a panic elsewhere never silently
        // disables mid-turn overrides; the stored value is a plain Option with
        // no broken invariant to worry about.
        let mut guard = self.inner.write().unwrap_or_else(|e| e.into_inner());
        *guard = effort;
    }

    /// Read the current override effort, if any.
    pub fn get(&self) -> Option<everruns_provider::model::ReasoningEffort> {
        // Recover from a poisoned lock rather than silently reverting to the
        // message-derived effort mid-turn (see `set`).
        let guard = self.inner.read().unwrap_or_else(|e| e.into_inner());
        *guard
    }
}

impl std::fmt::Debug for ReasoningEffortHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReasoningEffortHandle")
            .field("effort", &self.get())
            .finish()
    }
}

/// Runtime-owned services that may be exposed to tools.
///
/// Tools declare the subset they require through
/// [`crate::tools::Tool::required_context_services`]. Runtime hosts validate
/// those declarations before advertising tools to the model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolContextService {
    SessionFileSystem,
    SessionStorageStore,
    ImageArtifactStore,
    ProviderCredentialStore,
    UtilityLlmService,
    McpInvoker,
    EgressService,
    MessageRetriever,
    SessionStore,
    AgentStore,
    ConnectionResolver,
    ScheduleStore,
    SubagentSessionDelegate,
    LeasedResourceStore,
    SessionResourceRegistry,
    SessionTaskRegistry,
    EventEmitter,
    CapabilityRegistry,
    ToolRegistry,
    OrgId,
    BudgetChecker,
    PaymentAuthority,
    SessionCreationAuthority,
    SubagentSpawnStore,
    ReasoningEffortHandle,
}

impl ToolContextService {
    pub const fn name(self) -> &'static str {
        match self {
            Self::SessionFileSystem => "SessionFileSystem",
            Self::SessionStorageStore => "SessionStorageStore",
            Self::ImageArtifactStore => "ImageArtifactStore",
            Self::ProviderCredentialStore => "ProviderCredentialStore",
            Self::UtilityLlmService => "UtilityLlmService",
            Self::McpInvoker => "McpInvoker",
            Self::EgressService => "EgressService",
            Self::MessageRetriever => "MessageRetriever",
            Self::SessionStore => "SessionStore",
            Self::AgentStore => "AgentStore",
            Self::ConnectionResolver => "ConnectionResolver",
            Self::ScheduleStore => "SessionScheduleStore",
            Self::SubagentSessionDelegate => "SubagentSessionDelegate",
            Self::LeasedResourceStore => "LeasedResourceStore",
            Self::SessionResourceRegistry => "SessionResourceRegistry",
            Self::SessionTaskRegistry => "SessionTaskRegistry",
            Self::EventEmitter => "EventEmitter",
            Self::CapabilityRegistry => "CapabilityRegistry",
            Self::ToolRegistry => "ToolRegistry",
            Self::OrgId => "OrgId",
            Self::BudgetChecker => "BudgetChecker",
            Self::PaymentAuthority => "PaymentAuthority",
            Self::SessionCreationAuthority => "SessionCreationAuthority",
            Self::SubagentSpawnStore => "SubagentSpawnStore",
            Self::ReasoningEffortHandle => "ReasoningEffortHandle",
        }
    }
}

/// Cloneable snapshot of all services a runtime host makes available to tools.
///
/// Production act execution constructs each [`ToolContext`] from this bundle;
/// per-call metadata is applied afterwards.
#[derive(Clone, Default)]
pub struct ToolContextServices {
    pub file_store: Option<Arc<dyn SessionFileSystem>>,
    pub storage_store: Option<Arc<dyn SessionStorageStore>>,
    pub image_store: Option<Arc<dyn ImageArtifactStore>>,
    pub provider_credential_store: Option<Arc<dyn ProviderCredentialStore>>,
    pub utility_llm_service: Option<Arc<dyn crate::UtilityLlmService>>,
    pub mcp_invoker: Option<Arc<dyn crate::McpToolInvoker>>,
    pub egress_service: Option<Arc<dyn crate::EgressService>>,
    pub message_retriever: Option<Arc<dyn crate::message_retriever::MessageRetriever>>,
    pub session_store: Option<Arc<dyn SessionStore>>,
    pub agent_store: Option<Arc<dyn AgentStore>>,
    pub connection_resolver: Option<Arc<dyn UserConnectionResolver>>,
    pub schedule_store: Option<Arc<dyn SessionScheduleStore>>,
    pub subagent_delegate: Option<Arc<dyn crate::subagent_delegation::SubagentSessionDelegate>>,
    pub extensions: ToolContextExtensions,
    pub leased_resource_store: Option<Arc<dyn LeasedResourceStore>>,
    pub session_resource_registry: Option<Arc<dyn SessionResourceRegistry>>,
    pub session_task_registry: Option<Arc<dyn crate::session_task::SessionTaskRegistry>>,
    pub event_emitter: Option<Arc<dyn EventEmitter>>,
    pub capability_registry: Option<crate::capabilities::CapabilityRegistry>,
    pub tool_registry: Option<Arc<crate::tools::ToolRegistry>>,
    pub org_id: Option<crate::typed_id::OrgId>,
    pub network_access: Option<crate::network_access::NetworkAccessList>,
    pub budget_checker: Option<Arc<dyn BudgetChecker>>,
    pub payment_authority: Option<Arc<dyn PaymentAuthority>>,
    pub session_creation_authority: Option<Arc<dyn SessionCreationAuthority>>,
    pub subagent_spawn_store: Option<Arc<dyn SubagentSpawnStore>>,
    pub subagent_nesting_policy: SubagentNestingPolicy,
    pub reasoning_effort_handle: Option<ReasoningEffortHandle>,
}

impl ToolContextServices {
    pub fn provides(&self, service: ToolContextService) -> bool {
        match service {
            ToolContextService::SessionFileSystem => self.file_store.is_some(),
            ToolContextService::SessionStorageStore => self.storage_store.is_some(),
            ToolContextService::ImageArtifactStore => self.image_store.is_some(),
            ToolContextService::ProviderCredentialStore => self.provider_credential_store.is_some(),
            ToolContextService::UtilityLlmService => self.utility_llm_service.is_some(),
            ToolContextService::McpInvoker => self.mcp_invoker.is_some(),
            ToolContextService::EgressService => self.egress_service.is_some(),
            ToolContextService::MessageRetriever => self.message_retriever.is_some(),
            ToolContextService::SessionStore => self.session_store.is_some(),
            ToolContextService::AgentStore => self.agent_store.is_some(),
            ToolContextService::ConnectionResolver => self.connection_resolver.is_some(),
            ToolContextService::ScheduleStore => self.schedule_store.is_some(),
            ToolContextService::SubagentSessionDelegate => self.subagent_delegate.is_some(),
            ToolContextService::LeasedResourceStore => self.leased_resource_store.is_some(),
            ToolContextService::SessionResourceRegistry => self.session_resource_registry.is_some(),
            ToolContextService::SessionTaskRegistry => self.session_task_registry.is_some(),
            ToolContextService::EventEmitter => self.event_emitter.is_some(),
            ToolContextService::CapabilityRegistry => self.capability_registry.is_some(),
            ToolContextService::ToolRegistry => self.tool_registry.is_some(),
            ToolContextService::OrgId => self.org_id.is_some(),
            ToolContextService::BudgetChecker => self.budget_checker.is_some(),
            ToolContextService::PaymentAuthority => self.payment_authority.is_some(),
            ToolContextService::SessionCreationAuthority => {
                self.session_creation_authority.is_some()
            }
            ToolContextService::SubagentSpawnStore => self.subagent_spawn_store.is_some(),
            ToolContextService::ReasoningEffortHandle => self.reasoning_effort_handle.is_some(),
        }
    }
}

/// Type-erased bag of host-supplied extension services carried on a
/// [`ToolContext`]. Crates layered above core (e.g. `everruns-platform`) insert
/// a concrete wrapper type and resolve it by type, so core need not name the
/// hosted service (EVE-839).
#[derive(Clone, Default)]
pub struct ToolContextExtensions {
    values: Arc<HashMap<TypeId, Arc<dyn Any + Send + Sync>>>,
}

impl ToolContextExtensions {
    /// Insert (or replace) an extension keyed by its concrete type.
    pub fn insert<T: Any + Send + Sync>(&mut self, value: Arc<T>) {
        Arc::make_mut(&mut self.values).insert(TypeId::of::<T>(), value);
    }

    /// Resolve an extension by its concrete type.
    pub fn get<T: Any + Send + Sync>(&self) -> Option<Arc<T>> {
        self.values
            .get(&TypeId::of::<T>())
            .and_then(|value| value.clone().downcast::<T>().ok())
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

impl std::fmt::Debug for ToolContextExtensions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolContextExtensions")
            .field("len", &self.values.len())
            .finish()
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
    /// attached workspace's files. See knowledge/runtime-resources/workspace.md.
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

    /// Optional scoped-MCP tool invoker for capability internals that need to
    /// call an MCP server out-of-band (e.g. the guardrails `mcp` check
    /// delegating a decision to an external guardrail endpoint). The invoker
    /// resolves connections and credentials per the current session/org, so
    /// tenant scoping is enforced by the host that supplies it.
    pub mcp_invoker: Option<Arc<dyn crate::McpToolInvoker>>,

    /// Optional outbound egress service for HTTP/API traffic.
    pub egress_service: Option<Arc<dyn crate::EgressService>>,

    /// Optional message retriever for tools that need conversation history access
    pub message_retriever: Option<Arc<dyn crate::message_retriever::MessageRetriever>>,

    /// Optional session store for tools that need session metadata access.
    pub session_store: Option<Arc<dyn SessionStore>>,

    /// Optional agent store for tools that need agent metadata access.
    pub agent_store: Option<Arc<dyn AgentStore>>,

    /// Optional resolver for user connection tokens (lazy GitHub token lookup, etc.)
    pub connection_resolver: Option<Arc<dyn UserConnectionResolver>>,

    /// Optional session schedule store for scheduling tools.
    pub schedule_store: Option<Arc<dyn SessionScheduleStore>>,

    /// Optional narrow child-session delegate for host-provided orchestration.
    /// The hosted `PlatformStore` seam itself is not named by core; a host
    /// adapter implements this contract and platform tools use typed extensions.
    pub subagent_delegate: Option<Arc<dyn crate::subagent_delegation::SubagentSessionDelegate>>,
    /// Type-erased, host-supplied extensions keyed by concrete type. Lets crates
    /// layered above core (e.g. `everruns-platform`) hang typed services on the
    /// tool context without core naming them (EVE-839).
    pub extensions: ToolContextExtensions,
    /// Optional leased resource store for lifecycle-managed provider resources.
    pub leased_resource_store: Option<Arc<dyn LeasedResourceStore>>,

    /// Optional session resource registry — generic registry of active resources.
    pub session_resource_registry: Option<Arc<dyn SessionResourceRegistry>>,

    /// Optional session task registry — background work owned by the session
    /// (knowledge/runtime-resources/session-tasks.md).
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

    /// Optional host authority for detached peer-session creation.
    pub session_creation_authority: Option<Arc<dyn SessionCreationAuthority>>,

    /// Optional durable spawn handle store for subagent reattach (EVE-535).
    /// When set, subagent delegation uses claim/settle to prevent duplicate spawning
    /// on parent worker reclaim.
    pub subagent_spawn_store: Option<Arc<dyn SubagentSpawnStore>>,

    /// Resolved subagent nesting policy for this turn.
    pub subagent_nesting_policy: SubagentNestingPolicy,

    /// Optional live reasoning-effort handle (EVE-595). When set, a tool can
    /// change the reasoning effort mid-turn; subsequent LLM steps in the same
    /// `run_turn` re-read it and use the new effort.
    pub reasoning_effort_handle: Option<ReasoningEffortHandle>,

    /// Cooperative cancellation for this tool call.
    ///
    /// The act atom cancels it when the call ends *by any means* — the turn was
    /// cancelled, the act future was dropped, or the tool simply returned. A
    /// tool that only awaits inside its own future needs nothing here: dropping
    /// the future already stops it. This exists for work a tool starts and
    /// leaves running — a child process, a detached watcher task, a prompt
    /// waiting on an answer — which would otherwise outlive the call that
    /// created it. Clone the token into that work and let it die with the call.
    pub cancellation: Option<tokio_util::sync::CancellationToken>,
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
            mcp_invoker: None,
            egress_service: None,
            message_retriever: None,
            session_store: None,
            agent_store: None,
            connection_resolver: None,
            schedule_store: None,
            subagent_delegate: None,
            extensions: ToolContextExtensions::default(),
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
            session_creation_authority: None,
            subagent_spawn_store: None,
            subagent_nesting_policy: SubagentNestingPolicy::default(),
            reasoning_effort_handle: None,
            cancellation: None,
        }
    }

    /// Construct a production tool context from a validated runtime service snapshot.
    pub fn from_services(session_id: SessionId, services: &ToolContextServices) -> Self {
        Self {
            session_id,
            workspace_id: WorkspaceId::from_uuid(session_id.uuid()),
            file_store: services.file_store.clone(),
            storage_store: services.storage_store.clone(),
            image_store: services.image_store.clone(),
            provider_credential_store: services.provider_credential_store.clone(),
            utility_llm_service: services.utility_llm_service.clone(),
            mcp_invoker: services.mcp_invoker.clone(),
            egress_service: services.egress_service.clone(),
            message_retriever: services.message_retriever.clone(),
            session_store: services.session_store.clone(),
            agent_store: services.agent_store.clone(),
            connection_resolver: services.connection_resolver.clone(),
            schedule_store: services.schedule_store.clone(),
            subagent_delegate: services.subagent_delegate.clone(),
            extensions: services.extensions.clone(),
            leased_resource_store: services.leased_resource_store.clone(),
            session_resource_registry: services.session_resource_registry.clone(),
            session_task_registry: services.session_task_registry.clone(),
            event_emitter: services.event_emitter.clone(),
            event_context: None,
            tool_call_id: None,
            capability_registry: services.capability_registry.clone(),
            tool_registry: services.tool_registry.clone(),
            visible_tool_names: None,
            org_id: services.org_id,
            network_access: services.network_access.clone(),
            locale: None,
            budget_checker: services.budget_checker.clone(),
            payment_authority: services.payment_authority.clone(),
            session_creation_authority: services.session_creation_authority.clone(),
            subagent_spawn_store: services.subagent_spawn_store.clone(),
            subagent_nesting_policy: services.subagent_nesting_policy,
            reasoning_effort_handle: services.reasoning_effort_handle.clone(),
            cancellation: None,
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
            mcp_invoker: None,
            egress_service: None,
            message_retriever: None,
            session_store: None,
            agent_store: None,
            connection_resolver: None,
            schedule_store: None,
            subagent_delegate: None,
            extensions: ToolContextExtensions::default(),
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
            session_creation_authority: None,
            subagent_spawn_store: None,
            subagent_nesting_policy: SubagentNestingPolicy::default(),
            reasoning_effort_handle: None,
            cancellation: None,
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
            mcp_invoker: None,
            egress_service: None,
            message_retriever: None,
            session_store: None,
            agent_store: None,
            connection_resolver: None,
            schedule_store: None,
            subagent_delegate: None,
            extensions: ToolContextExtensions::default(),
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
            session_creation_authority: None,
            subagent_spawn_store: None,
            subagent_nesting_policy: SubagentNestingPolicy::default(),
            reasoning_effort_handle: None,
            cancellation: None,
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
            image_store: None,
            provider_credential_store: None,
            utility_llm_service: None,
            mcp_invoker: None,
            egress_service: None,
            message_retriever: None,
            session_store: None,
            agent_store: None,
            connection_resolver: None,
            schedule_store: None,
            subagent_delegate: None,
            extensions: ToolContextExtensions::default(),
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
            session_creation_authority: None,
            subagent_spawn_store: None,
            subagent_nesting_policy: SubagentNestingPolicy::default(),
            reasoning_effort_handle: None,
            cancellation: None,
        }
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

    /// Add a live reasoning-effort handle (EVE-595). Tools can call
    /// [`ReasoningEffortHandle::set`] on it to change the effort used by
    /// subsequent LLM steps within the same turn.
    /// Attach a cancellation token to this context (see
    /// [`ToolContext::cancellation`]).
    pub fn with_cancellation(mut self, token: tokio_util::sync::CancellationToken) -> Self {
        self.cancellation = Some(token);
        self
    }

    /// Whether this call has already been cancelled. A context with no token
    /// never reports cancellation.
    pub fn is_cancelled(&self) -> bool {
        self.cancellation
            .as_ref()
            .is_some_and(|token| token.is_cancelled())
    }

    pub fn with_reasoning_effort_handle(mut self, handle: ReasoningEffortHandle) -> Self {
        self.reasoning_effort_handle = Some(handle);
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
            mcp_invoker: None,
            egress_service: None,
            message_retriever: None,
            session_store: None,
            agent_store: None,
            connection_resolver: None,
            schedule_store: None,
            subagent_delegate: None,
            extensions: ToolContextExtensions::default(),
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
            session_creation_authority: None,
            subagent_spawn_store: None,
            subagent_nesting_policy: SubagentNestingPolicy::default(),
            reasoning_effort_handle: None,
            cancellation: None,
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

    /// Set the scoped-MCP tool invoker on this context.
    pub fn with_mcp_invoker(mut self, invoker: Arc<dyn crate::McpToolInvoker>) -> Self {
        self.mcp_invoker = Some(invoker);
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

    /// Add a narrow child-session delegate to this context (EVE-839).
    pub fn with_subagent_delegate(
        mut self,
        delegate: Arc<dyn crate::subagent_delegation::SubagentSessionDelegate>,
    ) -> Self {
        self.subagent_delegate = Some(delegate);
        self
    }

    /// Insert a type-keyed host extension (e.g. `everruns-platform`'s
    /// `PlatformStoreExt`) and return the updated context.
    pub fn with_extension<T: std::any::Any + Send + Sync>(mut self, value: Arc<T>) -> Self {
        self.extensions.insert(value);
        self
    }

    /// Resolve a type-keyed host extension previously inserted via
    /// [`Self::with_extension`].
    pub fn extension<T: std::any::Any + Send + Sync>(&self) -> Option<Arc<T>> {
        self.extensions.get::<T>()
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

    /// Set the resolved subagent nesting policy.
    pub fn with_subagent_nesting_policy(mut self, policy: SubagentNestingPolicy) -> Self {
        self.subagent_nesting_policy = policy;
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
            .field("message_retriever", &self.message_retriever.is_some())
            .field("session_store", &self.session_store.is_some())
            .field("agent_store", &self.agent_store.is_some())
            .field("connection_resolver", &self.connection_resolver.is_some())
            .field("schedule_store", &self.schedule_store.is_some())
            .field("subagent_delegate", &self.subagent_delegate.is_some())
            .field(
                "leased_resource_store",
                &self.leased_resource_store.is_some(),
            )
            .field("event_emitter", &self.event_emitter.is_some())
            .field("tool_registry", &self.tool_registry.is_some())
            .field("payment_authority", &self.payment_authority.is_some())
            .field("subagent_spawn_store", &self.subagent_spawn_store.is_some())
            .field("subagent_nesting_policy", &self.subagent_nesting_policy)
            .field("org_id", &self.org_id)
            .finish()
    }
}

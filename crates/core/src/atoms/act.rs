//! ActAtom - Atom for scheduled tool execution
//!
//! This atom handles:
//! 1. Emitting act.started event
//! 2. Executing the batch of tool calls via the [`tool_scheduler`] (with
//!    tool.started/completed events). Calls run concurrently by default, but
//!    calls that share a [`crate::tool_types::ToolHints::concurrency_class`] are
//!    serialized to avoid mutation races, total concurrency is capped, and
//!    `cpu_bound` tools are offloaded to their own task.
//! 3. Handling errors, timeouts, and cancellations as "normal" results
//! 4. Emitting act.completed event
//! 5. Returning all tool results (success, error, timeout, or cancelled)
//!
//! Tool results are emitted as `tool.completed` events and returned in ActResult.
//! Messages are derived from events - no separate message storage is needed.
//!
//! Note: OTel instrumentation is handled via the event-listener pattern.
//! tool.started/completed events are emitted by this atom, and OtelEventListener
//! creates the appropriate gen-ai spans from those events.
//!
//! NOTES from Python spec:
//! - Tool calls run concurrently by default; the scheduler serializes only
//!   conflicting (same-concurrency-class) calls. See [`tool_scheduler`].
//! - Error from tool call is not an error for the whole Act, error from tool is "normal" result
//! - Tool invocations should be timeouted, timeout is also "normal" result from tool
//! - Exit of act should have all tool calls finished (successfully or with error/timeout)
//! - Act and each tool call should emit start/end events
//! - Act and each tool call should be cancellable, and this is also "normal" result

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Instant;

use super::act_hooks::{self, PostActHook};
use super::tool_scheduler;
use super::{Atom, AtomContext};
use crate::error::Result;
use crate::events::{
    ActCompletedData, ActStartedData, EventContext, EventRequest, ToolCompletedData,
    ToolStartedData,
};
use crate::message::ContentPart;
use crate::tool_fingerprint::{
    tool_call_fingerprint, tool_error_fingerprint, tool_result_fingerprint,
};
use crate::tool_narration::{
    ToolNarrationPhase, render_group_headline_with_locale, render_tool_narration_with_locale,
};
use crate::tool_types::{ToolCall, ToolDefinition, ToolResult};
use crate::traits::{
    AgentStore, EventEmitter, SessionFileSystem, SessionMutator, SessionStore, ToolContext,
    ToolExecutor,
};
use crate::typed_id::{AgentId, HarnessId};
use uuid::Uuid;

/// A Tokio task handle that aborts its task if the parent future is dropped
/// before the task completes. Tokio detaches a bare [`tokio::task::JoinHandle`]
/// on drop, but tool execution must not outlive Act cancellation.
struct AbortOnDropJoinHandle<T> {
    handle: tokio::task::JoinHandle<T>,
}

impl<T> AbortOnDropJoinHandle<T> {
    fn new(handle: tokio::task::JoinHandle<T>) -> Self {
        Self { handle }
    }
}

impl<T> Future for AbortOnDropJoinHandle<T> {
    type Output = std::result::Result<T, tokio::task::JoinError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.handle).poll(cx)
    }
}

impl<T> Drop for AbortOnDropJoinHandle<T> {
    fn drop(&mut self) {
        if !self.handle.is_finished() {
            self.handle.abort();
        }
    }
}

// ============================================================================
// Input and Output Types
// ============================================================================

/// Input for ActAtom
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActInput {
    /// Organization ID for scoped data access.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub org_id: Option<i64>,
    /// Atom execution context
    pub context: AtomContext,
    /// Harness ID (needed for scheduling follow-up reason activity)
    pub harness_id: HarnessId,
    /// Agent ID (needed for scheduling follow-up reason activity, optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<AgentId>,
    /// Tool calls to execute
    pub tool_calls: Vec<ToolCall>,
    /// Available tool definitions for resolution
    pub tool_definitions: Vec<ToolDefinition>,
    /// Resolved locale for backend-authored tool narration and labels.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
    /// Blueprint ID for blueprint-backed sessions. When set, act_activity
    /// loads tools from the blueprint instead of from agent/harness capabilities.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blueprint_id: Option<String>,
    /// Merged network access list (harness ∩ agent ∩ session) for URL filtering.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_access: Option<crate::network_access::NetworkAccessList>,
    /// Mirrors the request's `parallel_tool_calls`. `Some(false)` forces the
    /// act scheduler to execute this batch strictly sequentially; `None` or
    /// `Some(true)` uses the default class-aware concurrent schedule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallel_tool_calls: Option<bool>,
}

/// Result of a single tool call execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallResult {
    /// The original tool call
    pub tool_call: ToolCall,
    /// The result of the tool call
    pub result: ToolResult,
    /// Whether the execution was successful
    pub success: bool,
    /// Status: "success", "error", "timeout", or "cancelled"
    pub status: String,
    /// If set, the tool requires a user connection for this provider before it can execute.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection_required: Option<String>,
}

/// Result of the ActAtom
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActResult {
    /// Results for all tool calls
    pub results: Vec<ToolCallResult>,
    /// Whether all tool calls completed (regardless of success/failure)
    pub completed: bool,
    /// Number of successful tool calls
    pub success_count: u32,
    /// Number of failed tool calls
    pub error_count: u32,
    /// When true, the act emitted client-side tool calls (connection setup,
    /// client-side tools, etc.) and the worker should pause until tool results
    /// arrive. Workers check this single flag — they never need to know *why*
    /// the act paused.
    #[serde(default)]
    pub waiting_for_tool_results: bool,
    /// True when execution stopped before tool execution because a dependency was archived or deleted.
    #[serde(default, skip_serializing_if = "is_false")]
    pub blocked: bool,
    /// Client-side tool calls that were NOT executed by ActAtom but need to be
    /// sent to the client. Populated by ActAtom's partitioning logic, consumed
    /// by ClientSideToolHook.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub client_tool_calls: Vec<ToolCall>,
    /// Tool definitions for the client-side tool calls (for narration/display).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub client_tool_definitions: Vec<ToolDefinition>,
}

fn is_false(value: &bool) -> bool {
    !*value
}

// ============================================================================
// ActAtom
// ============================================================================

/// Atom that executes a batch of tool calls via the [`tool_scheduler`]
///
/// This atom:
/// 1. Emits act.started event
/// 2. Schedules all tool calls (emitting tool.started/completed for each):
///    concurrent by default, serialized within a concurrency class, capped, and
///    with `cpu_bound` tools offloaded to their own task
/// 3. Handles errors, timeouts, and cancellations gracefully
/// 4. Emits act.completed event
/// 5. Returns comprehensive results for all tools
///
/// Tool results are emitted as events and returned in ActResult.
/// Messages are derived from events by the message store.
pub struct ActAtom<T, E>
where
    T: ToolExecutor,
    E: EventEmitter,
{
    // Held as `Arc` so individual `cpu_bound` tool calls can be offloaded to
    // their own task (`tokio::spawn`) without borrowing `self` for `'static`.
    tool_executor: Arc<T>,
    event_emitter: Arc<E>,
    /// Optional file store for context-aware tools
    file_store: Option<Arc<dyn SessionFileSystem>>,
    /// Optional SQL database store for sql_execute/sql_query/sql_schema tools
    sqldb_store: Option<crate::traits::SessionSqlDbStoreRef>,
    /// Optional session storage store for kv_store/secret_store tools
    storage_store: Option<Arc<dyn crate::traits::SessionStorageStore>>,
    /// Optional image artifact store for durable image persistence
    image_store: Option<Arc<dyn crate::traits::ImageArtifactStore>>,
    /// Optional provider credential store for tool-side API clients
    provider_credential_store: Option<Arc<dyn crate::traits::ProviderCredentialStore>>,
    /// Optional utility LLM service for capability internals
    utility_llm_service: Option<Arc<dyn crate::UtilityLlmService>>,
    /// Optional outbound egress service for HTTP/API traffic.
    egress_service: Option<Arc<dyn crate::EgressService>>,
    /// Optional resolver for user connection tokens
    connection_resolver: Option<Arc<dyn crate::traits::UserConnectionResolver>>,
    /// Optional session store for session metadata reads
    session_store: Option<Arc<dyn SessionStore>>,
    /// Optional session mutator for session metadata updates
    session_mutator: Option<Arc<dyn SessionMutator>>,
    /// Optional agent store for agent metadata reads
    agent_store: Option<Arc<dyn AgentStore>>,
    /// Optional session schedule store for scheduling tools
    schedule_store: Option<Arc<dyn crate::traits::SessionScheduleStore>>,
    /// Optional platform store for org-level management tools
    platform_store: Option<Arc<dyn crate::platform_store::PlatformStore>>,
    /// Optional leased resource store for provider lease tracking
    leased_resource_store: Option<Arc<dyn crate::traits::LeasedResourceStore>>,
    /// Optional session resource registry
    session_resource_registry: Option<Arc<dyn crate::traits::SessionResourceRegistry>>,
    /// Optional capability registry for blueprint lookups in subagent tools
    capability_registry: Option<crate::capabilities::CapabilityRegistry>,
    /// Optional built-in tool registry for meta-tools that delegate to sibling tools.
    tool_registry: Option<Arc<crate::tools::ToolRegistry>>,
    /// Optional memory store backend for persistent cross-session memory.
    memory_store: Option<Arc<dyn crate::memory_store::MemoryStoreBackend>>,
    /// Optional org ID for org-scoped operations.
    org_id: Option<crate::typed_id::OrgId>,
    /// Merged network access list for URL filtering in tools.
    network_access: Option<crate::network_access::NetworkAccessList>,
    /// Optional budget checker for the check_budget tool.
    budget_checker: Option<Arc<dyn crate::traits::BudgetChecker>>,
    /// Optional internal payment authority for paid capability tools.
    payment_authority: Option<Arc<dyn crate::traits::PaymentAuthority>>,
    /// Optional per-org outbound tool-call rate limiter (TM-TOOL-009).
    /// When present, each tool call increments the org counter; calls that
    /// exceed the per-org window return a tool error rather than a hard failure.
    outbound_tool_rate_limiter: Option<Arc<dyn crate::traits::OutboundToolRateLimiter>>,
    /// Post-act hooks that run after tool execution completes.
    /// Hooks inspect the result and may emit events (e.g. tool.call_requested).
    hooks: Vec<Box<dyn PostActHook>>,
    /// Post-tool-exec hooks (capability-contributed): run after each individual
    /// tool execution. Capabilities register these via `post_tool_exec_hooks()`.
    post_tool_hooks: Vec<Arc<dyn act_hooks::PostToolExecHook>>,
    /// Pre-tool-use hooks (capability-contributed): run before each individual
    /// tool execution. Capabilities wire these in via the user-hooks
    /// adapter chain (see `crate::hook_adapter`). Hooks can mutate the
    /// `ToolCall` (returning `Continue`) or refuse execution
    /// (returning `Block`).
    pre_tool_hooks: Vec<Arc<dyn act_hooks::PreToolUseHook>>,
    /// Tool-call hooks (capability-contributed): inspect model-authored tool
    /// calls for UI narration and transform calls before actual execution.
    tool_call_hooks: Vec<Arc<dyn crate::capabilities::ToolCallHook>>,
    /// Final post-tool-exec hooks (infrastructure): run after capability hooks.
    /// Always registered, cannot be removed by capabilities (EVE-225).
    final_post_tool_hooks: Vec<Arc<dyn act_hooks::PostToolExecHook>>,
}

impl<T, E> ActAtom<T, E>
where
    T: ToolExecutor,
    E: EventEmitter,
{
    /// Create a new ActAtom with default hooks (ConnectionSetup + ClientSideTool).
    pub fn new(tool_executor: T, event_emitter: E) -> Self {
        Self {
            tool_executor: Arc::new(tool_executor),
            event_emitter: Arc::new(event_emitter),
            file_store: None,
            sqldb_store: None,
            storage_store: None,
            image_store: None,
            provider_credential_store: None,
            utility_llm_service: None,
            egress_service: None,
            connection_resolver: None,
            session_store: None,
            session_mutator: None,
            agent_store: None,
            schedule_store: None,
            platform_store: None,
            leased_resource_store: None,
            session_resource_registry: None,
            capability_registry: None,
            tool_registry: None,
            memory_store: None,
            org_id: None,
            network_access: None,
            budget_checker: None,
            payment_authority: None,
            outbound_tool_rate_limiter: None,
            hooks: Self::default_hooks(),
            post_tool_hooks: Vec::new(),
            pre_tool_hooks: Vec::new(),
            tool_call_hooks: Vec::new(),
            final_post_tool_hooks: Self::default_final_hooks(),
        }
    }

    /// Create a new ActAtom with a file store for context-aware tools
    pub fn with_file_store(
        tool_executor: T,
        event_emitter: E,
        file_store: Arc<dyn SessionFileSystem>,
    ) -> Self {
        Self {
            tool_executor: Arc::new(tool_executor),
            event_emitter: Arc::new(event_emitter),
            file_store: Some(file_store),
            sqldb_store: None,
            storage_store: None,
            image_store: None,
            provider_credential_store: None,
            utility_llm_service: None,
            egress_service: None,
            connection_resolver: None,
            session_store: None,
            session_mutator: None,
            agent_store: None,
            schedule_store: None,
            platform_store: None,
            leased_resource_store: None,
            session_resource_registry: None,
            capability_registry: None,
            tool_registry: None,
            memory_store: None,
            org_id: None,
            network_access: None,
            budget_checker: None,
            payment_authority: None,
            outbound_tool_rate_limiter: None,
            hooks: Self::default_hooks(),
            post_tool_hooks: Vec::new(),
            pre_tool_hooks: Vec::new(),
            tool_call_hooks: Vec::new(),
            final_post_tool_hooks: Self::default_final_hooks(),
        }
    }

    /// Add a custom post-act hook.
    pub fn with_hook(mut self, hook: Box<dyn PostActHook>) -> Self {
        self.hooks.push(hook);
        self
    }

    /// Default hooks: ConnectionSetup (synthetic setup_connection calls)
    /// and ClientSideTool (emit tool.call_requested for client-side tools).
    fn default_hooks() -> Vec<Box<dyn PostActHook>> {
        vec![
            Box::new(act_hooks::ConnectionSetupHook),
            Box::new(act_hooks::ClientSideToolHook),
        ]
    }

    /// Default final post-tool-exec hooks (infrastructure, always-on).
    /// These run after all capability-contributed hooks and cannot be removed.
    fn default_final_hooks() -> Vec<Arc<dyn act_hooks::PostToolExecHook>> {
        vec![
            Arc::new(crate::capabilities::PersistOutputHook),
            Arc::new(act_hooks::OutputHardLimitHook),
        ]
    }

    /// Set the SQL database store on this atom
    pub fn with_sqldb_store(mut self, store: crate::traits::SessionSqlDbStoreRef) -> Self {
        self.sqldb_store = Some(store);
        self
    }

    /// Set the session storage store on this atom
    pub fn with_storage_store(
        mut self,
        store: Arc<dyn crate::traits::SessionStorageStore>,
    ) -> Self {
        self.storage_store = Some(store);
        self
    }

    /// Set the image artifact store on this atom
    pub fn with_image_store(mut self, store: Arc<dyn crate::traits::ImageArtifactStore>) -> Self {
        self.image_store = Some(store);
        self
    }

    /// Set the provider credential store on this atom
    pub fn with_provider_credential_store(
        mut self,
        store: Arc<dyn crate::traits::ProviderCredentialStore>,
    ) -> Self {
        self.provider_credential_store = Some(store);
        self
    }

    /// Set the utility LLM service on this atom.
    pub fn with_utility_llm_service(mut self, service: Arc<dyn crate::UtilityLlmService>) -> Self {
        self.utility_llm_service = Some(service);
        self
    }

    /// Set the outbound egress service on this atom.
    pub fn with_egress_service(mut self, service: Arc<dyn crate::EgressService>) -> Self {
        self.egress_service = Some(service);
        self
    }

    /// Set the user connection resolver on this atom
    pub fn with_connection_resolver(
        mut self,
        resolver: Arc<dyn crate::traits::UserConnectionResolver>,
    ) -> Self {
        self.connection_resolver = Some(resolver);
        self
    }

    /// Set session store for context-aware tools.
    pub fn with_session_store(mut self, store: Arc<dyn SessionStore>) -> Self {
        self.session_store = Some(store);
        self
    }

    /// Set session mutator for context-aware tools.
    pub fn with_session_mutator(mut self, mutator: Arc<dyn SessionMutator>) -> Self {
        self.session_mutator = Some(mutator);
        self
    }

    /// Set agent store for context-aware tools.
    pub fn with_agent_store(mut self, store: Arc<dyn AgentStore>) -> Self {
        self.agent_store = Some(store);
        self
    }

    /// Set session schedule store for scheduling tools.
    pub fn with_schedule_store(
        mut self,
        store: Arc<dyn crate::traits::SessionScheduleStore>,
    ) -> Self {
        self.schedule_store = Some(store);
        self
    }

    /// Set platform store for org-level management tools.
    pub fn with_platform_store(
        mut self,
        store: Arc<dyn crate::platform_store::PlatformStore>,
    ) -> Self {
        self.platform_store = Some(store);
        self
    }

    /// Set leased resource store for lifecycle-managed provider resources.
    pub fn with_leased_resource_store(
        mut self,
        store: Arc<dyn crate::traits::LeasedResourceStore>,
    ) -> Self {
        self.leased_resource_store = Some(store);
        self
    }

    /// Set session resource registry.
    pub fn with_session_resource_registry(
        mut self,
        registry: Arc<dyn crate::traits::SessionResourceRegistry>,
    ) -> Self {
        self.session_resource_registry = Some(registry);
        self
    }

    pub fn with_capability_registry(
        mut self,
        registry: crate::capabilities::CapabilityRegistry,
    ) -> Self {
        self.capability_registry = Some(registry);
        self
    }

    /// Set the active built-in tool registry for meta-tools like `spawn_background`.
    pub fn with_tool_registry(mut self, registry: Arc<crate::tools::ToolRegistry>) -> Self {
        self.tool_registry = Some(registry);
        self
    }

    /// Add capability-contributed post-tool-exec hooks.
    /// Callers should pass hooks from the *active* capabilities for this session,
    /// not from the full platform registry.
    pub fn with_post_tool_hooks(
        mut self,
        hooks: Vec<Arc<dyn act_hooks::PostToolExecHook>>,
    ) -> Self {
        self.post_tool_hooks.extend(hooks);
        self
    }

    /// Add capability-contributed pre-tool-use hooks. Pre-hooks fire before
    /// each tool call and can mutate or block it; see
    /// `act_hooks::PreToolUseHook` and `specs/user-hooks.md`.
    pub fn with_pre_tool_hooks(mut self, hooks: Vec<Arc<dyn act_hooks::PreToolUseHook>>) -> Self {
        self.pre_tool_hooks.extend(hooks);
        self
    }

    pub fn with_tool_call_hooks(
        mut self,
        hooks: Vec<Arc<dyn crate::capabilities::ToolCallHook>>,
    ) -> Self {
        self.tool_call_hooks.extend(hooks);
        self
    }

    /// Set memory store backend for persistent cross-session memory tools.
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

    /// Set the merged network access list for URL filtering in tools.
    pub fn with_network_access(
        mut self,
        network_access: Option<crate::network_access::NetworkAccessList>,
    ) -> Self {
        self.network_access = network_access;
        self
    }

    /// Set the budget checker for the check_budget tool.
    pub fn with_budget_checker(mut self, checker: Arc<dyn crate::traits::BudgetChecker>) -> Self {
        self.budget_checker = Some(checker);
        self
    }

    /// Set the internal payment authority for paid capability tools.
    pub fn with_payment_authority(
        mut self,
        authority: Arc<dyn crate::traits::PaymentAuthority>,
    ) -> Self {
        self.payment_authority = Some(authority);
        self
    }

    /// Set the per-org outbound tool-call rate limiter (TM-TOOL-009).
    pub fn with_outbound_tool_rate_limiter(
        mut self,
        limiter: Arc<dyn crate::traits::OutboundToolRateLimiter>,
    ) -> Self {
        self.outbound_tool_rate_limiter = Some(limiter);
        self
    }
}

#[async_trait]
impl<T, E> Atom for ActAtom<T, E>
where
    T: ToolExecutor + Send + Sync + 'static,
    E: EventEmitter + Send + Sync + 'static,
{
    type Input = ActInput;
    type Output = ActResult;

    fn name(&self) -> &'static str {
        "act"
    }

    async fn execute(&self, input: Self::Input) -> Result<Self::Output> {
        let ActInput {
            context,
            tool_calls,
            tool_definitions,
            locale,
            network_access,
            parallel_tool_calls,
            .. // agent_id/org_id not needed here, just passed through workflow
        } = input;

        // Partition tool calls: server-side tools get executed, client-side tools
        // are stored on ActResult for the ClientSideToolHook to emit.
        let (server_tool_calls, client_tool_calls): (Vec<_>, Vec<_>) =
            tool_calls.into_iter().partition(|tc| {
                tool_definitions
                    .iter()
                    .find(|td| td.name() == tc.name)
                    .map(|td| !matches!(td, ToolDefinition::ClientSide(_)))
                    .unwrap_or(true) // unknown tools go to server (will error there)
            });

        let client_tool_calls: Vec<_> = client_tool_calls
            .into_iter()
            .map(|tool_call| self.transform_tool_call_for_execution(tool_call))
            .collect();

        let client_tool_definitions: Vec<_> = if client_tool_calls.is_empty() {
            vec![]
        } else {
            tool_definitions
                .iter()
                .filter(|td| {
                    if let ToolDefinition::ClientSide(ct) = td {
                        client_tool_calls.iter().any(|tc| tc.name == ct.name)
                    } else {
                        false
                    }
                })
                .cloned()
                .collect()
        };

        if server_tool_calls.is_empty() && client_tool_calls.is_empty() {
            return Ok(ActResult {
                results: vec![],
                completed: true,
                success_count: 0,
                error_count: 0,
                waiting_for_tool_results: false,
                blocked: false,
                client_tool_calls: vec![],
                client_tool_definitions: vec![],
            });
        }

        // If only client-side tools (no server-side), skip tool execution entirely.
        // Just run hooks to emit tool.call_requested.
        if server_tool_calls.is_empty() {
            let mut result = ActResult {
                results: vec![],
                completed: true,
                success_count: 0,
                error_count: 0,
                waiting_for_tool_results: false,
                blocked: false,
                client_tool_calls,
                client_tool_definitions,
            };
            act_hooks::run_post_act_hooks(
                &self.hooks,
                &context,
                &mut result,
                &tool_definitions,
                &self.event_emitter,
                locale.as_deref(),
            )
            .await;
            return Ok(result);
        }

        // Replace tool_calls with only server-side tools for execution
        let tool_calls = server_tool_calls;

        tracing::info!(
            session_id = %context.session_id,
            turn_id = %context.turn_id,
            exec_id = %context.exec_id,
            tool_count = %tool_calls.len(),
            "ActAtom: executing tools in parallel"
        );

        // Generate OTel-style span IDs for hierarchical tracing
        // trace_id: groups all events in this turn
        // span_id: unique identifier for this act span (shared by started/completed)
        // parent_span_id: links to turn as parent
        //
        // NOTE: TurnId::to_string() returns prefixed format (e.g., "turn_abc123")
        // matching the format used by turn.started/completed events in Braintrust.
        let trace_id = context.turn_id.to_string();
        let act_span_id = Uuid::now_v7().to_string();
        let parent_span_id = trace_id.clone(); // Parent is the turn

        // Create event context from atom context with span info
        let event_context = EventContext::from_atom_context(&context).with_span(
            trace_id.clone(),
            act_span_id.clone(),
            Some(parent_span_id.clone()),
        );

        // Track act phase timing for Braintrust observability
        let act_start = Instant::now();

        let visible_tool_names = Arc::new(
            tool_definitions
                .iter()
                .map(|def| def.name().to_string())
                .collect::<HashSet<_>>(),
        );

        // Build tool name to definition map
        let tool_map: std::collections::HashMap<&str, &ToolDefinition> = tool_definitions
            .iter()
            .map(|def| {
                let name = def.name();
                (name, def)
            })
            .collect();

        let mut started_data = ActStartedData::with_definitions_and_locale(
            &tool_calls,
            &tool_definitions,
            locale.as_deref(),
        );
        for summary in &mut started_data.tool_calls {
            if let Some(tool_call) = tool_calls.iter().find(|tc| tc.id == summary.id) {
                let tool_def = tool_map.get(tool_call.name.as_str()).copied();
                summary.narration = Some(self.render_tool_narration(
                    tool_def,
                    tool_call,
                    ToolNarrationPhase::Started,
                    locale.as_deref(),
                ));
            }
        }
        if tool_calls.len() == 1 {
            started_data.headline = started_data
                .tool_calls
                .first()
                .and_then(|summary| summary.narration.clone());
        }

        // Emit act.started event (with display names from tool definitions)
        if let Err(e) = self
            .event_emitter
            .emit(EventRequest::new(
                context.session_id,
                event_context.clone(),
                started_data,
            ))
            .await
        {
            tracing::warn!(
                session_id = %context.session_id,
                error = %e,
                "ActAtom: failed to emit act.started event"
            );
        }

        // Decide the execution schedule from per-tool metadata. Calls that
        // share a concurrency class (mutations to the same shared resource) run
        // sequentially in arrival order; everything else runs concurrently,
        // bounded by a global cap. `parallel_tool_calls == Some(false)` forces a
        // fully sequential schedule. Each tool event references the act span as
        // its parent regardless of scheduling.
        let classes: Vec<Option<String>> = tool_calls
            .iter()
            .map(|tool_call| {
                tool_map
                    .get(tool_call.name.as_str())
                    .and_then(|def| def.concurrency_class())
                    .map(|class| class.to_string())
            })
            .collect();
        let schedule_config = tool_scheduler::ScheduleConfig {
            serialize_all: parallel_tool_calls == Some(false),
            ..tool_scheduler::ScheduleConfig::default()
        };
        let results =
            tool_scheduler::schedule(tool_calls.len(), &classes, schedule_config, |index| {
                let tool_call = &tool_calls[index];
                let tool_def = tool_map.get(tool_call.name.as_str()).cloned();
                self.execute_single_tool(
                    &context,
                    tool_call.clone(),
                    tool_def,
                    &trace_id,
                    &act_span_id,
                    locale.as_deref(),
                    network_access.as_ref(),
                    visible_tool_names.clone(),
                )
            })
            .await;

        // Count successes and errors
        let success_count = results.iter().filter(|r| r.success).count() as u32;
        let error_count = results.iter().filter(|r| !r.success).count() as u32;

        // Calculate act phase duration
        let act_duration_ms = act_start.elapsed().as_millis() as u64;

        // Emit act.completed event (same span as act.started, parent is turn)
        let completed_context = EventContext::from_atom_context(&context).with_span(
            trace_id.clone(),
            act_span_id.clone(), // Same span_id as started
            Some(parent_span_id.clone()),
        );
        let mut completed_headline = render_group_headline_with_locale(
            &tool_calls,
            &tool_definitions,
            ToolNarrationPhase::Completed,
            locale.as_deref(),
        );
        if tool_calls.len() == 1
            && let Some(tool_call) = tool_calls.first()
        {
            completed_headline = Some(self.render_tool_narration(
                tool_map.get(tool_call.name.as_str()).copied(),
                tool_call,
                ToolNarrationPhase::Completed,
                locale.as_deref(),
            ));
        }
        if error_count > 0 {
            let suffix = crate::localization::format_error_suffix(locale.as_deref(), error_count);
            completed_headline = Some(match completed_headline {
                Some(text) => format!("{text}{suffix}"),
                None => {
                    crate::localization::format_completed_tool_batch(locale.as_deref(), error_count)
                }
            });
        }

        if let Err(e) = self
            .event_emitter
            .emit(EventRequest::new(
                context.session_id,
                completed_context,
                ActCompletedData {
                    completed: true,
                    success_count,
                    error_count,
                    duration_ms: Some(act_duration_ms),
                    headline: completed_headline,
                },
            ))
            .await
        {
            tracing::warn!(
                session_id = %context.session_id,
                error = %e,
                "ActAtom: failed to emit act.completed event"
            );
        }

        tracing::info!(
            session_id = %context.session_id,
            turn_id = %context.turn_id,
            success_count = %success_count,
            error_count = %error_count,
            "ActAtom: all tools completed"
        );

        let mut act_result = ActResult {
            results,
            completed: true,
            success_count,
            error_count,
            waiting_for_tool_results: false,
            blocked: false,
            client_tool_calls,
            client_tool_definitions,
        };

        // Run post-act hooks (connection setup, client-side tool emission, etc.)
        act_hooks::run_post_act_hooks(
            &self.hooks,
            &context,
            &mut act_result,
            &tool_definitions,
            &self.event_emitter,
            locale.as_deref(),
        )
        .await;

        Ok(act_result)
    }
}

impl<T, E> ActAtom<T, E>
where
    T: ToolExecutor + Send + Sync + 'static,
    E: EventEmitter + Send + Sync + 'static,
{
    fn render_tool_narration(
        &self,
        tool_def: Option<&ToolDefinition>,
        tool_call: &ToolCall,
        phase: ToolNarrationPhase,
        locale: Option<&str>,
    ) -> String {
        for hook in &self.tool_call_hooks {
            if let Some(narration) = hook.narration(tool_def, tool_call, phase, locale) {
                return narration;
            }
        }
        render_tool_narration_with_locale(tool_def, tool_call, phase, locale)
    }

    fn transform_tool_call_for_execution(&self, tool_call: ToolCall) -> ToolCall {
        self.tool_call_hooks
            .iter()
            .fold(tool_call, |tool_call, hook| {
                hook.transform_for_execution(tool_call)
            })
    }

    /// Execute a single tool call
    ///
    /// Note: OTel instrumentation is handled via event listeners.
    /// tool.started/completed events are emitted, and OtelEventListener
    /// creates gen-ai spans from those events.
    #[allow(clippy::too_many_arguments)]
    async fn execute_single_tool(
        &self,
        context: &AtomContext,
        tool_call: ToolCall,
        tool_def: Option<&ToolDefinition>,
        trace_id: &str,
        act_span_id: &str,
        locale: Option<&str>,
        network_access: Option<&crate::network_access::NetworkAccessList>,
        visible_tool_names: Arc<HashSet<String>>,
    ) -> ToolCallResult {
        tracing::debug!(
            session_id = %context.session_id,
            turn_id = %context.turn_id,
            tool_name = %tool_call.name,
            tool_call_id = %tool_call.id,
            "ActAtom: executing tool"
        );

        // Generate a unique span_id for this tool call (child of act span)
        let tool_span_id = Uuid::now_v7().to_string();

        // Create event context from atom context (with act span as parent)
        let event_context = EventContext::from_atom_context(context).with_span(
            trace_id.to_string(),
            tool_span_id.clone(),
            Some(act_span_id.to_string()),
        );

        // Track tool call timing for Braintrust observability
        let tool_start = Instant::now();
        let tool_call_fingerprint = tool_call_fingerprint(&tool_call);

        // Resolve display name from tool definition
        let display_name = crate::localization::localized_tool_display_name(
            &tool_call.name,
            tool_def.and_then(|d| d.display_name()),
            locale,
        );
        let capability_attribution = tool_def.and_then(|def| {
            def.capability_attribution()
                .map(|(id, name)| (id.to_string(), name.map(str::to_string)))
        });

        // Per-org outbound tool-call rate limiting (TM-TOOL-009).
        // Checked before tool.started so a denied call emits no events and leaves
        // no unmatched started/completed pair in UI or telemetry.
        if let (Some(limiter), Some(ref org_id)) = (&self.outbound_tool_rate_limiter, self.org_id)
            && !limiter.check_org(org_id).await
        {
            tracing::warn!(
                session_id = %context.session_id,
                tool_name = %tool_call.name,
                "ActAtom: outbound tool rate limit exceeded for org"
            );
            return ToolCallResult {
                tool_call: tool_call.clone(),
                result: ToolResult {
                    tool_call_id: tool_call.id.clone(),
                    result: None,
                    images: None,
                    error: Some(
                        "Outbound tool rate limit exceeded for this organization; back off and retry later.".to_string(),
                    ),
                    connection_required: None,
                    raw_output: None,
                },
                success: false,
                status: "error".to_string(),
                connection_required: None,
            };
        }

        // Emit tool.started event (child of act.started)
        if let Err(e) = self
            .event_emitter
            .emit(EventRequest::new(
                context.session_id,
                event_context.clone(),
                ToolStartedData {
                    tool_call: tool_call.clone(),
                    tool_call_fingerprint: Some(tool_call_fingerprint.clone()),
                    display_name: display_name.clone(),
                    narration: Some(self.render_tool_narration(
                        tool_def,
                        &tool_call,
                        ToolNarrationPhase::Started,
                        locale,
                    )),
                },
            ))
            .await
        {
            tracing::warn!(
                session_id = %context.session_id,
                tool_call_id = %tool_call.id,
                error = %e,
                "ActAtom: failed to emit tool.started event"
            );
        }

        // If tool definition not found, return error result
        let Some(tool_def) = tool_def else {
            let error_msg = format!("Tool definition not found: {}", tool_call.name);
            let tool_duration_ms = tool_start.elapsed().as_millis() as u64;

            // Emit tool.completed event for error (child of act.started)
            if let Err(e) = self
                .event_emitter
                .emit(EventRequest::new(
                    context.session_id,
                    event_context,
                    ToolCompletedData::failure(
                        tool_call.id.clone(),
                        tool_call.name.clone(),
                        "error".to_string(),
                        error_msg.clone(),
                        Some(tool_duration_ms),
                    )
                    .with_fingerprints(
                        tool_call_fingerprint.clone(),
                        tool_error_fingerprint(&tool_call.name, "error", &error_msg),
                    )
                    .with_narration(Some(self.render_tool_narration(
                        None,
                        &tool_call,
                        ToolNarrationPhase::Failed,
                        locale,
                    ))),
                ))
                .await
            {
                tracing::warn!(
                    session_id = %context.session_id,
                    tool_call_id = %tool_call.id,
                    error = %e,
                    "ActAtom: failed to emit tool.completed event"
                );
            }

            return ToolCallResult {
                tool_call: tool_call.clone(),
                result: ToolResult {
                    tool_call_id: tool_call.id.clone(),
                    result: None,
                    images: None,
                    error: Some(error_msg),
                    connection_required: None,
                    raw_output: None,
                },
                success: false,
                status: "error".to_string(),
                connection_required: None,
            };
        };

        // Execute the tool (always with context so tools can emit progress events)
        let mut tool_context = if let Some(ref store) = self.file_store {
            ToolContext::with_file_store(context.session_id, store.clone())
        } else {
            ToolContext::new(context.session_id)
        };
        if let Some(ref store) = self.sqldb_store {
            tool_context.sqldb_store = Some(store.clone());
        }
        if let Some(ref store) = self.storage_store {
            tool_context.storage_store = Some(store.clone());
        }
        if let Some(ref store) = self.image_store {
            tool_context.image_store = Some(store.clone());
        }
        if let Some(ref store) = self.provider_credential_store {
            tool_context.provider_credential_store = Some(store.clone());
        }
        if let Some(ref service) = self.utility_llm_service {
            tool_context.utility_llm_service = Some(service.clone());
        }
        if let Some(ref service) = self.egress_service {
            tool_context.egress_service = Some(service.clone());
        }
        if let Some(ref resolver) = self.connection_resolver {
            tool_context.connection_resolver = Some(resolver.clone());
        }
        if let Some(ref store) = self.session_store {
            tool_context.session_store = Some(store.clone());
        }
        if let Some(ref mutator) = self.session_mutator {
            tool_context.session_mutator = Some(mutator.clone());
        }
        if let Some(ref store) = self.agent_store {
            tool_context.agent_store = Some(store.clone());
        }
        if let Some(ref store) = self.schedule_store {
            tool_context.schedule_store = Some(store.clone());
        }
        if let Some(ref store) = self.platform_store {
            tool_context.platform_store = Some(store.clone());
        }
        if let Some(ref store) = self.leased_resource_store {
            tool_context.leased_resource_store = Some(store.clone());
        }
        if let Some(ref registry) = self.session_resource_registry {
            tool_context.session_resource_registry = Some(registry.clone());
        }
        if let Some(ref registry) = self.capability_registry {
            tool_context.capability_registry = Some(registry.clone());
        }
        if let Some(ref registry) = self.tool_registry {
            tool_context.tool_registry = Some(registry.clone());
        }
        tool_context.visible_tool_names = Some(visible_tool_names.clone());
        if let Some(ref store) = self.memory_store {
            tool_context.memory_store = Some(store.clone());
        }
        if let Some(ref checker) = self.budget_checker {
            tool_context.budget_checker = Some(checker.clone());
        }
        if let Some(ref authority) = self.payment_authority {
            tool_context.payment_authority = Some(authority.clone());
        }
        tool_context.org_id = self.org_id;
        // Input network_access (per-session, merged from harness+agent+session) takes precedence
        tool_context.network_access = network_access
            .cloned()
            .or_else(|| self.network_access.clone());
        // Provide event emitter + context so tools can emit tool.progress events
        tool_context.event_emitter = Some(self.event_emitter.clone() as Arc<dyn EventEmitter>);
        tool_context.event_context = Some(event_context.clone());
        tool_context.tool_call_id = Some(tool_call.id.clone());

        let execution_tool_call = self.transform_tool_call_for_execution(tool_call.clone());

        // Run pre-tool-use hooks (capability-contributed). They can mutate
        // the tool call or block execution entirely. First Block wins; the
        // tool is not invoked, and the synthetic error result flows through
        // the same completion/event path as a tool failure.
        let (execution_tool_call, pre_block_reason) = if self.pre_tool_hooks.is_empty() {
            (execution_tool_call, None)
        } else {
            match act_hooks::run_pre_tool_use_hooks(
                &self.pre_tool_hooks,
                execution_tool_call.clone(),
                tool_def,
                &tool_context,
            )
            .await
            {
                act_hooks::PreToolUseDecision::Continue(updated) => (updated, None),
                act_hooks::PreToolUseDecision::Block {
                    tool_call: blocked,
                    reason,
                    ..
                } => (blocked, Some(reason)),
            }
        };

        let result = if let Some(reason) = pre_block_reason {
            tracing::warn!(
                session_id = %context.session_id,
                tool_call_id = %execution_tool_call.id,
                tool_name = %execution_tool_call.name,
                reason = %reason,
                "ActAtom: pre_tool_use hook blocked execution"
            );
            Ok(crate::tool_types::ToolResult {
                tool_call_id: execution_tool_call.id.clone(),
                result: None,
                images: None,
                error: Some(format!("blocked by pre_tool_use hook: {reason}")),
                connection_required: None,
                raw_output: None,
            })
        } else if tool_def.is_cpu_bound() {
            // CPU-bound / non-yielding in-process tools (e.g. the bash
            // interpreter) get their own task so a long synchronous burst
            // cannot starve the cooperative polling of I/O-bound tools running
            // alongside them in this act batch. On the multi-thread runtime the
            // spawned task can also progress on another worker thread.
            let executor = self.tool_executor.clone();
            let call = execution_tool_call.clone();
            let def = tool_def.clone();
            let ctx = tool_context.clone();
            match AbortOnDropJoinHandle::new(tokio::spawn(async move {
                executor.execute_with_context(&call, &def, &ctx).await
            }))
            .await
            {
                Ok(result) => result,
                Err(join_err) => Err(crate::error::AgentLoopError::tool(format!(
                    "tool task failed to complete: {join_err}"
                ))),
            }
        } else {
            self.tool_executor
                .execute_with_context(&execution_tool_call, tool_def, &tool_context)
                .await
        };

        match result {
            Ok(mut tool_result) => {
                // Run post-tool-exec hooks (capability then final/infrastructure)
                act_hooks::run_post_tool_exec_hooks(
                    &self.post_tool_hooks,
                    &self.final_post_tool_hooks,
                    &execution_tool_call,
                    tool_def,
                    &mut tool_result,
                    &tool_context,
                )
                .await;

                let tool_duration_ms = tool_start.elapsed().as_millis() as u64;
                let success = tool_result.error.is_none();
                let status = if success { "success" } else { "error" };

                // Emit tool.completed event
                let completed_data = if success {
                    let result_fingerprint = tool_result_fingerprint(&tool_call.name, &tool_result);
                    // Convert result to ContentPart (text + optional images)
                    let mut result_content = tool_result
                        .result
                        .as_ref()
                        .map(|r| vec![ContentPart::text(r.to_string())])
                        .unwrap_or_default();
                    // Append images as native Image content parts
                    if let Some(ref images) = tool_result.images {
                        for img in images {
                            result_content.push(ContentPart::Image(
                                crate::message::ImageContentPart::from_base64(
                                    &img.base64,
                                    &img.media_type,
                                ),
                            ));
                        }
                    }
                    ToolCompletedData::success(
                        tool_call.id.clone(),
                        tool_call.name.clone(),
                        result_content,
                        Some(tool_duration_ms),
                    )
                    .with_fingerprints(tool_call_fingerprint.clone(), result_fingerprint)
                    .with_display_name(display_name.clone())
                    .with_capability_attribution(
                        capability_attribution.as_ref().map(|(id, _)| id.clone()),
                        capability_attribution
                            .as_ref()
                            .and_then(|(_, name)| name.clone()),
                    )
                    .with_narration(Some(self.render_tool_narration(
                        Some(tool_def),
                        &tool_call,
                        ToolNarrationPhase::Completed,
                        locale,
                    )))
                } else {
                    let result_fingerprint = tool_result_fingerprint(&tool_call.name, &tool_result);
                    ToolCompletedData::failure(
                        tool_call.id.clone(),
                        tool_call.name.clone(),
                        status.to_string(),
                        tool_result.error.clone().unwrap_or_default(),
                        Some(tool_duration_ms),
                    )
                    .with_fingerprints(tool_call_fingerprint.clone(), result_fingerprint)
                    .with_display_name(display_name.clone())
                    .with_capability_attribution(
                        capability_attribution.as_ref().map(|(id, _)| id.clone()),
                        capability_attribution
                            .as_ref()
                            .and_then(|(_, name)| name.clone()),
                    )
                    .with_narration(Some(self.render_tool_narration(
                        Some(tool_def),
                        &tool_call,
                        ToolNarrationPhase::Failed,
                        locale,
                    )))
                };

                if let Err(e) = self
                    .event_emitter
                    .emit(EventRequest::new(
                        context.session_id,
                        event_context.clone(),
                        completed_data,
                    ))
                    .await
                {
                    tracing::warn!(
                        session_id = %context.session_id,
                        tool_call_id = %tool_call.id,
                        error = %e,
                        "ActAtom: failed to emit tool.completed event"
                    );
                }

                tracing::debug!(
                    session_id = %context.session_id,
                    tool_name = %tool_call.name,
                    tool_call_id = %tool_call.id,
                    success = %success,
                    "ActAtom: tool execution completed"
                );

                let conn_req = tool_result.connection_required.clone();
                ToolCallResult {
                    tool_call,
                    result: tool_result,
                    success,
                    status: status.to_string(),
                    connection_required: conn_req,
                }
            }
            Err(e) => {
                let tool_duration_ms = tool_start.elapsed().as_millis() as u64;
                let error_msg = e.to_string();

                // Emit tool.completed event for error
                if let Err(emit_err) = self
                    .event_emitter
                    .emit(EventRequest::new(
                        context.session_id,
                        event_context,
                        ToolCompletedData::failure(
                            tool_call.id.clone(),
                            tool_call.name.clone(),
                            "error".to_string(),
                            error_msg.clone(),
                            Some(tool_duration_ms),
                        )
                        .with_fingerprints(
                            tool_call_fingerprint.clone(),
                            tool_error_fingerprint(&tool_call.name, "error", &error_msg),
                        )
                        .with_display_name(display_name.clone())
                        .with_capability_attribution(
                            capability_attribution.as_ref().map(|(id, _)| id.clone()),
                            capability_attribution
                                .as_ref()
                                .and_then(|(_, name)| name.clone()),
                        )
                        .with_narration(Some(self.render_tool_narration(
                            Some(tool_def),
                            &tool_call,
                            ToolNarrationPhase::Failed,
                            locale,
                        ))),
                    ))
                    .await
                {
                    tracing::warn!(
                        session_id = %context.session_id,
                        tool_call_id = %tool_call.id,
                        error = %emit_err,
                        "ActAtom: failed to emit tool.completed event"
                    );
                }

                tracing::warn!(
                    session_id = %context.session_id,
                    tool_name = %tool_call.name,
                    tool_call_id = %tool_call.id,
                    error = %e,
                    "ActAtom: tool execution failed"
                );

                ToolCallResult {
                    tool_call: tool_call.clone(),
                    result: ToolResult {
                        tool_call_id: tool_call.id.clone(),
                        result: None,
                        images: None,
                        error: Some(error_msg),
                        connection_required: None,
                        raw_output: None,
                    },
                    success: false,
                    status: "error".to_string(),
                    connection_required: None,
                }
            }
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::ToolRegistry;
    use crate::traits::NoopEventEmitter;
    use crate::typed_id::{AgentId, HarnessId, MessageId, SessionId, TurnId};
    use async_trait::async_trait;
    use serde_json::json;

    struct ArgumentEchoTool;

    #[async_trait]
    impl crate::tools::Tool for ArgumentEchoTool {
        fn name(&self) -> &str {
            "argument_echo"
        }

        fn description(&self) -> &str {
            "returns the execution arguments"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            json!({
                "type": "object",
                "properties": {
                    "value": { "type": "string" }
                }
            })
        }

        async fn execute(&self, arguments: serde_json::Value) -> crate::ToolExecutionResult {
            crate::ToolExecutionResult::success(arguments)
        }
    }

    struct UtilityLlmContextProbeTool;

    #[async_trait]
    impl crate::tools::Tool for UtilityLlmContextProbeTool {
        fn name(&self) -> &str {
            "utility_llm_context_probe"
        }

        fn description(&self) -> &str {
            "checks whether the utility LLM service is present in tool context"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            json!({
                "type": "object",
                "properties": {}
            })
        }

        async fn execute(&self, _arguments: serde_json::Value) -> crate::ToolExecutionResult {
            crate::ToolExecutionResult::tool_error("context required")
        }

        async fn execute_with_context(
            &self,
            _arguments: serde_json::Value,
            context: &crate::traits::ToolContext,
        ) -> crate::ToolExecutionResult {
            crate::ToolExecutionResult::success(json!({
                "utility_llm_service": context.utility_llm_service.is_some(),
                "configured": context
                    .utility_llm_service
                    .as_ref()
                    .is_some_and(|service| service.is_configured()),
            }))
        }

        fn requires_context(&self) -> bool {
            true
        }
    }

    /// Shared scheduling observations recorded by `RecordingTool`.
    #[derive(Default)]
    struct SchedObservations {
        /// Currently-executing count per concurrency class.
        class_inflight: std::collections::HashMap<String, usize>,
        /// Peak concurrent executions observed per class.
        class_max: std::collections::HashMap<String, usize>,
        /// Currently-executing count across all tools.
        global_inflight: usize,
        /// Peak concurrent executions across all tools.
        global_max: usize,
    }

    /// Tool that records start/end so a test can observe how the act scheduler
    /// ran a batch (intra-class serialization, cross-class parallelism).
    struct RecordingTool {
        name: String,
        class: Option<String>,
        obs: Arc<std::sync::Mutex<SchedObservations>>,
    }

    #[async_trait]
    impl crate::tools::Tool for RecordingTool {
        fn name(&self) -> &str {
            &self.name
        }
        fn description(&self) -> &str {
            "records scheduling order"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            json!({ "type": "object", "properties": {} })
        }
        async fn execute(&self, _arguments: serde_json::Value) -> crate::ToolExecutionResult {
            // Enter: bump counters in a short critical section (no await held).
            {
                let mut obs = self.obs.lock().unwrap();
                obs.global_inflight += 1;
                let g = obs.global_inflight;
                if g > obs.global_max {
                    obs.global_max = g;
                }
                if let Some(class) = &self.class {
                    let n = obs.class_inflight.entry(class.clone()).or_default();
                    *n += 1;
                    let cur = *n;
                    let m = obs.class_max.entry(class.clone()).or_default();
                    if cur > *m {
                        *m = cur;
                    }
                }
            }
            // Hold the slot long enough that any concurrency is observable.
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            // Exit.
            {
                let mut obs = self.obs.lock().unwrap();
                obs.global_inflight -= 1;
                if let Some(class) = &self.class
                    && let Some(n) = obs.class_inflight.get_mut(class)
                {
                    *n -= 1;
                }
            }
            crate::ToolExecutionResult::success(json!({ "tool": self.name }))
        }
    }

    struct CancellationProbeTool {
        started: Arc<tokio::sync::Notify>,
        dropped_tx: Arc<std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
    }

    impl CancellationProbeTool {
        fn new(
            started: Arc<tokio::sync::Notify>,
            dropped_tx: tokio::sync::oneshot::Sender<()>,
        ) -> Self {
            Self {
                started,
                dropped_tx: Arc::new(std::sync::Mutex::new(Some(dropped_tx))),
            }
        }
    }

    #[async_trait]
    impl crate::tools::Tool for CancellationProbeTool {
        fn name(&self) -> &str {
            "cancellation_probe"
        }

        fn description(&self) -> &str {
            "waits until cancelled"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            json!({ "type": "object", "properties": {} })
        }

        async fn execute(&self, _arguments: serde_json::Value) -> crate::ToolExecutionResult {
            struct DropSignal {
                tx: Arc<std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
            }

            impl Drop for DropSignal {
                fn drop(&mut self) {
                    if let Ok(mut guard) = self.tx.lock()
                        && let Some(tx) = guard.take()
                    {
                        let _ = tx.send(());
                    }
                }
            }

            let _drop_signal = DropSignal {
                tx: self.dropped_tx.clone(),
            };
            self.started.notify_one();
            std::future::pending::<()>().await;
            unreachable!("pending cancellation probe should only finish by cancellation")
        }
    }

    /// Build a server-side tool definition carrying scheduling hints.
    fn recording_tool_def(name: &str, class: Option<&str>, cpu_bound: bool) -> ToolDefinition {
        let mut hints = crate::tool_types::ToolHints::default();
        if let Some(class) = class {
            hints = hints.with_concurrency_class(class);
        }
        if cpu_bound {
            hints = hints.with_cpu_bound(true);
        }
        ToolDefinition::Builtin(crate::BuiltinTool {
            name: name.to_string(),
            display_name: None,
            description: "records scheduling order".to_string(),
            parameters: json!({ "type": "object", "properties": {} }),
            policy: Default::default(),
            category: None,
            deferrable: Default::default(),
            hints,
            full_parameters: None,
        })
    }

    #[tokio::test]
    async fn test_act_atom_empty_tool_calls() {
        let executor = ToolRegistry::with_defaults();
        let event_emitter = NoopEventEmitter;
        let atom = ActAtom::new(executor, event_emitter);

        let context = AtomContext::new(SessionId::new(), TurnId::new(), MessageId::new());
        let input = ActInput {
            org_id: Some(1),
            context,
            harness_id: HarnessId::from_seed(1),
            agent_id: Some(AgentId::new()),
            tool_calls: vec![],
            tool_definitions: vec![],
            locale: None,
            blueprint_id: None,
            network_access: None,
            parallel_tool_calls: None,
        };

        let result = atom.execute(input).await.unwrap();

        assert!(result.completed);
        assert!(result.results.is_empty());
        assert_eq!(result.success_count, 0);
        assert_eq!(result.error_count, 0);
    }

    #[tokio::test]
    async fn test_act_atom_threads_utility_llm_service_to_tool_context() {
        let mut executor = ToolRegistry::with_defaults();
        executor.register(UtilityLlmContextProbeTool);
        let event_emitter = NoopEventEmitter;
        let atom = ActAtom::new(executor, event_emitter)
            .with_utility_llm_service(Arc::new(crate::DisabledUtilityLlmService));

        let context = AtomContext::new(SessionId::new(), TurnId::new(), MessageId::new());
        let input = ActInput {
            org_id: Some(1),
            context,
            harness_id: HarnessId::from_seed(1),
            agent_id: Some(AgentId::new()),
            tool_calls: vec![ToolCall {
                id: "call_1".to_string(),
                name: "utility_llm_context_probe".to_string(),
                arguments: json!({}),
            }],
            tool_definitions: vec![ToolDefinition::Builtin(crate::BuiltinTool {
                name: "utility_llm_context_probe".to_string(),
                display_name: None,
                description: "checks context".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {}
                }),
                policy: Default::default(),
                category: None,
                deferrable: Default::default(),
                hints: crate::tool_types::ToolHints::default(),
                full_parameters: None,
            })],
            locale: None,
            blueprint_id: None,
            network_access: None,
            parallel_tool_calls: None,
        };

        let result = atom.execute(input).await.unwrap();

        assert_eq!(result.success_count, 1);
        let payload = result.results[0].result.result.as_ref().unwrap();
        assert_eq!(payload["utility_llm_service"], true);
        assert_eq!(payload["configured"], false);
    }

    /// End-to-end ActAtom scheduling: a single batch with two same-class tools
    /// (one of them `cpu_bound`, exercising the spawn path) plus an independent
    /// tool. Asserts the scheduler serializes within the class, parallelizes
    /// across classes, runs every tool, and preserves call order in results.
    #[tokio::test]
    async fn test_act_atom_schedules_batch_by_concurrency_class() {
        let obs = Arc::new(std::sync::Mutex::new(SchedObservations::default()));

        let mut executor = ToolRegistry::new();
        executor.register(RecordingTool {
            name: "writer_a".to_string(),
            class: Some("ws".to_string()),
            obs: obs.clone(),
        });
        executor.register(RecordingTool {
            name: "writer_b".to_string(),
            class: Some("ws".to_string()),
            obs: obs.clone(),
        });
        executor.register(RecordingTool {
            name: "reader".to_string(),
            class: None,
            obs: obs.clone(),
        });

        let atom = ActAtom::new(executor, NoopEventEmitter);
        let context = AtomContext::new(SessionId::new(), TurnId::new(), MessageId::new());

        // Call order: writer_a, reader, writer_b. writer_a and writer_b share
        // class "ws" (writer_b is cpu_bound → executed on its own task).
        let input = ActInput {
            org_id: Some(1),
            context,
            harness_id: HarnessId::from_seed(1),
            agent_id: Some(AgentId::new()),
            tool_calls: vec![
                ToolCall {
                    id: "call_a".to_string(),
                    name: "writer_a".to_string(),
                    arguments: json!({}),
                },
                ToolCall {
                    id: "call_r".to_string(),
                    name: "reader".to_string(),
                    arguments: json!({}),
                },
                ToolCall {
                    id: "call_b".to_string(),
                    name: "writer_b".to_string(),
                    arguments: json!({}),
                },
            ],
            tool_definitions: vec![
                recording_tool_def("writer_a", Some("ws"), false),
                recording_tool_def("reader", None, false),
                recording_tool_def("writer_b", Some("ws"), true),
            ],
            locale: None,
            blueprint_id: None,
            network_access: None,
            parallel_tool_calls: None,
        };

        let result = atom.execute(input).await.unwrap();

        // Every tool ran and succeeded.
        assert_eq!(result.success_count, 3, "all three tools should succeed");
        // Results are returned in the model's original call order.
        let names: Vec<&str> = result
            .results
            .iter()
            .map(|r| r.tool_call.name.as_str())
            .collect();
        assert_eq!(names, vec!["writer_a", "reader", "writer_b"]);

        let obs = obs.lock().unwrap();
        // Same-class tools never overlapped (serialized) — even though one is
        // cpu_bound and runs on its own task.
        assert_eq!(
            obs.class_max.get("ws").copied(),
            Some(1),
            "same-class tools must serialize"
        );
        // The independent tool overlapped with the class group: peak global
        // concurrency exceeded 1, proving cross-class parallelism.
        assert!(
            obs.global_max >= 2,
            "independent tool should run concurrently with the class group (global_max={})",
            obs.global_max
        );
    }

    #[tokio::test]
    async fn test_act_atom_aborts_cpu_bound_tool_task_on_cancellation() {
        let started = Arc::new(tokio::sync::Notify::new());
        let (dropped_tx, dropped_rx) = tokio::sync::oneshot::channel();

        let mut executor = ToolRegistry::new();
        executor.register(CancellationProbeTool::new(started.clone(), dropped_tx));

        let atom = ActAtom::new(executor, NoopEventEmitter);
        let context = AtomContext::new(SessionId::new(), TurnId::new(), MessageId::new());
        let input = ActInput {
            org_id: Some(1),
            context,
            harness_id: HarnessId::from_seed(1),
            agent_id: Some(AgentId::new()),
            tool_calls: vec![ToolCall {
                id: "call_1".to_string(),
                name: "cancellation_probe".to_string(),
                arguments: json!({}),
            }],
            tool_definitions: vec![recording_tool_def("cancellation_probe", None, true)],
            locale: None,
            blueprint_id: None,
            network_access: None,
            parallel_tool_calls: None,
        };

        let act_task = tokio::spawn(async move { atom.execute(input).await });
        started.notified().await;
        act_task.abort();
        assert!(act_task.await.unwrap_err().is_cancelled());

        tokio::time::timeout(std::time::Duration::from_secs(1), dropped_rx)
            .await
            .expect("cpu-bound tool task should be aborted when ActAtom is cancelled")
            .expect("drop signal should be sent by cancelled tool future");
    }

    /// With `parallel_tool_calls = Some(false)`, the whole batch runs strictly
    /// sequentially regardless of class — peak concurrency must be 1.
    #[tokio::test]
    async fn test_act_atom_parallel_tool_calls_false_serializes_everything() {
        let obs = Arc::new(std::sync::Mutex::new(SchedObservations::default()));
        let mut executor = ToolRegistry::new();
        for name in ["t0", "t1", "t2"] {
            executor.register(RecordingTool {
                name: name.to_string(),
                class: None,
                obs: obs.clone(),
            });
        }
        let atom = ActAtom::new(executor, NoopEventEmitter);
        let context = AtomContext::new(SessionId::new(), TurnId::new(), MessageId::new());
        let input = ActInput {
            org_id: Some(1),
            context,
            harness_id: HarnessId::from_seed(1),
            agent_id: Some(AgentId::new()),
            tool_calls: vec![
                ToolCall {
                    id: "c0".to_string(),
                    name: "t0".to_string(),
                    arguments: json!({}),
                },
                ToolCall {
                    id: "c1".to_string(),
                    name: "t1".to_string(),
                    arguments: json!({}),
                },
                ToolCall {
                    id: "c2".to_string(),
                    name: "t2".to_string(),
                    arguments: json!({}),
                },
            ],
            tool_definitions: vec![
                recording_tool_def("t0", None, false),
                recording_tool_def("t1", None, false),
                recording_tool_def("t2", None, false),
            ],
            locale: None,
            blueprint_id: None,
            network_access: None,
            parallel_tool_calls: Some(false),
        };

        let result = atom.execute(input).await.unwrap();
        assert_eq!(result.success_count, 3);
        assert_eq!(
            obs.lock().unwrap().global_max,
            1,
            "parallel_tool_calls=false must serialize the whole batch"
        );
    }

    #[tokio::test]
    async fn test_act_atom_tool_not_found() {
        let executor = ToolRegistry::with_defaults();
        let event_emitter = NoopEventEmitter;
        let atom = ActAtom::new(executor, event_emitter);

        let context = AtomContext::new(SessionId::new(), TurnId::new(), MessageId::new());
        let input = ActInput {
            org_id: Some(1),
            context,
            harness_id: HarnessId::from_seed(1),
            agent_id: Some(AgentId::new()),
            tool_calls: vec![ToolCall {
                id: "call_1".to_string(),
                name: "nonexistent_tool".to_string(),
                arguments: json!({}),
            }],
            tool_definitions: vec![],
            locale: None,
            blueprint_id: None,
            network_access: None,
            parallel_tool_calls: None,
        };

        let result = atom.execute(input).await.unwrap();

        assert!(result.completed);
        assert_eq!(result.results.len(), 1);
        assert!(!result.results[0].success);
        assert_eq!(result.results[0].status, "error");
        assert!(
            result.results[0]
                .result
                .error
                .as_ref()
                .unwrap()
                .contains("not found")
        );
    }

    #[tokio::test]
    async fn test_act_atom_uses_tool_call_hooks_for_execution_arguments() {
        use crate::capabilities::{Capability, HumanIntentCapability};

        let mut executor = ToolRegistry::new();
        executor.register(ArgumentEchoTool);
        let tool_def = executor.get("argument_echo").unwrap().to_definition();
        let emitter = crate::memory::InMemoryEventEmitter::new();
        let atom = ActAtom::new(executor, emitter.clone())
            .with_tool_call_hooks(HumanIntentCapability.tool_call_hooks());

        let context = AtomContext::new(SessionId::new(), TurnId::new(), MessageId::new());
        let input = ActInput {
            org_id: Some(1),
            context,
            harness_id: HarnessId::from_seed(1),
            agent_id: Some(AgentId::new()),
            tool_calls: vec![ToolCall {
                id: "call_1".to_string(),
                name: "argument_echo".to_string(),
                arguments: json!({
                    "value": "visible",
                    "human_intent": "Echoing test arguments"
                }),
            }],
            tool_definitions: vec![tool_def],
            locale: None,
            blueprint_id: None,
            network_access: None,
            parallel_tool_calls: None,
        };

        let result = atom.execute(input).await.unwrap();

        assert!(result.results[0].success);
        assert_eq!(
            result.results[0].result.result,
            Some(json!({ "value": "visible" }))
        );

        let events = emitter.events().await;
        let act_started = events
            .iter()
            .find(|event| event.event_type == "act.started")
            .expect("act.started event");
        let crate::events::EventData::ActStarted(data) = &act_started.data else {
            panic!("expected act.started data");
        };
        assert_eq!(data.headline.as_deref(), Some("Echoing test arguments"));
        assert_eq!(
            data.tool_calls[0].narration.as_deref(),
            Some("Echoing test arguments")
        );

        let tool_started = events
            .iter()
            .find(|event| event.event_type == "tool.started")
            .expect("tool.started event");
        let crate::events::EventData::ToolStarted(data) = &tool_started.data else {
            panic!("expected tool.started data");
        };
        let started_fingerprint = data
            .tool_call_fingerprint
            .as_ref()
            .expect("tool.started call fingerprint");
        assert_eq!(data.narration.as_deref(), Some("Echoing test arguments"));

        let tool_completed = events
            .iter()
            .find(|event| event.event_type == "tool.completed")
            .expect("tool.completed event");
        let crate::events::EventData::ToolCompleted(data) = &tool_completed.data else {
            panic!("expected tool.completed data");
        };
        assert_eq!(
            data.tool_call_fingerprint.as_ref(),
            Some(started_fingerprint)
        );
        assert!(data.tool_result_fingerprint.is_some());
        assert_eq!(data.narration.as_deref(), Some("Echoing test arguments"));
    }

    #[tokio::test]
    async fn test_act_atom_strips_human_intent_from_client_tool_calls() {
        use crate::capabilities::{Capability, HumanIntentCapability};

        let executor = ToolRegistry::new();
        let emitter = crate::memory::InMemoryEventEmitter::new();
        let atom = ActAtom::new(executor, emitter)
            .with_tool_call_hooks(HumanIntentCapability.tool_call_hooks());

        let context = AtomContext::new(SessionId::new(), TurnId::new(), MessageId::new());
        let input = ActInput {
            org_id: Some(1),
            context,
            harness_id: HarnessId::from_seed(1),
            agent_id: Some(AgentId::new()),
            tool_calls: vec![ToolCall {
                id: "call_client".to_string(),
                name: "browser_click".to_string(),
                arguments: json!({
                    "selector": "#btn",
                    "human_intent": "Clicking approve"
                }),
            }],
            tool_definitions: vec![crate::ToolDefinition::ClientSide(crate::ClientSideTool {
                name: "browser_click".to_string(),
                display_name: None,
                description: "Click button".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "selector": {"type": "string"}
                    },
                    "required": ["selector"]
                }),
                category: None,
                deferrable: Default::default(),
                hints: crate::tool_types::ToolHints::default(),
                full_parameters: None,
            })],
            locale: None,
            blueprint_id: None,
            network_access: None,
            parallel_tool_calls: None,
        };

        let result = atom.execute(input).await.unwrap();

        assert_eq!(result.client_tool_calls.len(), 1);
        assert_eq!(
            result.client_tool_calls[0].arguments,
            json!({ "selector": "#btn" })
        );
    }

    fn manage_harnesses_tool_def() -> crate::ToolDefinition {
        crate::ToolDefinition::Builtin(crate::BuiltinTool {
            name: "manage_harnesses".to_string(),
            display_name: None,
            description: "CRUD operations for harnesses".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "operation": {"type": "string"}
                },
                "required": ["operation"]
            }),
            policy: Default::default(),
            category: None,
            deferrable: Default::default(),
            hints: crate::tool_types::ToolHints::default(),
            full_parameters: None,
        })
    }

    fn read_capabilities_tool_def() -> crate::ToolDefinition {
        crate::ToolDefinition::Builtin(crate::BuiltinTool {
            name: "read_capabilities".to_string(),
            display_name: None,
            description: "List capabilities".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "id": {"type": "string"},
                    "search": {"type": "string"}
                }
            }),
            policy: Default::default(),
            category: None,
            deferrable: Default::default(),
            hints: crate::tool_types::ToolHints::default(),
            full_parameters: None,
        })
    }

    #[tokio::test]
    async fn test_act_atom_platform_tool_works_with_platform_store() {
        use crate::capabilities::{Capability, PlatformManagementCapability};

        let mut executor = ToolRegistry::with_defaults();
        for tool in PlatformManagementCapability.tools() {
            executor.register_boxed(tool);
        }
        let event_emitter = NoopEventEmitter;

        // Build mock platform store
        let mock_store = crate::platform_store::tests::MockPlatformStore::new();
        let platform_store: Arc<dyn crate::platform_store::PlatformStore> = Arc::new(mock_store);

        let atom = ActAtom::new(executor, event_emitter).with_platform_store(platform_store);

        let context = AtomContext::new(SessionId::new(), TurnId::new(), MessageId::new());
        let input = ActInput {
            org_id: Some(1),
            context,
            harness_id: HarnessId::from_seed(1),
            agent_id: None,
            tool_calls: vec![ToolCall {
                id: "call_1".to_string(),
                name: "read_capabilities".to_string(),
                arguments: json!({}),
            }],
            tool_definitions: vec![read_capabilities_tool_def()],
            locale: None,
            blueprint_id: None,
            network_access: None,
            parallel_tool_calls: None,
        };

        let result = atom.execute(input).await.unwrap();

        assert!(result.completed);
        assert_eq!(result.results.len(), 1);
        assert!(
            result.results[0].success,
            "read_capabilities should succeed when platform_store is wired: {:?}",
            result.results[0].result.error
        );
    }

    /// Regression test: `manage_harnesses` called through ActAtom WITHOUT
    /// `platform_store` should produce an error message in the result body.
    /// ToolErrors set error field so they are logged as failures in events.
    #[tokio::test]
    async fn test_act_atom_platform_tool_fails_without_platform_store() {
        use crate::capabilities::{Capability, PlatformManagementCapability};

        let mut executor = ToolRegistry::with_defaults();
        for tool in PlatformManagementCapability.tools() {
            executor.register_boxed(tool);
        }
        let event_emitter = NoopEventEmitter;

        // No platform_store set → tool returns ToolError about platform management
        // not being available (execute_with_context is always used now).
        let atom = ActAtom::new(executor, event_emitter);

        let context = AtomContext::new(SessionId::new(), TurnId::new(), MessageId::new());
        let input = ActInput {
            org_id: Some(1),
            context,
            harness_id: HarnessId::from_seed(1),
            agent_id: None,
            tool_calls: vec![ToolCall {
                id: "call_1".to_string(),
                name: "manage_harnesses".to_string(),
                arguments: json!({"operation": "list"}),
            }],
            tool_definitions: vec![manage_harnesses_tool_def()],
            locale: None,
            blueprint_id: None,
            network_access: None,
            parallel_tool_calls: None,
        };

        let result = atom.execute(input).await.unwrap();

        assert!(result.completed);
        assert_eq!(result.results.len(), 1);
        // ToolErrors set error field and are logged as failures
        assert!(!result.results[0].success);
        let err_msg = result.results[0].result.error.as_deref().unwrap();
        assert!(
            err_msg.contains("Platform management not available"),
            "Expected platform management error, got: {err_msg}"
        );
    }

    #[test]
    fn test_act_result_connection_required_serialization() {
        let result = ActResult {
            results: vec![ToolCallResult {
                tool_call: ToolCall {
                    id: "call_1".to_string(),
                    name: "daytona_create_sandbox".to_string(),
                    arguments: json!({}),
                },
                result: ToolResult {
                    tool_call_id: "call_1".to_string(),
                    result: Some(json!({"connection_required": "daytona"})),
                    images: None,
                    error: None,
                    connection_required: Some("daytona".to_string()),
                    raw_output: None,
                },
                success: false,
                status: "success".to_string(),
                connection_required: Some("daytona".to_string()),
            }],
            completed: true,
            success_count: 0,
            error_count: 0,
            waiting_for_tool_results: true,
            blocked: false,
            client_tool_calls: vec![],
            client_tool_definitions: vec![],
        };

        let json_str = serde_json::to_string(&result).unwrap();
        let parsed: ActResult = serde_json::from_str(&json_str).unwrap();

        assert!(parsed.waiting_for_tool_results);
        assert_eq!(
            parsed.results[0].connection_required,
            Some("daytona".to_string())
        );
    }

    #[test]
    fn test_act_result_backward_compat_deserialization() {
        // Old JSON without new fields still deserializes
        let json_str = r#"{
            "results": [],
            "completed": true,
            "success_count": 0,
            "error_count": 0
        }"#;
        let parsed: ActResult = serde_json::from_str(json_str).unwrap();

        assert!(!parsed.waiting_for_tool_results);
        assert!(parsed.client_tool_calls.is_empty());
    }

    /// Verify that a denying `OutboundToolRateLimiter` short-circuits tool execution
    /// and returns a rate-limit error result rather than calling the actual tool.
    #[tokio::test]
    async fn test_outbound_tool_rate_limiter_blocks_execution() {
        use crate::typed_id::OrgId;

        struct DenyAll;
        #[async_trait]
        impl crate::traits::OutboundToolRateLimiter for DenyAll {
            async fn check_org(&self, _org_id: &OrgId) -> bool {
                false
            }
        }

        let mut executor = ToolRegistry::with_defaults();
        executor.register(ArgumentEchoTool);
        let atom = ActAtom::new(executor, NoopEventEmitter)
            .with_org_id(OrgId::from_seed(1))
            .with_outbound_tool_rate_limiter(Arc::new(DenyAll));

        let context = AtomContext::new(SessionId::new(), TurnId::new(), MessageId::new());
        let input = ActInput {
            org_id: Some(1),
            context,
            harness_id: HarnessId::from_seed(1),
            agent_id: Some(AgentId::new()),
            tool_calls: vec![ToolCall {
                id: "call_1".to_string(),
                name: "argument_echo".to_string(),
                arguments: json!({"value": "should_not_reach"}),
            }],
            tool_definitions: vec![ToolDefinition::Builtin(crate::BuiltinTool {
                name: "argument_echo".to_string(),
                display_name: None,
                description: "echo".to_string(),
                parameters: json!({"type": "object"}),
                policy: Default::default(),
                category: None,
                deferrable: Default::default(),
                hints: crate::tool_types::ToolHints::default(),
                full_parameters: None,
            })],
            locale: None,
            blueprint_id: None,
            network_access: None,
            parallel_tool_calls: None,
        };

        let result = atom.execute(input).await.unwrap();

        assert_eq!(result.success_count, 0);
        assert_eq!(result.error_count, 1);
        let tool_result = &result.results[0];
        assert!(!tool_result.success);
        assert_eq!(tool_result.status, "error");
        assert!(
            tool_result
                .result
                .error
                .as_deref()
                .unwrap_or("")
                .contains("rate limit exceeded")
        );
        assert!(tool_result.result.result.is_none());
    }

    /// Verify that an allowing `OutboundToolRateLimiter` does not block execution.
    #[tokio::test]
    async fn test_outbound_tool_rate_limiter_allows_execution() {
        use crate::typed_id::OrgId;

        struct AllowAll;
        #[async_trait]
        impl crate::traits::OutboundToolRateLimiter for AllowAll {
            async fn check_org(&self, _org_id: &OrgId) -> bool {
                true
            }
        }

        let mut executor = ToolRegistry::with_defaults();
        executor.register(ArgumentEchoTool);
        let atom = ActAtom::new(executor, NoopEventEmitter)
            .with_org_id(OrgId::from_seed(1))
            .with_outbound_tool_rate_limiter(Arc::new(AllowAll));

        let context = AtomContext::new(SessionId::new(), TurnId::new(), MessageId::new());
        let input = ActInput {
            org_id: Some(1),
            context,
            harness_id: HarnessId::from_seed(1),
            agent_id: Some(AgentId::new()),
            tool_calls: vec![ToolCall {
                id: "call_1".to_string(),
                name: "argument_echo".to_string(),
                arguments: json!({"value": "hello"}),
            }],
            tool_definitions: vec![ToolDefinition::Builtin(crate::BuiltinTool {
                name: "argument_echo".to_string(),
                display_name: None,
                description: "echo".to_string(),
                parameters: json!({"type": "object"}),
                policy: Default::default(),
                category: None,
                deferrable: Default::default(),
                hints: crate::tool_types::ToolHints::default(),
                full_parameters: None,
            })],
            locale: None,
            blueprint_id: None,
            network_access: None,
            parallel_tool_calls: None,
        };

        let result = atom.execute(input).await.unwrap();

        assert_eq!(result.success_count, 1);
        assert_eq!(result.error_count, 0);
    }
}

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
    GroupHeadlineAction, ToolNarrationContext, ToolNarrationPhase,
    render_tool_narration_with_locale, summarize_group_actions, tool_call_for_group_summary,
};
use crate::tool_types::{SideEffectClass, ToolCall, ToolDefinition, ToolResult};
use crate::typed_id::{AgentId, HarnessId};
use crate::{
    durability::DurableToolResultStore, durability::ToolCallClaimResult,
    event_emitter::EventEmitter, execution_loading::AgentStore, execution_loading::SessionStore,
    session_files::SessionFileSystem, tool_context::ToolContext, tool_execution::ToolExecutor,
};
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
    /// Determinism violation message. When Some, ActAtom::execute returns Err to fail the
    /// durable workflow fast rather than continuing with a corrupted replay.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub determinism_fatal: Option<String>,
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
    /// Runtime-owned service snapshot cloned into every per-call ToolContext.
    context_services: crate::tool_context::ToolContextServices,
    /// Optional per-org outbound tool-call rate limiter (TM-TOOL-009).
    /// When present, each tool call increments the org counter; calls that
    /// exceed the per-org window return a tool error rather than a hard failure.
    outbound_tool_rate_limiter: Option<Arc<dyn crate::tool_execution::OutboundToolRateLimiter>>,
    /// Per-tool-call idempotency store (EVE-530). When present, each tool call
    /// is claimed before dispatch and settled after completion so that reclaiming
    /// workers can skip already-settled calls and avoid double side-effects for
    /// `AtMostOnce` tools.
    durable_tool_result_store: Option<Arc<dyn DurableToolResultStore>>,
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
            context_services: crate::tool_context::ToolContextServices::default(),
            outbound_tool_rate_limiter: None,
            durable_tool_result_store: None,
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
            context_services: crate::tool_context::ToolContextServices {
                file_store: Some(file_store),
                ..Default::default()
            },
            outbound_tool_rate_limiter: None,
            durable_tool_result_store: None,
            hooks: Self::default_hooks(),
            post_tool_hooks: Vec::new(),
            pre_tool_hooks: Vec::new(),
            tool_call_hooks: Vec::new(),
            final_post_tool_hooks: Self::default_final_hooks(),
        }
    }

    /// Replace the complete runtime-owned service snapshot used for every
    /// per-call [`ToolContext`]. Production hosts should prefer this over
    /// assembling individual services on the atom.
    pub fn with_context_services(
        mut self,
        services: crate::tool_context::ToolContextServices,
    ) -> Self {
        self.context_services = services;
        self
    }

    /// Add a custom post-act hook.
    pub fn with_hook(mut self, hook: Box<dyn PostActHook>) -> Self {
        self.hooks.push(hook);
        self
    }

    /// Add a runtime-owned final post-tool hook. Hosts use this for portable
    /// policies that must run after capability hooks but before the hard output
    /// limit.
    pub fn with_final_post_tool_hook(mut self, hook: Arc<dyn act_hooks::PostToolExecHook>) -> Self {
        let hard_limit_index = self.final_post_tool_hooks.len().saturating_sub(1);
        self.final_post_tool_hooks.insert(hard_limit_index, hook);
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
        vec![Arc::new(act_hooks::OutputHardLimitHook)]
    }

    /// Set the session storage store on this atom
    pub fn with_storage_store(
        mut self,
        store: Arc<dyn crate::session_services::SessionStorageStore>,
    ) -> Self {
        self.context_services.storage_store = Some(store);
        self
    }

    /// Set the image artifact store on this atom
    pub fn with_image_store(
        mut self,
        store: Arc<dyn crate::image_services::ImageArtifactStore>,
    ) -> Self {
        self.context_services.image_store = Some(store);
        self
    }

    /// Set the provider credential store on this atom
    pub fn with_provider_credential_store(
        mut self,
        store: Arc<dyn crate::connection_services::ProviderCredentialStore>,
    ) -> Self {
        self.context_services.provider_credential_store = Some(store);
        self
    }

    /// Set the utility LLM service on this atom.
    pub fn with_utility_llm_service(mut self, service: Arc<dyn crate::UtilityLlmService>) -> Self {
        self.context_services.utility_llm_service = Some(service);
        self
    }

    /// Set the scoped-MCP tool invoker on this atom (guardrails `mcp` check).
    pub fn with_mcp_invoker(mut self, invoker: Arc<dyn crate::McpToolInvoker>) -> Self {
        self.context_services.mcp_invoker = Some(invoker);
        self
    }

    /// Set the outbound egress service on this atom.
    pub fn with_egress_service(mut self, service: Arc<dyn crate::EgressService>) -> Self {
        self.context_services.egress_service = Some(service);
        self
    }

    /// Set the user connection resolver on this atom
    pub fn with_connection_resolver(
        mut self,
        resolver: Arc<dyn crate::connection_services::UserConnectionResolver>,
    ) -> Self {
        self.context_services.connection_resolver = Some(resolver);
        self
    }

    /// Set session store for context-aware tools.
    pub fn with_session_store(mut self, store: Arc<dyn SessionStore>) -> Self {
        self.context_services.session_store = Some(store);
        self
    }

    /// Set agent store for context-aware tools.
    pub fn with_agent_store(mut self, store: Arc<dyn AgentStore>) -> Self {
        self.context_services.agent_store = Some(store);
        self
    }

    /// Set session schedule store for scheduling tools.
    pub fn with_schedule_store(
        mut self,
        store: Arc<dyn crate::session_services::SessionScheduleStore>,
    ) -> Self {
        self.context_services.schedule_store = Some(store);
        self
    }

    /// Set platform store for org-level management tools.
    pub fn with_subagent_delegate(
        mut self,
        store: Arc<dyn crate::subagent_delegation::SubagentSessionDelegate>,
    ) -> Self {
        self.context_services.subagent_delegate = Some(store);
        self
    }

    /// Set leased resource store for lifecycle-managed provider resources.
    pub fn with_leased_resource_store(
        mut self,
        store: Arc<dyn crate::session_services::LeasedResourceStore>,
    ) -> Self {
        self.context_services.leased_resource_store = Some(store);
        self
    }

    /// Set session resource registry.
    pub fn with_session_resource_registry(
        mut self,
        registry: Arc<dyn crate::session_services::SessionResourceRegistry>,
    ) -> Self {
        self.context_services.session_resource_registry = Some(registry);
        self
    }

    /// Add a session task registry passed to tool contexts.
    pub fn with_session_task_registry(
        mut self,
        registry: Arc<dyn crate::session_task::SessionTaskRegistry>,
    ) -> Self {
        self.context_services.session_task_registry = Some(registry);
        self
    }

    pub fn with_capability_registry(
        mut self,
        registry: crate::capabilities::CapabilityRegistry,
    ) -> Self {
        self.context_services.capability_registry = Some(registry);
        self
    }

    /// Set the active built-in tool registry for meta-tools like `spawn_background`.
    pub fn with_tool_registry(mut self, registry: Arc<crate::tools::ToolRegistry>) -> Self {
        self.context_services.tool_registry = Some(registry);
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
    /// `act_hooks::PreToolUseHook` and `knowledge/runtime-resources/user-hooks.md`.
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

    /// Set org ID for org-scoped operations.
    pub fn with_org_id(mut self, org_id: crate::typed_id::OrgId) -> Self {
        self.context_services.org_id = Some(org_id);
        self
    }

    /// Set the merged network access list for URL filtering in tools.
    pub fn with_network_access(
        mut self,
        network_access: Option<crate::network_access::NetworkAccessList>,
    ) -> Self {
        self.context_services.network_access = network_access;
        self
    }

    /// Set the budget checker for the check_budget tool.
    pub fn with_budget_checker(
        mut self,
        checker: Arc<dyn crate::tool_execution::BudgetChecker>,
    ) -> Self {
        self.context_services.budget_checker = Some(checker);
        self
    }

    /// Set the internal payment authority for paid capability tools.
    pub fn with_payment_authority(
        mut self,
        authority: Arc<dyn crate::tool_execution::PaymentAuthority>,
    ) -> Self {
        self.context_services.payment_authority = Some(authority);
        self
    }

    /// Set the authority used to authorize detached peer-session creation.
    pub fn with_session_creation_authority(
        mut self,
        authority: Arc<dyn crate::delegation_services::SessionCreationAuthority>,
    ) -> Self {
        self.context_services.session_creation_authority = Some(authority);
        self
    }

    /// Set the per-org outbound tool-call rate limiter (TM-TOOL-009).
    pub fn with_outbound_tool_rate_limiter(
        mut self,
        limiter: Arc<dyn crate::tool_execution::OutboundToolRateLimiter>,
    ) -> Self {
        self.outbound_tool_rate_limiter = Some(limiter);
        self
    }

    /// Set the durable per-tool-call idempotency store (EVE-530).
    pub fn with_durable_tool_result_store(
        mut self,
        store: Arc<dyn DurableToolResultStore>,
    ) -> Self {
        self.durable_tool_result_store = Some(store);
        self
    }

    /// Set the durable subagent spawn handle store (EVE-535).
    pub fn with_subagent_spawn_store(
        mut self,
        store: Arc<dyn crate::delegation_services::SubagentSpawnStore>,
    ) -> Self {
        self.context_services.subagent_spawn_store = Some(store);
        self
    }

    /// Set the resolved subagent nesting policy for tool contexts.
    pub fn with_subagent_nesting_policy(
        mut self,
        policy: crate::delegation_services::SubagentNestingPolicy,
    ) -> Self {
        self.context_services.subagent_nesting_policy = policy;
        self
    }

    /// Set the live reasoning-effort handle (EVE-595). When set, each tool's
    /// `ToolContext` receives a clone so a tool can change the reasoning effort
    /// mid-turn for subsequent LLM steps in the same turn.
    pub fn with_reasoning_effort_handle(
        mut self,
        handle: crate::tool_context::ReasoningEffortHandle,
    ) -> Self {
        self.context_services.reasoning_effort_handle = Some(handle);
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
                    &context,
                    tool_def,
                    tool_call,
                    ToolNarrationPhase::Started,
                    locale.as_deref(),
                ));
                summary.completed_narration = Some(self.render_tool_narration(
                    &context,
                    tool_def,
                    tool_call,
                    ToolNarrationPhase::Completed,
                    locale.as_deref(),
                ));
            }
        }
        started_data.headline = self.render_group_headline(
            &context,
            &tool_calls,
            &tool_map,
            ToolNarrationPhase::Started,
            locale.as_deref(),
        );

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
        let mut completed_headline = self.render_group_headline(
            &context,
            &tool_calls,
            &tool_map,
            ToolNarrationPhase::Completed,
            locale.as_deref(),
        );
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

        // Fail the durable workflow fast on any determinism violation (EVE-530).
        // All tool.completed events have already been emitted above for affected calls.
        if let Some(fatal_msg) = results.iter().find_map(|r| r.determinism_fatal.as_deref()) {
            return Err(crate::error::AgentLoopError::tool(format!(
                "act activity aborted due to determinism violation: {fatal_msg}"
            )));
        }

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
        atom_context: &AtomContext,
        tool_def: Option<&ToolDefinition>,
        tool_call: &ToolCall,
        phase: ToolNarrationPhase,
        locale: Option<&str>,
    ) -> String {
        let wrapped_store = self.wrap_file_store_for_narration(atom_context);
        let ctx = ToolNarrationContext::new(wrapped_store.as_deref());
        for hook in &self.tool_call_hooks {
            if let Some(narration) = hook.narration(tool_def, tool_call, phase, locale, ctx) {
                return narration;
            }
        }
        render_tool_narration_with_locale(tool_def, tool_call, phase, locale)
    }

    fn render_group_headline(
        &self,
        atom_context: &AtomContext,
        tool_calls: &[ToolCall],
        tool_map: &std::collections::HashMap<&str, &ToolDefinition>,
        phase: ToolNarrationPhase,
        locale: Option<&str>,
    ) -> Option<String> {
        if tool_calls.is_empty() {
            return None;
        }
        if let [tool_call] = tool_calls {
            return Some(self.render_tool_narration(
                atom_context,
                tool_map.get(tool_call.name.as_str()).copied(),
                tool_call,
                phase,
                locale,
            ));
        }

        let actions = tool_calls
            .iter()
            .map(|tool_call| {
                let tool_def = tool_map.get(tool_call.name.as_str()).copied();
                let narration =
                    self.render_tool_narration(atom_context, tool_def, tool_call, phase, locale);
                let repeated_narration = self.render_tool_narration(
                    atom_context,
                    tool_def,
                    &tool_call_for_group_summary(tool_call),
                    phase,
                    locale,
                );
                GroupHeadlineAction::new(tool_call, narration, repeated_narration)
            })
            .collect::<Vec<_>>();

        Some(summarize_group_actions(&actions, locale))
    }

    /// Mirror the file-store wrapping applied during tool execution so
    /// path-bearing narration uses the same mount resolver and workspace key.
    fn wrap_file_store_for_narration(
        &self,
        atom_context: &AtomContext,
    ) -> Option<Arc<dyn SessionFileSystem>> {
        let store = self.context_services.file_store.as_ref()?.clone();
        let store = if let Some(workspace_id) = atom_context.workspace_id {
            crate::session_files::WorkspaceScopedFileSystem::wrap(store, workspace_id)
        } else {
            store
        };
        Some(crate::mount_fs::MountFs::wrap_if_needed(store))
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
        if let (Some(limiter), Some(ref org_id)) = (
            &self.outbound_tool_rate_limiter,
            self.context_services.org_id,
        ) && !limiter.check_org(org_id).await
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
                determinism_fatal: None,
            };
        }

        // Per-tool-call idempotency (EVE-530): claim before dispatch, replay if
        // already settled, refuse AtMostOnce re-execution on stale running claims.
        let claim_token = if let Some(ref store) = self.durable_tool_result_store {
            let turn_id = context.turn_id.to_string();
            match store
                .try_claim_tool_call(
                    &turn_id,
                    &tool_call.id,
                    &tool_call.name,
                    &tool_call_fingerprint,
                )
                .await
            {
                Ok(ToolCallClaimResult::Claimed { claim_token }) => Some(claim_token),

                Ok(ToolCallClaimResult::AlreadySettled {
                    result_json,
                    args_fingerprint: stored_fp,
                }) => {
                    // Determinism guard: stored args fingerprint must match current call.
                    if stored_fp != tool_call_fingerprint {
                        let err_msg = format!(
                            "determinism violation: tool '{}' replay args fingerprint \
                             does not match prior execution (stored={stored_fp}, \
                             current={})",
                            tool_call.name, tool_call_fingerprint
                        );
                        tracing::error!(
                            session_id = %context.session_id,
                            turn_id = %context.turn_id,
                            tool_call_id = %tool_call.id,
                            stored_fp = %stored_fp,
                            current_fp = %tool_call_fingerprint,
                            "ActAtom: determinism violation — replay args fingerprint mismatch"
                        );
                        let result_fp =
                            tool_result_fingerprint(&tool_call.name, &ToolResult::error(&err_msg));
                        let _ = self
                            .event_emitter
                            .emit(EventRequest::new(
                                context.session_id,
                                event_context,
                                ToolCompletedData::failure(
                                    tool_call.id.clone(),
                                    tool_call.name.clone(),
                                    "error".to_string(),
                                    err_msg.clone(),
                                    None,
                                )
                                .with_fingerprints(tool_call_fingerprint.clone(), result_fp)
                                .with_display_name(display_name.clone()),
                            ))
                            .await;
                        return ToolCallResult {
                            tool_call: tool_call.clone(),
                            result: ToolResult {
                                tool_call_id: tool_call.id.clone(),
                                result: None,
                                images: None,
                                error: Some(err_msg.clone()),
                                connection_required: None,
                                raw_output: None,
                            },
                            success: false,
                            status: "error".to_string(),
                            connection_required: None,
                            determinism_fatal: Some(err_msg),
                        };
                    }
                    tracing::debug!(
                        session_id = %context.session_id,
                        turn_id = %context.turn_id,
                        tool_call_id = %tool_call.id,
                        "ActAtom: replaying already-settled tool call"
                    );
                    // Emit a replayed tool.completed without re-emitting tool.started.
                    let replayed_result: ToolResult = serde_json::from_value(result_json.clone())
                        .unwrap_or(ToolResult {
                            tool_call_id: tool_call.id.clone(),
                            result: Some(result_json),
                            images: None,
                            error: None,
                            connection_required: None,
                            raw_output: None,
                        });
                    let success = replayed_result.error.is_none();
                    let status = if success { "success" } else { "error" };
                    let result_fp = tool_result_fingerprint(&tool_call.name, &replayed_result);
                    let completed_data = if success {
                        // Reconstruct content: text + images (preserves image-producing tools on replay)
                        let mut content = replayed_result
                            .result
                            .as_ref()
                            .map(|r| vec![ContentPart::tool_result_text(r)])
                            .unwrap_or_default();
                        if let Some(ref images) = replayed_result.images {
                            for img in images {
                                content.push(ContentPart::Image(
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
                            content,
                            None,
                        )
                        .with_fingerprints(tool_call_fingerprint.clone(), result_fp)
                        .with_display_name(display_name.clone())
                    } else {
                        ToolCompletedData::failure(
                            tool_call.id.clone(),
                            tool_call.name.clone(),
                            status.to_string(),
                            replayed_result.error.clone().unwrap_or_default(),
                            None,
                        )
                        .with_fingerprints(tool_call_fingerprint.clone(), result_fp)
                        .with_display_name(display_name.clone())
                    };
                    let _ = self
                        .event_emitter
                        .emit(EventRequest::new(
                            context.session_id,
                            event_context,
                            completed_data,
                        ))
                        .await;
                    let conn_req = replayed_result.connection_required.clone();
                    return ToolCallResult {
                        tool_call,
                        result: replayed_result,
                        success,
                        status: status.to_string(),
                        connection_required: conn_req,
                        determinism_fatal: None,
                    };
                }

                Ok(ToolCallClaimResult::AlreadyRunning {
                    args_fingerprint: stored_fp,
                }) => {
                    // Determinism guard: even in the running state, a fingerprint mismatch
                    // means the workflow is replaying with different args — fail loudly.
                    if stored_fp != tool_call_fingerprint {
                        let err_msg = format!(
                            "determinism violation: tool '{}' args fingerprint changed \
                             while prior claim is still running (stored={stored_fp}, \
                             current={tool_call_fingerprint})",
                            tool_call.name
                        );
                        tracing::error!(
                            session_id = %context.session_id,
                            turn_id = %context.turn_id,
                            tool_call_id = %tool_call.id,
                            stored = %stored_fp,
                            current = %tool_call_fingerprint,
                            "ActAtom: determinism violation — running claim fingerprint mismatch"
                        );
                        let result_fp =
                            tool_result_fingerprint(&tool_call.name, &ToolResult::error(&err_msg));
                        let _ = self
                            .event_emitter
                            .emit(EventRequest::new(
                                context.session_id,
                                event_context,
                                ToolCompletedData::failure(
                                    tool_call.id.clone(),
                                    tool_call.name.clone(),
                                    "error".to_string(),
                                    err_msg.clone(),
                                    None,
                                )
                                .with_fingerprints(tool_call_fingerprint.clone(), result_fp)
                                .with_display_name(display_name.clone()),
                            ))
                            .await;
                        return ToolCallResult {
                            tool_call: tool_call.clone(),
                            result: ToolResult {
                                tool_call_id: tool_call.id.clone(),
                                result: None,
                                images: None,
                                error: Some(err_msg.clone()),
                                connection_required: None,
                                raw_output: None,
                            },
                            success: false,
                            status: "error".to_string(),
                            connection_required: None,
                            determinism_fatal: Some(err_msg),
                        };
                    }

                    let sec = tool_def
                        .map(|d| d.side_effect_class())
                        .unwrap_or(SideEffectClass::AtMostOnce);
                    match sec {
                        SideEffectClass::Pure | SideEffectClass::Idempotent => {
                            // Safe to re-execute; proceed as normal (no claim token).
                            tracing::debug!(
                                session_id = %context.session_id,
                                tool_call_id = %tool_call.id,
                                "ActAtom: stale running claim for idempotent tool, re-executing"
                            );
                            None
                        }
                        SideEffectClass::AtMostOnce => {
                            tracing::warn!(
                                session_id = %context.session_id,
                                turn_id = %context.turn_id,
                                tool_call_id = %tool_call.id,
                                "ActAtom: AtMostOnce tool has stale running claim; returning interrupted result"
                            );
                            // Settle the stale claim as interrupted, then return an error.
                            let _ = store
                                .settle_tool_call(
                                    &turn_id,
                                    &tool_call.id,
                                    serde_json::Value::Null,
                                    "interrupted",
                                    Uuid::nil(), // sentinel — bypass token check for interrupt
                                )
                                .await;
                            let err_msg = format!(
                                "tool '{}' was interrupted mid-execution during a prior \
                                 worker failure; result is uncertain and was not re-run \
                                 (AtMostOnce safety)",
                                tool_call.name
                            );
                            let result_fp = tool_result_fingerprint(
                                &tool_call.name,
                                &ToolResult::error(&err_msg),
                            );
                            let _ = self
                                .event_emitter
                                .emit(EventRequest::new(
                                    context.session_id,
                                    event_context,
                                    ToolCompletedData::failure(
                                        tool_call.id.clone(),
                                        tool_call.name.clone(),
                                        "interrupted".to_string(),
                                        err_msg.clone(),
                                        None,
                                    )
                                    .with_fingerprints(tool_call_fingerprint.clone(), result_fp)
                                    .with_display_name(display_name.clone()),
                                ))
                                .await;
                            return ToolCallResult {
                                tool_call: tool_call.clone(),
                                result: ToolResult {
                                    tool_call_id: tool_call.id.clone(),
                                    result: None,
                                    images: None,
                                    error: Some(err_msg),
                                    connection_required: None,
                                    raw_output: None,
                                },
                                success: false,
                                status: "error".to_string(),
                                connection_required: None,
                                determinism_fatal: None,
                            };
                        }
                    }
                }

                Ok(ToolCallClaimResult::DeterminismViolation {
                    stored_fingerprint,
                    current_fingerprint,
                }) => {
                    let err_msg = format!(
                        "determinism violation: tool '{}' args fingerprint changed \
                         on replay (stored={stored_fingerprint}, \
                         current={current_fingerprint})",
                        tool_call.name
                    );
                    tracing::error!(
                        session_id = %context.session_id,
                        turn_id = %context.turn_id,
                        tool_call_id = %tool_call.id,
                        stored = %stored_fingerprint,
                        current = %current_fingerprint,
                        "ActAtom: determinism violation on claim"
                    );
                    let result_fp =
                        tool_result_fingerprint(&tool_call.name, &ToolResult::error(&err_msg));
                    let _ = self
                        .event_emitter
                        .emit(EventRequest::new(
                            context.session_id,
                            event_context,
                            ToolCompletedData::failure(
                                tool_call.id.clone(),
                                tool_call.name.clone(),
                                "error".to_string(),
                                err_msg.clone(),
                                None,
                            )
                            .with_fingerprints(tool_call_fingerprint.clone(), result_fp)
                            .with_display_name(display_name.clone()),
                        ))
                        .await;
                    return ToolCallResult {
                        tool_call: tool_call.clone(),
                        result: ToolResult {
                            tool_call_id: tool_call.id.clone(),
                            result: None,
                            images: None,
                            error: Some(err_msg.clone()),
                            connection_required: None,
                            raw_output: None,
                        },
                        success: false,
                        status: "error".to_string(),
                        connection_required: None,
                        determinism_fatal: Some(err_msg),
                    };
                }

                Err(e) => {
                    tracing::warn!(
                        session_id = %context.session_id,
                        tool_call_id = %tool_call.id,
                        error = %e,
                        "ActAtom: durable claim failed; proceeding without idempotency"
                    );
                    None
                }
            }
        } else {
            None
        };

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
                        context,
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
                        context,
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
                determinism_fatal: None,
            };
        };

        // Execute the tool (always with context so tools can emit progress events)
        let mut tool_context =
            ToolContext::from_services(context.session_id, &self.context_services);
        // Key file I/O by the attached workspace when known: pin the file store
        // to the workspace so shared-workspace sessions address the workspace's
        // files, not the session's own keyspace. For the default 1:1 case this
        // is a transparent pass-through.
        if let Some(workspace_id) = context.workspace_id {
            tool_context.workspace_id = workspace_id;
            if let Some(store) = tool_context.file_store.take() {
                tool_context.file_store = Some(
                    crate::session_files::WorkspaceScopedFileSystem::wrap(store, workspace_id),
                );
            }
        }
        // Resolve model paths through the mount resolver (EVE-660): `/workspace`
        // is a mount + cwd, not a per-store prefix. Applied over the
        // workspace-keyed store so resolution sits above re-keying.
        if let Some(store) = tool_context.file_store.take() {
            tool_context.file_store = Some(crate::mount_fs::MountFs::wrap_if_needed(store));
        }
        tool_context.visible_tool_names = Some(visible_tool_names.clone());
        // Input network_access (per-session, merged from harness+agent+session) takes precedence
        tool_context.network_access = network_access
            .cloned()
            .or_else(|| self.context_services.network_access.clone());
        // Provide event emitter + context so tools can emit tool.progress events
        if tool_context.event_emitter.is_none() {
            tool_context.event_emitter = Some(self.event_emitter.clone() as Arc<dyn EventEmitter>);
        }
        tool_context.event_context = Some(event_context.clone());
        tool_context.tool_call_id = Some(tool_call.id.clone());

        // Cooperative cancellation for this call. The guard fires when this
        // future is dropped — which is what a cancelled turn looks like from
        // here — and also on normal return, so the contract a tool sees is
        // simply "this call is over". Work the tool leaves running (a child
        // process, a detached watcher) can hold a clone and die with the call
        // instead of outliving it; dropping the future alone cannot tell it
        // anything, because a dropped future is never polled again.
        let call_cancellation = tokio_util::sync::CancellationToken::new();
        tool_context.cancellation = Some(call_cancellation.clone());
        let _cancel_on_call_end = call_cancellation.drop_guard();

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
                        .map(|r| vec![ContentPart::tool_result_text(r)])
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
                        context,
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
                        context,
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

                // Settle the durable claim (EVE-530).
                if let (Some(store), Some(token)) = (&self.durable_tool_result_store, claim_token) {
                    let result_snapshot =
                        serde_json::to_value(&tool_result).unwrap_or(serde_json::Value::Null);
                    match store
                        .settle_tool_call(
                            &context.turn_id.to_string(),
                            &tool_call.id,
                            result_snapshot,
                            "settled",
                            token,
                        )
                        .await
                    {
                        Ok(false) => {
                            tracing::warn!(
                                session_id = %context.session_id,
                                tool_call_id = %tool_call.id,
                                "ActAtom: settle ownership check failed (task reclaimed)"
                            );
                        }
                        Err(e) => {
                            tracing::warn!(
                                session_id = %context.session_id,
                                tool_call_id = %tool_call.id,
                                error = %e,
                                "ActAtom: settle_tool_call failed"
                            );
                        }
                        Ok(true) => {}
                    }
                }

                let conn_req = tool_result.connection_required.clone();
                ToolCallResult {
                    tool_call,
                    result: tool_result,
                    success,
                    status: status.to_string(),
                    connection_required: conn_req,
                    determinism_fatal: None,
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
                            context,
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
                    determinism_fatal: None,
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
    use crate::event_emitter::NoopEventEmitter;
    use crate::tools::ToolRegistry;
    use crate::typed_id::{AgentId, HarnessId, MessageId, SessionId, TurnId};
    use async_trait::async_trait;
    use serde_json::json;

    struct ArgumentEchoTool;

    struct NarratingGrepTool;

    struct HumanIntentFixtureHook;

    impl crate::capabilities::ToolCallHook for HumanIntentFixtureHook {
        fn narration(
            &self,
            _tool_def: Option<&crate::ToolDefinition>,
            tool_call: &crate::ToolCall,
            _phase: crate::tool_narration::ToolNarrationPhase,
            _locale: Option<&str>,
            _ctx: crate::tool_narration::ToolNarrationContext<'_>,
        ) -> Option<String> {
            crate::tool_types::human_intent(&tool_call.arguments).map(str::to_string)
        }

        fn transform_for_execution(&self, mut tool_call: crate::ToolCall) -> crate::ToolCall {
            tool_call.arguments = tool_call.execution_arguments();
            tool_call
        }
    }

    #[async_trait]
    impl crate::tools::Tool for NarratingGrepTool {
        fn name(&self) -> &str {
            "grep_files"
        }

        fn description(&self) -> &str {
            "Search files"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            json!({"type": "object"})
        }

        async fn execute(&self, _arguments: serde_json::Value) -> crate::ToolExecutionResult {
            crate::ToolExecutionResult::success(json!({}))
        }

        fn narrate(
            &self,
            tool_call: &crate::ToolCall,
            phase: crate::tool_narration::ToolNarrationPhase,
            locale: Option<&str>,
            _ctx: crate::tool_narration::ToolNarrationContext<'_>,
        ) -> Option<String> {
            Some(crate::tool_narration::narrate_grep_files(
                &tool_call.arguments,
                phase,
                locale,
            ))
        }
    }

    struct NarratingCapability;

    #[async_trait]
    impl crate::Capability for NarratingCapability {
        fn id(&self) -> &str {
            "narrating_test"
        }

        fn name(&self) -> &str {
            "Narrating test"
        }

        fn description(&self) -> &str {
            "Test-only narration capability"
        }

        fn tools(&self) -> Vec<Box<dyn crate::Tool>> {
            vec![Box::new(NarratingGrepTool)]
        }
    }

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

    #[test]
    fn grouped_headline_uses_tool_owned_narration_for_repeated_actions() {
        use crate::capabilities::{Capability, CapabilityNarrationHook};

        let capability: Arc<dyn Capability> = Arc::new(NarratingCapability);
        let tool_definitions = capability
            .tools()
            .into_iter()
            .map(|tool| tool.to_definition())
            .collect::<Vec<_>>();
        let tool_map = tool_definitions
            .iter()
            .map(|tool_def| (tool_def.name(), tool_def))
            .collect::<std::collections::HashMap<_, _>>();
        let atom = ActAtom::new(ToolRegistry::new(), NoopEventEmitter)
            .with_tool_call_hooks(vec![Arc::new(CapabilityNarrationHook(capability))]);
        let context = AtomContext::new(SessionId::new(), TurnId::new(), MessageId::new());
        let tool_calls = vec![
            ToolCall {
                id: "grep-1".to_string(),
                name: "grep_files".to_string(),
                arguments: json!({ "pattern": "full_name" }),
            },
            ToolCall {
                id: "grep-2".to_string(),
                name: "grep_files".to_string(),
                arguments: json!({ "pattern": "login" }),
            },
        ];

        assert_eq!(
            atom.render_group_headline(
                &context,
                &tool_calls,
                &tool_map,
                ToolNarrationPhase::Started,
                None,
            )
            .as_deref(),
            Some("Searching files twice")
        );
        assert_eq!(
            atom.render_group_headline(
                &context,
                &tool_calls,
                &tool_map,
                ToolNarrationPhase::Completed,
                None,
            )
            .as_deref(),
            Some("Searched files twice")
        );
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
            context: &crate::tool_context::ToolContext,
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

    /// A tool that leaves work running past its own future: it hands the
    /// call's cancellation token to a detached task and returns immediately.
    /// That task is the thing a dropped future cannot reach.
    struct DetachedWorkTool {
        cancelled_tx: Arc<std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
    }

    impl DetachedWorkTool {
        fn new(cancelled_tx: tokio::sync::oneshot::Sender<()>) -> Self {
            Self {
                cancelled_tx: Arc::new(std::sync::Mutex::new(Some(cancelled_tx))),
            }
        }
    }

    #[async_trait]
    impl crate::tools::Tool for DetachedWorkTool {
        fn name(&self) -> &str {
            "detached_work"
        }

        fn description(&self) -> &str {
            "spawns work that outlives the call unless cancelled"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            json!({ "type": "object", "properties": {} })
        }

        fn requires_context(&self) -> bool {
            true
        }

        async fn execute(&self, _arguments: serde_json::Value) -> crate::ToolExecutionResult {
            crate::ToolExecutionResult::tool_error("requires context")
        }

        async fn execute_with_context(
            &self,
            _arguments: serde_json::Value,
            context: &crate::tool_context::ToolContext,
        ) -> crate::ToolExecutionResult {
            let token = context
                .cancellation
                .clone()
                .expect("act must supply a cancellation token");
            assert!(!token.is_cancelled(), "token is live during the call");
            let tx = self.cancelled_tx.clone();
            tokio::spawn(async move {
                token.cancelled().await;
                if let Ok(mut guard) = tx.lock()
                    && let Some(tx) = guard.take()
                {
                    let _ = tx.send(());
                }
            });
            crate::ToolExecutionResult::success(json!({ "spawned": true }))
        }
    }

    /// Work a tool leaves running must learn that its call ended. Dropping the
    /// act future cannot tell it — a dropped future is never polled again — so
    /// the token on `ToolContext` is the only signal that reaches it.
    #[tokio::test]
    async fn test_act_atom_cancels_detached_tool_work_when_the_call_ends() {
        let (cancelled_tx, cancelled_rx) = tokio::sync::oneshot::channel();

        let mut executor = ToolRegistry::new();
        executor.register(DetachedWorkTool::new(cancelled_tx));

        let atom = ActAtom::new(executor, NoopEventEmitter);
        let context = AtomContext::new(SessionId::new(), TurnId::new(), MessageId::new());
        let input = ActInput {
            org_id: Some(1),
            context,
            harness_id: HarnessId::from_seed(1),
            agent_id: Some(AgentId::new()),
            tool_calls: vec![ToolCall {
                id: "call_1".to_string(),
                name: "detached_work".to_string(),
                arguments: json!({}),
            }],
            tool_definitions: vec![recording_tool_def("detached_work", None, false)],
            locale: None,
            blueprint_id: None,
            network_access: None,
            parallel_tool_calls: None,
        };

        atom.execute(input).await.expect("act should succeed");

        tokio::time::timeout(std::time::Duration::from_secs(1), cancelled_rx)
            .await
            .expect("detached work should be cancelled once the call ends")
            .expect("cancellation signal should be sent");
    }

    #[tokio::test]
    async fn test_act_atom_cancels_detached_tool_work_when_the_turn_is_cancelled() {
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

        // The existing abort path still holds: the tool future itself is dropped.
        tokio::time::timeout(std::time::Duration::from_secs(1), dropped_rx)
            .await
            .expect("tool future should be dropped when the turn is cancelled")
            .expect("drop signal should be sent");
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
        let mut executor = ToolRegistry::new();
        executor.register(ArgumentEchoTool);
        let tool_def = executor.get("argument_echo").unwrap().to_definition();
        let emitter = crate::test_fixtures::TestEventEmitter::new();
        let atom = ActAtom::new(executor, emitter.clone())
            .with_tool_call_hooks(vec![std::sync::Arc::new(HumanIntentFixtureHook)]);

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
        let executor = ToolRegistry::new();
        let emitter = crate::test_fixtures::TestEventEmitter::new();
        let atom = ActAtom::new(executor, emitter)
            .with_tool_call_hooks(vec![std::sync::Arc::new(HumanIntentFixtureHook)]);

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
                determinism_fatal: None,
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
        impl crate::tool_execution::OutboundToolRateLimiter for DenyAll {
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
        impl crate::tool_execution::OutboundToolRateLimiter for AllowAll {
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

    // -----------------------------------------------------------------------
    // DurableToolResultStore idempotency tests (EVE-530)
    // -----------------------------------------------------------------------

    use crate::tool_types::{SideEffectClass, ToolHints};
    use crate::{durability::DurableToolResultStore, durability::ToolCallClaimResult};
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Default)]
    struct InMemoryDurableStore {
        rows: Mutex<HashMap<(String, String), StoreRow>>,
    }

    #[derive(Clone)]
    struct StoreRow {
        status: String,
        result_json: serde_json::Value,
        args_fingerprint: String,
        #[allow(dead_code)]
        claim_token: Uuid,
    }

    #[async_trait]
    impl DurableToolResultStore for InMemoryDurableStore {
        async fn try_claim_tool_call(
            &self,
            turn_id: &str,
            tool_call_id: &str,
            _tool_name: &str,
            args_fingerprint: &str,
        ) -> crate::error::Result<ToolCallClaimResult> {
            let key = (turn_id.to_string(), tool_call_id.to_string());
            let mut rows = self.rows.lock().unwrap();
            if let Some(row) = rows.get(&key) {
                match row.status.as_str() {
                    "settled" => {
                        if row.args_fingerprint != args_fingerprint {
                            return Ok(ToolCallClaimResult::DeterminismViolation {
                                stored_fingerprint: row.args_fingerprint.clone(),
                                current_fingerprint: args_fingerprint.to_string(),
                            });
                        }
                        return Ok(ToolCallClaimResult::AlreadySettled {
                            result_json: row.result_json.clone(),
                            args_fingerprint: row.args_fingerprint.clone(),
                        });
                    }
                    _ => {
                        return Ok(ToolCallClaimResult::AlreadyRunning {
                            args_fingerprint: row.args_fingerprint.clone(),
                        });
                    }
                }
            }
            let token = Uuid::new_v4();
            rows.insert(
                key,
                StoreRow {
                    status: "running".to_string(),
                    result_json: serde_json::Value::Null,
                    args_fingerprint: args_fingerprint.to_string(),
                    claim_token: token,
                },
            );
            Ok(ToolCallClaimResult::Claimed { claim_token: token })
        }

        async fn settle_tool_call(
            &self,
            turn_id: &str,
            tool_call_id: &str,
            result_json: serde_json::Value,
            status: &str,
            _claim_token: Uuid,
        ) -> crate::error::Result<bool> {
            let key = (turn_id.to_string(), tool_call_id.to_string());
            let mut rows = self.rows.lock().unwrap();
            if let Some(row) = rows.get_mut(&key) {
                row.status = status.to_string();
                row.result_json = result_json;
                return Ok(true);
            }
            Ok(false)
        }

        async fn get_tool_call_status(
            &self,
            turn_id: &str,
            tool_call_id: &str,
        ) -> crate::error::Result<Option<crate::durability::DurableToolCallStatus>> {
            let key = (turn_id.to_string(), tool_call_id.to_string());
            let rows = self.rows.lock().unwrap();
            Ok(rows.get(&key).map(|row| match row.status.as_str() {
                "settled" => crate::durability::DurableToolCallStatus::Settled {
                    result_json: row.result_json.clone(),
                },
                "interrupted" => crate::durability::DurableToolCallStatus::Interrupted {
                    result_json: Some(row.result_json.clone()),
                },
                _ => crate::durability::DurableToolCallStatus::Running,
            }))
        }
    }

    fn make_act_input_with_store(
        tool_call: ToolCall,
        tool_defs: Vec<ToolDefinition>,
        context: AtomContext,
    ) -> ActInput {
        ActInput {
            org_id: None,
            context,
            harness_id: HarnessId::from_seed(1),
            agent_id: Some(AgentId::new()),
            tool_calls: vec![tool_call],
            tool_definitions: tool_defs,
            locale: None,
            blueprint_id: None,
            network_access: None,
            parallel_tool_calls: None,
        }
    }

    fn arg_echo_tool_def(side_effect: SideEffectClass) -> ToolDefinition {
        ToolDefinition::Builtin(crate::BuiltinTool {
            name: "argument_echo".to_string(),
            display_name: None,
            description: "echo".to_string(),
            parameters: json!({"type": "object"}),
            policy: Default::default(),
            category: None,
            deferrable: Default::default(),
            hints: ToolHints::default().with_side_effect_class(side_effect),
            full_parameters: None,
        })
    }

    /// First execution succeeds normally and the result is settled in the store.
    #[tokio::test]
    async fn test_idempotency_first_execution_claims_and_settles() {
        let store = Arc::new(InMemoryDurableStore::default());
        let mut executor = ToolRegistry::with_defaults();
        executor.register(ArgumentEchoTool);
        let atom =
            ActAtom::new(executor, NoopEventEmitter).with_durable_tool_result_store(store.clone());

        let context = AtomContext::new(SessionId::new(), TurnId::new(), MessageId::new());
        let tc = ToolCall {
            id: "c1".to_string(),
            name: "argument_echo".to_string(),
            arguments: json!({"value": "hello"}),
        };
        let input = make_act_input_with_store(
            tc,
            vec![arg_echo_tool_def(SideEffectClass::AtMostOnce)],
            context,
        );

        let result = atom.execute(input).await.unwrap();
        assert_eq!(result.success_count, 1);
        assert_eq!(result.error_count, 0);

        // Row should be settled now.
        let rows = store.rows.lock().unwrap();
        let row = rows.values().next().unwrap();
        assert_eq!(row.status, "settled");
    }

    /// Second execution replays the stored result without re-running the tool.
    #[tokio::test]
    async fn test_idempotency_replay_already_settled() {
        use crate::tool_fingerprint::tool_call_fingerprint;

        let store = Arc::new(InMemoryDurableStore::default());
        let tc = ToolCall {
            id: "c1".to_string(),
            name: "argument_echo".to_string(),
            arguments: json!({"value": "hello"}),
        };
        let fp = tool_call_fingerprint(&tc);

        // Pre-populate as settled.
        {
            let stored_result = serde_json::to_value(crate::ToolResult {
                tool_call_id: "c1".to_string(),
                result: Some(json!({"value": "hello"})),
                images: None,
                error: None,
                connection_required: None,
                raw_output: None,
            })
            .unwrap();
            store.rows.lock().unwrap().insert(
                (
                    "turn_00000000000000000000000000000000".to_string(),
                    "c1".to_string(),
                ),
                StoreRow {
                    status: "settled".to_string(),
                    result_json: stored_result,
                    args_fingerprint: fp,
                    claim_token: Uuid::new_v4(),
                },
            );
        }

        let mut executor = ToolRegistry::with_defaults();
        executor.register(ArgumentEchoTool);
        let atom =
            ActAtom::new(executor, NoopEventEmitter).with_durable_tool_result_store(store.clone());

        let context = AtomContext::new(
            SessionId::new(),
            TurnId::from_uuid(Uuid::nil()),
            MessageId::new(),
        );
        let input = make_act_input_with_store(
            tc,
            vec![arg_echo_tool_def(SideEffectClass::AtMostOnce)],
            context,
        );

        let result = atom.execute(input).await.unwrap();
        assert_eq!(result.success_count, 1, "replay should count as success");
        assert_eq!(result.error_count, 0);
    }

    /// AtMostOnce tool with a stale running claim returns an interrupted error.
    #[tokio::test]
    async fn test_idempotency_at_most_once_stale_running_returns_interrupted() {
        use crate::tool_fingerprint::tool_call_fingerprint;

        let store = Arc::new(InMemoryDurableStore::default());
        let tc = ToolCall {
            id: "c1".to_string(),
            name: "argument_echo".to_string(),
            arguments: json!({"value": "x"}),
        };
        let fp = tool_call_fingerprint(&tc);

        // Pre-populate as running (stale from dead worker).
        store.rows.lock().unwrap().insert(
            (
                "turn_00000000000000000000000000000000".to_string(),
                "c1".to_string(),
            ),
            StoreRow {
                status: "running".to_string(),
                result_json: serde_json::Value::Null,
                args_fingerprint: fp,
                claim_token: Uuid::new_v4(),
            },
        );

        let mut executor = ToolRegistry::with_defaults();
        executor.register(ArgumentEchoTool);
        let atom =
            ActAtom::new(executor, NoopEventEmitter).with_durable_tool_result_store(store.clone());

        let context = AtomContext::new(
            SessionId::new(),
            TurnId::from_uuid(Uuid::nil()),
            MessageId::new(),
        );
        let input = make_act_input_with_store(
            tc,
            vec![arg_echo_tool_def(SideEffectClass::AtMostOnce)],
            context,
        );

        let result = atom.execute(input).await.unwrap();
        assert_eq!(
            result.error_count, 1,
            "AtMostOnce stale running should error"
        );
        let err = result.results[0].result.error.as_deref().unwrap_or("");
        assert!(
            err.contains("interrupted"),
            "error should mention interrupted: {err}"
        );

        // Row should be settled as interrupted.
        let rows = store.rows.lock().unwrap();
        let row = rows.values().next().unwrap();
        assert_eq!(row.status, "interrupted");
    }

    /// Pure/Idempotent tool with a stale running claim proceeds to execution normally.
    #[tokio::test]
    async fn test_idempotency_idempotent_tool_stale_running_reexecutes() {
        use crate::tool_fingerprint::tool_call_fingerprint;

        let store = Arc::new(InMemoryDurableStore::default());
        let tc = ToolCall {
            id: "c1".to_string(),
            name: "argument_echo".to_string(),
            arguments: json!({"value": "x"}),
        };
        let fp = tool_call_fingerprint(&tc);

        store.rows.lock().unwrap().insert(
            (
                "turn_00000000000000000000000000000000".to_string(),
                "c1".to_string(),
            ),
            StoreRow {
                status: "running".to_string(),
                result_json: serde_json::Value::Null,
                args_fingerprint: fp,
                claim_token: Uuid::new_v4(),
            },
        );

        let mut executor = ToolRegistry::with_defaults();
        executor.register(ArgumentEchoTool);
        let atom =
            ActAtom::new(executor, NoopEventEmitter).with_durable_tool_result_store(store.clone());

        let context = AtomContext::new(
            SessionId::new(),
            TurnId::from_uuid(Uuid::nil()),
            MessageId::new(),
        );
        let input = make_act_input_with_store(
            tc,
            vec![arg_echo_tool_def(SideEffectClass::Idempotent)],
            context,
        );

        let result = atom.execute(input).await.unwrap();
        assert_eq!(
            result.success_count, 1,
            "Idempotent should re-execute successfully"
        );
        assert_eq!(result.error_count, 0);
    }
}

//! Scheduled tool execution through [`tool_scheduler`].
//! Calls run concurrently within the configured bound; matching concurrency
//! classes serialize mutation and CPU-bound tools use separate tasks.
//! Timeouts, errors, and cancellation return normal per-tool outcomes.
//! Lifecycle events provide canonical history and observability.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use web_time::Instant;

use super::ExecutionContext;
use super::act_hooks::{self, PostActHook};
use super::tool_scheduler;
use crate::engine::error::Result;
use crate::engine::events::{
    ActCompletedData, ActStartedData, EventContext, EventRequest, ToolCompletedData,
    ToolStartedData,
};
use crate::engine::message::ContentPart;
use crate::engine::phase_effects::{PhaseEffectEmitter, PhaseEffectSink};
use crate::engine::tool_fingerprint::{
    tool_call_fingerprint, tool_error_fingerprint, tool_result_fingerprint,
};
use crate::engine::tool_narration::{
    GroupHeadlineAction, ToolNarrationContext, ToolNarrationPhase,
    render_tool_narration_with_locale, summarize_group_actions, tool_call_for_group_summary,
};
use crate::engine::tool_types::ConnectionRequired;
use crate::engine::tool_types::{SideEffectClass, ToolCall, ToolDefinition, ToolResult};
use crate::engine::typed_id::{AgentId, HarnessId};
use crate::engine::{
    durability::DurableToolResultStore, durability::ToolCallClaimResult,
    event_emitter::EventEmitter, execution_loading::AgentStore, execution_loading::SessionStore,
    session_files::SessionFileSystem, tool_execution::ToolExecutor,
};
use uuid::Uuid;

/// A Tokio task handle that aborts its task if the parent future is dropped
/// before the task completes. Tokio detaches a bare [`tokio::task::JoinHandle`]
/// on drop, but tool execution must not outlive Act cancellation.
struct AbortOnDropJoinHandle<T> {
    handle: everruns_contracts::rt::JoinHandle<T>,
}

impl<T> AbortOnDropJoinHandle<T> {
    fn new(handle: everruns_contracts::rt::JoinHandle<T>) -> Self {
        Self { handle }
    }
}

impl<T> Future for AbortOnDropJoinHandle<T> {
    type Output = std::result::Result<T, everruns_contracts::rt::JoinError>;

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
    pub context: ExecutionContext,
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
    pub network_access: Option<crate::engine::network_access::NetworkAccessList>,
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
    /// If set, the tool requires a connection before it can execute.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection_required: Option<ConnectionRequired>,
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
    /// True when the pause is specifically a URL mode elicitation waiting on a
    /// human to open a link. Kept apart from the generic flag because only a
    /// client that renders the consent card can answer it — see
    /// `plan_after_act`, which will not hold a turn for a card nobody can draw.
    #[serde(default, skip_serializing_if = "is_false")]
    pub waiting_for_url_elicitation: bool,
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
    E: PhaseEffectSink,
{
    // Held as `Arc` so individual `cpu_bound` tool calls can be offloaded to
    // their own task (`tokio::spawn`) without borrowing `self` for `'static`.
    tool_executor: Arc<T>,
    event_emitter: PhaseEffectEmitter<E>,
    /// Runtime-owned service snapshot cloned into every per-call ToolContext.
    context_services: crate::engine::tool_context::ToolContextServices,
    /// Optional per-org outbound tool-call rate limiter (TM-TOOL-009).
    /// When present, each tool call increments the org counter; calls that
    /// exceed the per-org window return a tool error rather than a hard failure.
    outbound_tool_rate_limiter:
        Option<Arc<dyn crate::engine::tool_execution::OutboundToolRateLimiter>>,
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
    /// Pre-tool-use hooks (capability-contributed): run before each server tool
    /// execution and before a client-side call is emitted. Capabilities wire
    /// these in via the user-hooks adapter chain (see `crate::engine::hook_adapter`).
    /// Hooks can mutate the `ToolCall` (returning `Continue`) or refuse it
    /// (returning `Block` or `Defer`).
    pre_tool_hooks: Vec<Arc<dyn act_hooks::PreToolUseHook>>,
    /// Tool-call hooks (capability-contributed): inspect model-authored tool
    /// calls for UI narration and transform calls before actual execution.
    tool_call_hooks: Vec<Arc<dyn crate::engine::capabilities::ToolCallHook>>,
    /// Final post-tool-exec hooks (infrastructure): run after capability hooks.
    /// Always registered, cannot be removed by capabilities (EVE-225).
    final_post_tool_hooks: Vec<Arc<dyn act_hooks::PostToolExecHook>>,
}

impl<T, E> ActAtom<T, E>
where
    T: ToolExecutor,
    E: PhaseEffectSink,
{
    /// Create a new ActAtom with default hooks (ConnectionSetup + ClientSideTool).
    pub fn new(tool_executor: T, event_emitter: E) -> Self {
        Self {
            tool_executor: Arc::new(tool_executor),
            event_emitter: PhaseEffectEmitter::new(Arc::new(event_emitter)),
            context_services: crate::engine::tool_context::ToolContextServices::default(),
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
            event_emitter: PhaseEffectEmitter::new(Arc::new(event_emitter)),
            context_services: crate::engine::tool_context::ToolContextServices {
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
        services: crate::engine::tool_context::ToolContextServices,
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

    /// Default hooks, in order. FormElicitation and ToolApprovalPause append synthetic calls, so
    /// they must precede ClientSideTool, which emits `tool.call_requested` for client-side calls.
    fn default_hooks() -> Vec<Box<dyn PostActHook>> {
        vec![
            Box::new(act_hooks::ConnectionSetupHook),
            Box::new(act_hooks::UrlElicitationHook),
            Box::new(act_hooks::FormElicitationHook),
            Box::new(act_hooks::ToolApprovalPauseHook),
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
        store: Arc<dyn crate::engine::session_services::SessionStorageStore>,
    ) -> Self {
        self.context_services.storage_store = Some(store);
        self
    }

    /// Set the image artifact store on this atom
    pub fn with_image_store(
        mut self,
        store: Arc<dyn crate::engine::image_services::ImageArtifactStore>,
    ) -> Self {
        self.context_services.image_store = Some(store);
        self
    }

    /// Set the provider credential store on this atom
    pub fn with_provider_credential_store(
        mut self,
        store: Arc<dyn crate::engine::connection_services::ProviderCredentialStore>,
    ) -> Self {
        self.context_services.provider_credential_store = Some(store);
        self
    }

    /// Set the utility LLM service on this atom.
    pub fn with_utility_llm_service(
        mut self,
        service: Arc<dyn crate::engine::UtilityLlmService>,
    ) -> Self {
        self.context_services.utility_llm_service = Some(service);
        self
    }

    /// Set the scoped-MCP tool invoker on this atom (guardrails `mcp` check).
    pub fn with_mcp_invoker(mut self, invoker: Arc<dyn crate::engine::McpToolInvoker>) -> Self {
        self.context_services.mcp_invoker = Some(invoker);
        self
    }

    /// Set the outbound egress service on this atom.
    pub fn with_egress_service(mut self, service: Arc<dyn crate::engine::EgressService>) -> Self {
        self.context_services.egress_service = Some(service);
        self
    }

    /// Set the user connection resolver on this atom
    pub fn with_connection_resolver(
        mut self,
        resolver: Arc<dyn crate::engine::connection_services::UserConnectionResolver>,
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
        store: Arc<dyn crate::engine::session_services::SessionScheduleStore>,
    ) -> Self {
        self.context_services.schedule_store = Some(store);
        self
    }

    /// Set platform store for org-level management tools.
    pub fn with_subagent_delegate(
        mut self,
        store: Arc<dyn crate::engine::subagent_delegation::SubagentSessionDelegate>,
    ) -> Self {
        self.context_services.subagent_delegate = Some(store);
        self
    }

    /// Set leased resource store for lifecycle-managed provider resources.
    pub fn with_leased_resource_store(
        mut self,
        store: Arc<dyn crate::engine::session_services::LeasedResourceStore>,
    ) -> Self {
        self.context_services.leased_resource_store = Some(store);
        self
    }

    /// Set session resource registry.
    pub fn with_session_resource_registry(
        mut self,
        registry: Arc<dyn crate::engine::session_services::SessionResourceRegistry>,
    ) -> Self {
        self.context_services.session_resource_registry = Some(registry);
        self
    }

    /// Add a session task registry passed to tool contexts.
    pub fn with_session_task_registry(
        mut self,
        registry: Arc<dyn crate::engine::session_task::SessionTaskRegistry>,
    ) -> Self {
        self.context_services.session_task_registry = Some(registry);
        self
    }

    pub fn with_capability_registry(
        mut self,
        registry: crate::engine::capabilities::CapabilityRegistry,
    ) -> Self {
        self.context_services.capability_registry = Some(registry);
        self
    }

    /// Set the active built-in tool registry for meta-tools like `spawn_background`.
    pub fn with_tool_registry(mut self, registry: Arc<crate::engine::tools::ToolRegistry>) -> Self {
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
        hooks: Vec<Arc<dyn crate::engine::capabilities::ToolCallHook>>,
    ) -> Self {
        self.tool_call_hooks.extend(hooks);
        self
    }

    /// Set org ID for org-scoped operations.
    pub fn with_org_id(mut self, org_id: crate::engine::typed_id::OrgId) -> Self {
        self.context_services.org_id = Some(org_id);
        self
    }

    /// Set the merged network access list for URL filtering in tools.
    pub fn with_network_access(
        mut self,
        network_access: Option<crate::engine::network_access::NetworkAccessList>,
    ) -> Self {
        self.context_services.network_access = network_access;
        self
    }

    /// Set the budget checker for the check_budget tool.
    pub fn with_budget_checker(
        mut self,
        checker: Arc<dyn crate::engine::tool_execution::BudgetChecker>,
    ) -> Self {
        self.context_services.budget_checker = Some(checker);
        self
    }

    /// Set the internal payment authority for paid capability tools.
    pub fn with_payment_authority(
        mut self,
        authority: Arc<dyn crate::engine::tool_execution::PaymentAuthority>,
    ) -> Self {
        self.context_services.payment_authority = Some(authority);
        self
    }

    /// Set the authority used to authorize detached peer-session creation.
    pub fn with_session_creation_authority(
        mut self,
        authority: Arc<dyn crate::engine::delegation_services::SessionCreationAuthority>,
    ) -> Self {
        self.context_services.session_creation_authority = Some(authority);
        self
    }

    /// Set the per-org outbound tool-call rate limiter (TM-TOOL-009).
    pub fn with_outbound_tool_rate_limiter(
        mut self,
        limiter: Arc<dyn crate::engine::tool_execution::OutboundToolRateLimiter>,
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
        store: Arc<dyn crate::engine::delegation_services::SubagentSpawnStore>,
    ) -> Self {
        self.context_services.subagent_spawn_store = Some(store);
        self
    }

    /// Set the resolved subagent nesting policy for tool contexts.
    pub fn with_subagent_nesting_policy(
        mut self,
        policy: crate::engine::delegation_services::SubagentNestingPolicy,
    ) -> Self {
        self.context_services.subagent_nesting_policy = policy;
        self
    }

    /// Set the live reasoning-effort handle (EVE-595). When set, each tool's
    /// `ToolContext` receives a clone so a tool can change the reasoning effort
    /// mid-turn for subsequent LLM steps in the same turn.
    pub fn with_reasoning_effort_handle(
        mut self,
        handle: crate::engine::tool_context::ReasoningEffortHandle,
    ) -> Self {
        self.context_services.reasoning_effort_handle = Some(handle);
        self
    }
}

impl<T, E> ActAtom<T, E>
where
    T: ToolExecutor + Send + Sync + 'static,
    E: EventEmitter + Send + Sync + 'static,
{
    /// Stable phase name used by logs and durable activity adapters.
    pub fn name(&self) -> &'static str {
        "act"
    }

    /// Execute one scheduled tool-call batch through injected contracts.
    pub async fn execute(&self, input: ActInput) -> Result<ActResult> {
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
        let (server_tool_calls, client_tool_calls): (Vec<_>, Vec<_>) = tool_calls
            .into_iter()
            .partition(|tc| act_hooks::runs_on_server(tc, &tool_definitions));

        let client_tool_calls: Vec<_> = client_tool_calls
            .into_iter()
            .map(|tool_call| self.transform_tool_call_for_execution(tool_call))
            .collect();

        // THREAT[TM-CLIENT-005]: gate client calls before any execution request.
        let (client_tool_calls, client_tool_definitions, client_policy_results) =
            client_policy::apply_pre_tool_policy(
                self,
                &context,
                client_tool_calls,
                &tool_definitions,
                network_access.as_ref(),
                locale.as_deref(),
            )
            .await;

        if server_tool_calls.is_empty()
            && client_tool_calls.is_empty()
            && client_policy_results.is_empty()
        {
            return Ok(ActResult {
                results: vec![],
                completed: true,
                success_count: 0,
                error_count: 0,
                waiting_for_tool_results: false,
                waiting_for_url_elicitation: false,
                blocked: false,
                client_tool_calls: vec![],
                client_tool_definitions: vec![],
            });
        }

        // No server tools. Post-act hooks emit tool.call_requested only for
        // client calls the pre-tool chain allowed.
        if server_tool_calls.is_empty() {
            let success_count = client_policy_results
                .iter()
                .filter(|result| result.success)
                .count() as u32;
            let error_count = client_policy_results.len() as u32 - success_count;
            let mut result = ActResult {
                results: client_policy_results,
                completed: true,
                success_count,
                error_count,
                waiting_for_tool_results: false,
                waiting_for_url_elicitation: false,
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
        let event_context = EventContext::from_execution_context(&context).with_span(
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

        // Count successes and errors, including client calls already settled
        // by the pre-tool chain. Those never reach the server scheduler.
        let policy_success = client_policy_results
            .iter()
            .filter(|result| result.success)
            .count() as u32;
        let policy_errors = client_policy_results.len() as u32 - policy_success;
        let success_count = results.iter().filter(|r| r.success).count() as u32 + policy_success;
        let error_count = results.iter().filter(|r| !r.success).count() as u32 + policy_errors;

        // Calculate act phase duration
        let act_duration_ms = act_start.elapsed().as_millis() as u64;

        // Emit act.completed event (same span as act.started, parent is turn)
        let completed_context = EventContext::from_execution_context(&context).with_span(
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
            let suffix =
                crate::engine::localization::format_error_suffix(locale.as_deref(), error_count);
            completed_headline = Some(match completed_headline {
                Some(text) => format!("{text}{suffix}"),
                None => crate::engine::localization::format_completed_tool_batch(
                    locale.as_deref(),
                    error_count,
                ),
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
            return Err(crate::engine::error::AgentLoopError::tool(format!(
                "act activity aborted due to determinism violation: {fatal_msg}"
            )));
        }

        let mut results = results;
        results.extend(client_policy_results);
        let mut act_result = ActResult {
            results,
            completed: true,
            success_count,
            error_count,
            waiting_for_tool_results: false,
            waiting_for_url_elicitation: false,
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
        execution_context: &ExecutionContext,
        tool_def: Option<&ToolDefinition>,
        tool_call: &ToolCall,
        phase: ToolNarrationPhase,
        locale: Option<&str>,
    ) -> String {
        let wrapped_store = self.wrap_file_store_for_narration(execution_context);
        let ctx = ToolNarrationContext::new(wrapped_store.as_deref());
        for hook in &self.tool_call_hooks {
            if let Some(narration) = hook.narration(tool_def, tool_call, phase, locale, ctx) {
                return narration;
            }
        }
        // Capability hooks only reach tools a capability lists in `tools()`.
        // Tools assembled outside any capability — the unified `spawn_agent`
        // dispatcher built from delegation targets, registry-augmented and
        // proxied tools — still own narration, so ask the executing tool
        // directly before falling back to the generic display-name phrasing.
        if let Some(narration) = self
            .context_services
            .tool_registry
            .as_ref()
            .and_then(|registry| registry.get(&tool_call.name))
            .and_then(|tool| tool.narrate(tool_call, phase, locale, ctx))
        {
            return narration;
        }
        render_tool_narration_with_locale(tool_def, tool_call, phase, locale)
    }

    fn render_group_headline(
        &self,
        execution_context: &ExecutionContext,
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
                execution_context,
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
                let narration = self.render_tool_narration(
                    execution_context,
                    tool_def,
                    tool_call,
                    phase,
                    locale,
                );
                let repeated_narration = self.render_tool_narration(
                    execution_context,
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
        execution_context: &ExecutionContext,
    ) -> Option<Arc<dyn SessionFileSystem>> {
        let store = self.context_services.file_store.as_ref()?.clone();
        let store = if let Some(workspace_id) = execution_context.workspace_id {
            crate::engine::session_files::WorkspaceScopedFileSystem::wrap(store, workspace_id)
        } else {
            store
        };
        Some(crate::engine::mount_fs::MountFs::wrap_if_needed(store))
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
        context: &ExecutionContext,
        tool_call: ToolCall,
        tool_def: Option<&ToolDefinition>,
        trace_id: &str,
        act_span_id: &str,
        locale: Option<&str>,
        network_access: Option<&crate::engine::network_access::NetworkAccessList>,
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
        let event_context = EventContext::from_execution_context(context).with_span(
            trace_id.to_string(),
            tool_span_id.clone(),
            Some(act_span_id.to_string()),
        );

        // Track tool call timing for Braintrust observability
        let tool_start = Instant::now();
        let tool_call_fingerprint = tool_call_fingerprint(&tool_call);

        // Resolve display name from tool definition
        let display_name = crate::engine::localization::localized_tool_display_name(
            &tool_call.name,
            tool_def.and_then(|d| d.display_name()),
            locale,
        );
        let capability_attribution = tool_def.and_then(|def| {
            def.capability_attribution()
                .map(|(id, name)| (id.to_string(), name.map(str::to_string)))
        });

        // THREAT[TM-TOOL-009]: enforce the injected per-org outbound tool-call limit.
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
                                    crate::engine::message::ImageContentPart::from_base64(
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

        // Same context the client-side pre-tool gate uses, so a hook sees one
        // session whether this process runs the tool or the client does.
        // The guard fires when this future is dropped — a cancelled turn — and
        // on normal return. Work the tool leaves running can hold a clone and
        // die with the call; a dropped future is never polled again.
        let (tool_context, call_cancellation) = client_policy::tool_context_for_call(
            self,
            context,
            &event_context,
            &tool_call.id,
            network_access,
            &visible_tool_names,
        );
        let _cancel_on_call_end = call_cancellation.drop_guard();

        let execution_tool_call = self.transform_tool_call_for_execution(tool_call.clone());

        // Run pre-tool-use hooks (capability-contributed). They can mutate the
        // call, block it, or defer it; a blocked or deferred call is not
        // invoked and its result flows through the ordinary completion path.
        let (execution_tool_call, pre_hook_result) = act_hooks::pre_tool_use_outcome(
            &self.pre_tool_hooks,
            execution_tool_call,
            tool_def,
            &tool_context,
        )
        .await;

        let result = if let Some(pre_hook_result) = pre_hook_result {
            Ok(pre_hook_result)
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
            match AbortOnDropJoinHandle::new(everruns_contracts::rt::spawn(async move {
                executor.execute_with_context(&call, &def, &ctx).await
            }))
            .await
            {
                Ok(result) => result,
                Err(join_err) => Err(crate::engine::error::AgentLoopError::tool(format!(
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
                                crate::engine::message::ImageContentPart::from_base64(
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

#[path = "act_client_policy.rs"]
mod client_policy;

#[path = "act_nested_policy.rs"]
mod nested_policy;

#[cfg(test)]
#[path = "act_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "act_approval_tests.rs"]
mod approval_tests;

#[cfg(test)]
#[path = "act_client_policy_tests.rs"]
mod client_policy_tests;

//! Typed, in-process lifecycle hooks for Framework applications.
//!
//! Hooks are awaited extension points attached to an [`Agent`](crate::Agent),
//! not a second event subscription API. Applications register async closures
//! on [`AgentBuilder`](crate::AgentBuilder); each closure receives a small,
//! owned Framework context and may perform application work without importing
//! `everruns-core` or `everruns-host`.
//!
//! The contract is intentionally non-mutating: hooks cannot rewrite prompts,
//! tool arguments, tool results, or turn outcomes. A failing pre-effect hook
//! prevents the affected work; a failing post-effect hook cannot roll work
//! back, so the failure is isolated and surfaced through
//! [`Turn::hook_failures`](crate::Turn::hook_failures).

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use everruns_core::atoms::{PreToolUseDecision, PreToolUseHook};
use everruns_core::capabilities::Capability;
use everruns_core::{PostToolExecHook, tool_context::ToolContext};
use everruns_provider::tool_types::{ToolCall, ToolDefinition, ToolResult};
use serde_json::Value;

pub(crate) const LIFECYCLE_HOOK_CAPABILITY_ID: &str = "__everruns_framework_lifecycle_hooks";

/// The boxed future returned by one type-erased lifecycle handler.
type HookFuture = Pin<Box<dyn Future<Output = Result<(), String>> + Send>>;

/// One async lifecycle handler receiving an owned, typed context.
type Handler<T> = Arc<dyn Fn(T) -> HookFuture + Send + Sync>;

/// Convert a lifecycle handler's output into the Framework hook result.
///
/// Async closures may return `()` when they are infallible, or
/// `Result<(), E>` for any displayable error. This keeps simple hooks concise
/// while allowing fallible hooks to use their application's own error type.
/// Applications normally rely on those implementations without naming this
/// trait; an application-owned output type may implement it for custom result
/// conversion.
pub trait IntoHookResult {
    /// Convert this handler output into a string-backed hook result.
    fn into_hook_result(self) -> Result<(), String>;
}

impl IntoHookResult for () {
    fn into_hook_result(self) -> Result<(), String> {
        Ok(())
    }
}

impl<E: fmt::Display> IntoHookResult for Result<(), E> {
    fn into_hook_result(self) -> Result<(), String> {
        self.map_err(|error| error.to_string())
    }
}

fn handler<T, F, Fut, O>(callback: F) -> Handler<T>
where
    T: Send + 'static,
    F: Fn(T) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = O> + Send + 'static,
    O: IntoHookResult + 'static,
{
    Arc::new(move |context| {
        let future = callback(context);
        Box::pin(async move { future.await.into_hook_result() })
    })
}

/// A typed Framework lifecycle point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum HookPoint {
    /// Before the first turn attempted by one session.
    AgentStart,
    /// Before one turn is handed to the runtime.
    TurnStart,
    /// Before one model-requested tool call executes.
    ToolStart,
    /// After one tool call reaches a terminal result, including a blocked call.
    ToolEnd,
    /// After one non-cancelled turn reaches a terminal outcome.
    Completion,
}

impl fmt::Display for HookPoint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            HookPoint::AgentStart => "agent_start",
            HookPoint::TurnStart => "turn_start",
            HookPoint::ToolStart => "tool_start",
            HookPoint::ToolEnd => "tool_end",
            HookPoint::Completion => "completion",
        })
    }
}

/// A lifecycle handler failure surfaced by the Framework.
///
/// `handler_index` is zero-based within one lifecycle point and follows
/// builder registration order. Tool failures also carry the relevant call
/// identity. No prompt, credential, or unrelated tool payload is copied into
/// the failure.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct HookFailure {
    /// Lifecycle point whose handler failed.
    pub point: HookPoint,
    /// Zero-based registration index within `point`.
    pub handler_index: usize,
    /// Display text returned by the handler's error.
    ///
    /// Tool-start failures use a generic model-visible error; this detailed
    /// message remains application-facing.
    pub message: String,
    /// Tool name for tool lifecycle points.
    pub tool_name: Option<String>,
    /// Tool-call id for tool lifecycle points.
    pub tool_call_id: Option<String>,
}

impl HookFailure {
    fn new(point: HookPoint, handler_index: usize, message: String) -> Self {
        Self {
            point,
            handler_index,
            message,
            tool_name: None,
            tool_call_id: None,
        }
    }

    fn for_tool(
        point: HookPoint,
        handler_index: usize,
        message: String,
        tool_name: &str,
        tool_call_id: &str,
    ) -> Self {
        Self {
            point,
            handler_index,
            message,
            tool_name: Some(tool_name.to_string()),
            tool_call_id: Some(tool_call_id.to_string()),
        }
    }
}

impl fmt::Display for HookFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} hook #{} failed: {}",
            self.point, self.handler_index, self.message
        )
    }
}

impl std::error::Error for HookFailure {}

/// Context passed to an `on_agent_start` handler.
///
/// An agent starts independently in every [`Session`](crate::Session), just
/// before that session's first turn. If the chain fails, the next run retries
/// the whole chain; handlers with external effects should therefore be
/// idempotent.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct AgentStartContext {
    /// Human-readable agent name.
    pub agent_name: String,
    /// Typed identity of the session beginning execution.
    pub session_id: crate::SessionId,
}

/// Context passed to an `on_turn_start` handler.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct TurnStartContext {
    /// Human-readable agent name.
    pub agent_name: String,
    /// Typed identity of the session running the turn.
    pub session_id: crate::SessionId,
    /// Exact input that will be handed to the runtime after the hook chain.
    pub input: crate::InputMessage,
}

/// Context passed to an `on_tool_start` handler.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ToolStartContext {
    /// Typed identity of the session running the tool.
    pub session_id: crate::SessionId,
    /// Opaque parent turn id when available.
    pub turn_id: Option<String>,
    /// Opaque id of this tool call.
    pub tool_call_id: String,
    /// Model-facing tool name.
    pub tool_name: String,
    /// Tool arguments that will be passed to the tool implementation.
    pub arguments: Value,
}

/// Context passed to an `on_tool_end` handler.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ToolEndContext {
    /// Typed identity of the session running the tool.
    pub session_id: crate::SessionId,
    /// Opaque parent turn id when available.
    pub turn_id: Option<String>,
    /// Opaque id of this tool call.
    pub tool_call_id: String,
    /// Model-facing tool name.
    pub tool_name: String,
    /// Tool arguments selected for execution. A blocked call carries the
    /// arguments it would have executed.
    pub arguments: Value,
    /// Structured success value, when the tool produced one.
    pub result: Option<Value>,
    /// Tool error text, when execution failed.
    pub error: Option<String>,
}

impl ToolEndContext {
    /// Whether the tool returned without an error.
    pub fn success(&self) -> bool {
        self.error.is_none()
    }
}

/// Context passed to an `on_completion` handler.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct CompletionContext {
    /// Human-readable agent name.
    pub agent_name: String,
    /// Typed identity of the session whose turn completed.
    pub session_id: crate::SessionId,
    /// Stable Framework projection of the terminal turn.
    pub turn: crate::Turn,
}

/// Ordered callback lists attached to a built agent.
#[derive(Clone, Default)]
pub(crate) struct LifecycleHooks {
    agent_start: Vec<Handler<AgentStartContext>>,
    turn_start: Vec<Handler<TurnStartContext>>,
    tool_start: Vec<Handler<ToolStartContext>>,
    tool_end: Vec<Handler<ToolEndContext>>,
    completion: Vec<Handler<CompletionContext>>,
}

impl fmt::Debug for LifecycleHooks {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LifecycleHooks")
            .field("agent_start", &self.agent_start.len())
            .field("turn_start", &self.turn_start.len())
            .field("tool_start", &self.tool_start.len())
            .field("tool_end", &self.tool_end.len())
            .field("completion", &self.completion.len())
            .finish()
    }
}

impl LifecycleHooks {
    pub(crate) fn on_agent_start<F, Fut, O>(&mut self, callback: F)
    where
        F: Fn(AgentStartContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = O> + Send + 'static,
        O: IntoHookResult + 'static,
    {
        self.agent_start.push(handler(callback));
    }

    pub(crate) fn on_turn_start<F, Fut, O>(&mut self, callback: F)
    where
        F: Fn(TurnStartContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = O> + Send + 'static,
        O: IntoHookResult + 'static,
    {
        self.turn_start.push(handler(callback));
    }

    pub(crate) fn on_tool_start<F, Fut, O>(&mut self, callback: F)
    where
        F: Fn(ToolStartContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = O> + Send + 'static,
        O: IntoHookResult + 'static,
    {
        self.tool_start.push(handler(callback));
    }

    pub(crate) fn on_tool_end<F, Fut, O>(&mut self, callback: F)
    where
        F: Fn(ToolEndContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = O> + Send + 'static,
        O: IntoHookResult + 'static,
    {
        self.tool_end.push(handler(callback));
    }

    pub(crate) fn on_completion<F, Fut, O>(&mut self, callback: F)
    where
        F: Fn(CompletionContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = O> + Send + 'static,
        O: IntoHookResult + 'static,
    {
        self.completion.push(handler(callback));
    }

    pub(crate) fn has_tool_hooks(&self) -> bool {
        !self.tool_start.is_empty() || !self.tool_end.is_empty()
    }

    pub(crate) async fn run_agent_start(
        &self,
        context: AgentStartContext,
    ) -> Result<(), HookFailure> {
        run_pre_effect(&self.agent_start, context, HookPoint::AgentStart).await
    }

    pub(crate) async fn run_turn_start(
        &self,
        context: TurnStartContext,
    ) -> Result<(), HookFailure> {
        run_pre_effect(&self.turn_start, context, HookPoint::TurnStart).await
    }

    pub(crate) async fn run_completion(&self, context: CompletionContext) -> Vec<HookFailure> {
        let mut failures = Vec::new();
        for (handler_index, handler) in self.completion.iter().enumerate() {
            if let Err(message) = handler(context.clone()).await {
                failures.push(HookFailure::new(
                    HookPoint::Completion,
                    handler_index,
                    message,
                ));
            }
        }
        failures
    }
}

async fn run_pre_effect<T: Clone>(
    handlers: &[Handler<T>],
    context: T,
    point: HookPoint,
) -> Result<(), HookFailure> {
    for (handler_index, handler) in handlers.iter().enumerate() {
        if let Err(message) = handler(context.clone()).await {
            return Err(HookFailure::new(point, handler_index, message));
        }
    }
    Ok(())
}

/// Per-session state shared with the runtime's tool-hook adapters.
pub(crate) struct HookRunState {
    hooks: LifecycleHooks,
    failures: Mutex<Vec<HookFailure>>,
}

impl HookRunState {
    pub(crate) fn new(hooks: LifecycleHooks) -> Arc<Self> {
        Arc::new(Self {
            hooks,
            failures: Mutex::new(Vec::new()),
        })
    }

    pub(crate) fn hooks(&self) -> &LifecycleHooks {
        &self.hooks
    }

    pub(crate) fn begin_turn(&self) {
        self.failures
            .lock()
            .expect("hook failures mutex poisoned")
            .clear();
    }

    pub(crate) fn take_failures(&self) -> Vec<HookFailure> {
        std::mem::take(&mut *self.failures.lock().expect("hook failures mutex poisoned"))
    }

    fn record(&self, failure: HookFailure) {
        self.failures
            .lock()
            .expect("hook failures mutex poisoned")
            .push(failure);
    }

    pub(crate) fn capability(self: &Arc<Self>) -> Option<LifecycleHookCapability> {
        self.hooks
            .has_tool_hooks()
            .then(|| LifecycleHookCapability {
                state: self.clone(),
            })
    }
}

/// Private bridge from Framework closures to the runtime's in-process tool
/// hook seams. The application never constructs a capability or persisted hook
/// record.
pub(crate) struct LifecycleHookCapability {
    state: Arc<HookRunState>,
}

impl Capability for LifecycleHookCapability {
    fn id(&self) -> &str {
        LIFECYCLE_HOOK_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Framework lifecycle hooks"
    }

    fn description(&self) -> &str {
        "Runs typed in-process lifecycle handlers registered by the application."
    }

    fn pre_tool_use_hooks(&self) -> Vec<Arc<dyn PreToolUseHook>> {
        if self.state.hooks.tool_start.is_empty() {
            Vec::new()
        } else {
            vec![Arc::new(ToolStartHook {
                state: self.state.clone(),
            })]
        }
    }

    fn post_tool_exec_hooks(&self) -> Vec<Arc<dyn PostToolExecHook>> {
        if self.state.hooks.tool_end.is_empty() {
            Vec::new()
        } else {
            vec![Arc::new(ToolEndHook {
                state: self.state.clone(),
            })]
        }
    }
}

struct ToolStartHook {
    state: Arc<HookRunState>,
}

#[async_trait]
impl PreToolUseHook for ToolStartHook {
    async fn before_exec(
        &self,
        tool_call: ToolCall,
        _tool_def: &ToolDefinition,
        context: &ToolContext,
    ) -> PreToolUseDecision {
        let hook_context = ToolStartContext {
            session_id: context.session_id,
            turn_id: context
                .event_context
                .as_ref()
                .and_then(|event| event.turn_id)
                .map(|turn_id| turn_id.to_string()),
            tool_call_id: tool_call.id.clone(),
            tool_name: tool_call.name.clone(),
            arguments: tool_call.execution_arguments(),
        };
        for (handler_index, handler) in self.state.hooks.tool_start.iter().enumerate() {
            if let Err(message) = handler(hook_context.clone()).await {
                let failure = HookFailure::for_tool(
                    HookPoint::ToolStart,
                    handler_index,
                    message,
                    &tool_call.name,
                    &tool_call.id,
                );
                // The runtime turns this reason into model-visible tool
                // output. Keep application error details on `HookFailure` so
                // credentials or backend diagnostics are not sent to the LLM.
                let reason = format!(
                    "tool call blocked by {} hook #{}",
                    failure.point, failure.handler_index
                );
                self.state.record(failure);
                return PreToolUseDecision::Block {
                    tool_call,
                    reason,
                    user_message: None,
                };
            }
        }
        PreToolUseDecision::Continue(tool_call)
    }
}

struct ToolEndHook {
    state: Arc<HookRunState>,
}

#[async_trait]
impl PostToolExecHook for ToolEndHook {
    async fn after_exec(
        &self,
        tool_call: &ToolCall,
        _tool_def: &ToolDefinition,
        result: &mut ToolResult,
        context: &ToolContext,
    ) {
        let hook_context = ToolEndContext {
            session_id: context.session_id,
            turn_id: context
                .event_context
                .as_ref()
                .and_then(|event| event.turn_id)
                .map(|turn_id| turn_id.to_string()),
            tool_call_id: tool_call.id.clone(),
            tool_name: tool_call.name.clone(),
            arguments: tool_call.execution_arguments(),
            result: result.result.clone(),
            error: result.error.clone(),
        };
        for (handler_index, handler) in self.state.hooks.tool_end.iter().enumerate() {
            if let Err(message) = handler(hook_context.clone()).await {
                self.state.record(HookFailure::for_tool(
                    HookPoint::ToolEnd,
                    handler_index,
                    message,
                    &tool_call.name,
                    &tool_call.id,
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[tokio::test]
    async fn pre_effect_hooks_run_in_order_and_stop_on_error() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mut hooks = LifecycleHooks::default();
        for (label, fails) in [("first", false), ("second", true), ("third", false)] {
            let seen = seen.clone();
            hooks.on_agent_start(move |_context| {
                let seen = seen.clone();
                async move {
                    seen.lock().unwrap().push(label);
                    if fails { Err("broken") } else { Ok(()) }
                }
            });
        }

        let failure = hooks
            .run_agent_start(AgentStartContext {
                agent_name: "agent".into(),
                session_id: crate::SessionId::new(),
            })
            .await
            .expect_err("second hook fails");

        assert_eq!(*seen.lock().unwrap(), ["first", "second"]);
        assert_eq!(failure.point, HookPoint::AgentStart);
        assert_eq!(failure.handler_index, 1);
        assert_eq!(failure.message, "broken");
    }

    #[tokio::test]
    async fn post_effect_hooks_isolate_errors_and_continue() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut hooks = LifecycleHooks::default();
        hooks.on_completion(|_context| async move { Err::<(), _>("broken") });
        let later = calls.clone();
        hooks.on_completion(move |_context| {
            let later = later.clone();
            async move {
                later.fetch_add(1, Ordering::SeqCst);
            }
        });

        let failures = hooks
            .run_completion(CompletionContext {
                agent_name: "agent".into(),
                session_id: crate::SessionId::new(),
                turn: crate::Turn::cancelled(everruns_provider::typed_id::TurnId::new()),
            })
            .await;

        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].point, HookPoint::Completion);
        assert_eq!(failures[0].handler_index, 0);
    }

    #[test]
    fn empty_hooks_do_not_register_a_runtime_capability() {
        let state = HookRunState::new(LifecycleHooks::default());
        assert!(state.capability().is_none());
    }
}

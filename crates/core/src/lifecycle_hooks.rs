// User-defined lifecycle hooks: session and turn events.
//
// Two trait families bridge the four non-tool `HookEvent`s to runtime firing
// points (see `knowledge/runtime-resources/user-hooks.md`):
//
// - `SessionLifecycleHook` — `session_start` / `session_end`. Advisory only;
//   a hook failure is logged and never aborts the session.
// - `TurnLifecycleHook` — `user_prompt_submit` / `turn_end`. `turn_end` is
//   advisory; `user_prompt_submit` can *block* (reject the inbound message and
//   abort the turn) and *mutate* (rewrite the user message text).
//
// As with the tool hooks, capability authors hand the runtime *data*
// (`UserHookSpec`); this module owns the spec -> `Arc<dyn …Hook>` translation
// so the central timeout/output/sandbox limits stay enforced. The adapters
// reuse the same `BashHookExecutor` + `BashHookDispatcher` as the pre/post
// tool adapters, so payload delivery, output parsing, and `on_error` policy
// are identical across every event.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use crate::hook_executor::{BashHookExecutor, ExecutorOpts, HookExecutor, HookPayload};
use crate::typed_id::{OrgId, SessionId, TurnId};
use crate::user_hook_types::{ExecutorSpec, HookEvent, HookId, HookOutcome, OnError, UserHookSpec};

// ============================================================================
// Context structs
// ============================================================================

/// Context available when a session-lifecycle hook fires. Lighter than
/// `ToolContext`: at session create/close there is no tool, no turn, and no
/// per-tool stores — only the session identity and (optionally) the agent.
#[derive(Debug, Clone)]
pub struct SessionHookContext {
    pub session_id: SessionId,
    pub org_id: Option<OrgId>,
    pub agent_id: Option<String>,
}

/// Context available when a turn-lifecycle hook fires.
#[derive(Debug, Clone)]
pub struct TurnHookContext {
    pub session_id: SessionId,
    pub turn_id: Option<TurnId>,
    pub org_id: Option<OrgId>,
    pub agent_id: Option<String>,
}

// ============================================================================
// Decisions
// ============================================================================

/// Outcome of a `user_prompt_submit` chain. Mirrors `PreToolUseDecision` but
/// over the user message text rather than a `ToolCall`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserPromptDecision {
    /// Continue the turn with the (possibly rewritten) user message text.
    Continue { message: String },
    /// Reject the inbound message and abort the turn. `reason` is logged /
    /// audited; `user_message` (if any) is surfaced to the caller.
    Block {
        reason: String,
        user_message: Option<String>,
    },
}

// ============================================================================
// SessionLifecycleHook
// ============================================================================

/// Hook that fires on `session_start` or `session_end`. Advisory only — the
/// runtime calls `fire` and logs any failure; it never blocks the session.
#[async_trait]
pub trait SessionLifecycleHook: Send + Sync {
    /// Which event this hook is bound to (`SessionStart` or `SessionEnd`).
    fn event(&self) -> HookEvent;
    /// Stable id for logs.
    fn hook_id(&self) -> &HookId;
    /// Run the hook. The outcome is advisory; `Block`/`Mutate` are ignored
    /// (logged) since these events cannot block or mutate anything.
    async fn fire(&self, ctx: &SessionHookContext, data: serde_json::Value);
}

// ============================================================================
// TurnLifecycleHook
// ============================================================================

/// Hook that fires on `user_prompt_submit` (blockable/mutable) or `turn_end`
/// (advisory).
#[async_trait]
pub trait TurnLifecycleHook: Send + Sync {
    fn event(&self) -> HookEvent;
    fn hook_id(&self) -> &HookId;
    /// The spec's `on_error` policy, used by the `user_prompt_submit` chain
    /// runner to decide block-vs-continue on executor failure.
    fn on_error(&self) -> OnError;
    /// Run the hook against a payload. Returns the raw `HookOutcome`; chain
    /// runners interpret it per-event (block/mutate for `user_prompt_submit`,
    /// advisory for `turn_end`).
    async fn run(&self, ctx: &TurnHookContext, data: serde_json::Value) -> HookOutcome;
}

// ============================================================================
// Bash-backed adapters
// ============================================================================

/// Single adapter that implements both lifecycle traits over a bash executor.
/// The `event` discriminates which trait method the runtime drives; the
/// executor/limits/`on_error` handling is shared.
struct BashLifecycleHook {
    spec: UserHookSpec,
    executor: Arc<dyn HookExecutor>,
    opts: ExecutorOpts,
    hook_id: HookId,
}

impl BashLifecycleHook {
    fn new(spec: UserHookSpec, executor: Arc<dyn HookExecutor>, index: usize) -> Self {
        let opts = ExecutorOpts {
            timeout_ms: spec.timeout_ms,
            max_output_bytes: 64 * 1024,
        };
        let hook_id = spec.resolve_id(index);
        Self {
            spec,
            executor,
            opts,
            hook_id,
        }
    }

    fn payload(&self, event: HookEvent, data: serde_json::Value) -> HookPayload {
        HookPayload {
            event,
            hook_id: self.hook_id.clone(),
            // session_id/org_id/turn_id are filled by the chain runner from
            // the live context before dispatch; the nil default is overwritten
            // there. (Lifecycle contexts vary by event, so the runner owns it.)
            session_id: SessionId::from_uuid(uuid::Uuid::nil()),
            turn_id: None,
            org_id: None,
            agent_id: None,
            ts: chrono::Utc::now().to_rfc3339(),
            data,
        }
    }
}

#[async_trait]
impl SessionLifecycleHook for BashLifecycleHook {
    fn event(&self) -> HookEvent {
        self.spec.event
    }
    fn hook_id(&self) -> &HookId {
        &self.hook_id
    }
    async fn fire(&self, ctx: &SessionHookContext, data: serde_json::Value) {
        let mut payload = self.payload(self.spec.event, data);
        payload.session_id = ctx.session_id;
        payload.org_id = ctx.org_id;
        payload.agent_id = ctx.agent_id.clone();
        let outcome = self.executor.run(payload, &self.opts).await;
        match outcome {
            HookOutcome::Allow => {}
            HookOutcome::Mutate { .. } | HookOutcome::Block { .. } => {
                tracing::warn!(
                    hook_id = %self.hook_id.as_str(),
                    event = %self.spec.event.as_str(),
                    "lifecycle hook returned block/mutate on an advisory event; ignoring"
                );
            }
            HookOutcome::Error { message } => {
                // Advisory: log per on_error but never abort the session.
                tracing::warn!(
                    hook_id = %self.hook_id.as_str(),
                    event = %self.spec.event.as_str(),
                    on_error = ?self.spec.on_error,
                    message = %message,
                    "lifecycle hook errored (advisory event, continuing)"
                );
            }
        }
    }
}

#[async_trait]
impl TurnLifecycleHook for BashLifecycleHook {
    fn event(&self) -> HookEvent {
        self.spec.event
    }
    fn hook_id(&self) -> &HookId {
        &self.hook_id
    }
    fn on_error(&self) -> OnError {
        self.spec.on_error
    }
    async fn run(&self, ctx: &TurnHookContext, data: serde_json::Value) -> HookOutcome {
        let mut payload = self.payload(self.spec.event, data);
        payload.session_id = ctx.session_id;
        payload.turn_id = ctx.turn_id.map(|t| t.to_string());
        payload.org_id = ctx.org_id;
        payload.agent_id = ctx.agent_id.clone();
        self.executor.run(payload, &self.opts).await
    }
}

// ============================================================================
// Builders
// ============================================================================

fn build_bash_executor(
    spec: &UserHookSpec,
    dispatcher: Arc<dyn crate::hook_executor::BashHookDispatcher>,
) -> Arc<dyn HookExecutor> {
    match &spec.executor {
        ExecutorSpec::Bash { command, env } => Arc::new(BashHookExecutor::with_dispatcher(
            command.clone(),
            env.clone(),
            dispatcher,
        )),
    }
}

/// Build `SessionLifecycleHook` adapters for every spec whose event is
/// `event` (must be `SessionStart` or `SessionEnd`). Invalid specs are dropped
/// with a warning so one bad entry can't take down the chain.
pub fn build_session_lifecycle_hooks(
    specs: &[UserHookSpec],
    event: HookEvent,
    dispatcher: Arc<dyn crate::hook_executor::BashHookDispatcher>,
) -> Vec<Arc<dyn SessionLifecycleHook>> {
    debug_assert!(matches!(
        event,
        HookEvent::SessionStart | HookEvent::SessionEnd
    ));
    let mut out: Vec<Arc<dyn SessionLifecycleHook>> = Vec::new();
    for (index, spec) in specs.iter().enumerate() {
        if spec.event != event {
            continue;
        }
        if let Err(e) = spec.validate() {
            tracing::warn!(
                hook_id = %spec.resolve_id(index).as_str(),
                error = %e,
                "skipping invalid lifecycle hook spec"
            );
            continue;
        }
        let executor = build_bash_executor(spec, dispatcher.clone());
        out.push(Arc::new(BashLifecycleHook::new(
            spec.clone(),
            executor,
            index,
        )));
    }
    out
}

/// Build `TurnLifecycleHook` adapters for every spec whose event is `event`
/// (must be `UserPromptSubmit` or `TurnEnd`).
pub fn build_turn_lifecycle_hooks(
    specs: &[UserHookSpec],
    event: HookEvent,
    dispatcher: Arc<dyn crate::hook_executor::BashHookDispatcher>,
) -> Vec<Arc<dyn TurnLifecycleHook>> {
    debug_assert!(matches!(
        event,
        HookEvent::UserPromptSubmit | HookEvent::TurnEnd
    ));
    let mut out: Vec<Arc<dyn TurnLifecycleHook>> = Vec::new();
    for (index, spec) in specs.iter().enumerate() {
        if spec.event != event {
            continue;
        }
        if let Err(e) = spec.validate() {
            tracing::warn!(
                hook_id = %spec.resolve_id(index).as_str(),
                error = %e,
                "skipping invalid lifecycle hook spec"
            );
            continue;
        }
        let executor = build_bash_executor(spec, dispatcher.clone());
        out.push(Arc::new(BashLifecycleHook::new(
            spec.clone(),
            executor,
            index,
        )));
    }
    out
}

// ============================================================================
// Chain runners
// ============================================================================

/// Run every session-lifecycle hook in order. Advisory: failures are handled
/// inside each `fire` call; this never returns an error.
pub async fn run_session_lifecycle_hooks(
    hooks: &[Arc<dyn SessionLifecycleHook>],
    ctx: &SessionHookContext,
    data: serde_json::Value,
) {
    for hook in hooks {
        hook.fire(ctx, data.clone()).await;
    }
}

/// Run the `turn_end` chain. Advisory: each hook runs independently; block /
/// mutate outcomes are logged and ignored, errors are logged per `on_error`.
pub async fn run_turn_end_hooks(
    hooks: &[Arc<dyn TurnLifecycleHook>],
    ctx: &TurnHookContext,
    data: serde_json::Value,
) {
    for hook in hooks {
        match hook.run(ctx, data.clone()).await {
            HookOutcome::Allow => {}
            HookOutcome::Mutate { .. } | HookOutcome::Block { .. } => {
                tracing::warn!(
                    hook_id = %hook.hook_id().as_str(),
                    "turn_end hook returned block/mutate on an advisory event; ignoring"
                );
            }
            HookOutcome::Error { message } => {
                tracing::warn!(
                    hook_id = %hook.hook_id().as_str(),
                    message = %message,
                    "turn_end hook errored (advisory, continuing)"
                );
            }
        }
    }
}

/// Run the `user_prompt_submit` chain sequentially over the message text.
/// Each hook sees the previous hook's mutated text. The first `Block` wins and
/// aborts the chain; on executor error the spec's `on_error` policy applies
/// (`Block` -> block, `Warn`/`Allow` -> continue with current text).
pub async fn run_user_prompt_submit_hooks(
    hooks: &[Arc<dyn TurnLifecycleHook>],
    ctx: &TurnHookContext,
    mut message: String,
) -> UserPromptDecision {
    for hook in hooks {
        let data = json!({ "message": message });
        match hook.run(ctx, data).await {
            HookOutcome::Allow => {}
            HookOutcome::Mutate { patch, .. } => {
                if let Some(new_msg) = patch.get("message").and_then(|v| v.as_str()) {
                    message = new_msg.to_string();
                }
            }
            HookOutcome::Block {
                reason,
                user_message,
            } => {
                return UserPromptDecision::Block {
                    reason,
                    user_message,
                };
            }
            HookOutcome::Error { message: err } => match hook.on_error() {
                OnError::Block => {
                    return UserPromptDecision::Block {
                        reason: format!("hook {} errored: {err}", hook.hook_id().as_str()),
                        user_message: None,
                    };
                }
                OnError::Warn => {
                    tracing::warn!(
                        hook_id = %hook.hook_id().as_str(),
                        message = %err,
                        "user_prompt_submit hook errored"
                    );
                }
                OnError::Allow => {}
            },
        }
    }
    UserPromptDecision::Continue { message }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hook_executor::{BashExecOutput, BashHookDispatcher};
    use crate::user_hook_types::{HookMatcher, HookSource};
    use async_trait::async_trait;
    use std::collections::BTreeMap;

    /// Dispatcher that returns a canned stdout/exit so we exercise the full
    /// parse + decision path without a real sandbox.
    struct CannedDispatcher {
        stdout: String,
        exit_code: i32,
        payloads: std::sync::Mutex<Vec<HookPayload>>,
    }

    #[async_trait]
    impl BashHookDispatcher for CannedDispatcher {
        async fn dispatch(
            &self,
            payload: &HookPayload,
            _command: &str,
            _extra_env: &BTreeMap<String, String>,
            _opts: &ExecutorOpts,
        ) -> Result<BashExecOutput, String> {
            self.payloads.lock().unwrap().push(payload.clone());
            Ok(BashExecOutput {
                exit_code: self.exit_code,
                stdout: self.stdout.clone(),
                stderr: String::new(),
            })
        }
    }

    fn spec(event: HookEvent, on_error: OnError) -> UserHookSpec {
        UserHookSpec {
            id: Some("t".into()),
            event,
            matcher: HookMatcher::default(),
            executor: ExecutorSpec::Bash {
                command: "true".into(),
                env: Default::default(),
            },
            timeout_ms: 5000,
            on_error,
            description: None,
            source: HookSource::UserConfig,
        }
    }

    fn dispatcher(stdout: &str, exit: i32) -> Arc<CannedDispatcher> {
        Arc::new(CannedDispatcher {
            stdout: stdout.into(),
            exit_code: exit,
            payloads: Default::default(),
        })
    }

    fn turn_ctx() -> TurnHookContext {
        TurnHookContext {
            session_id: SessionId::new(),
            turn_id: Some(TurnId::new()),
            org_id: None,
            agent_id: None,
        }
    }

    #[tokio::test]
    async fn user_prompt_submit_allow_passes_message_through() {
        let hooks = build_turn_lifecycle_hooks(
            &[spec(HookEvent::UserPromptSubmit, OnError::Warn)],
            HookEvent::UserPromptSubmit,
            dispatcher("", 0),
        );
        let decision = run_user_prompt_submit_hooks(&hooks, &turn_ctx(), "hello".into()).await;
        assert_eq!(
            decision,
            UserPromptDecision::Continue {
                message: "hello".into()
            }
        );
    }

    #[tokio::test]
    async fn user_prompt_submit_block_via_json() {
        let hooks = build_turn_lifecycle_hooks(
            &[spec(HookEvent::UserPromptSubmit, OnError::Warn)],
            HookEvent::UserPromptSubmit,
            dispatcher(
                r#"{"decision":"block","reason":"nope","user_message":"blocked"}"#,
                0,
            ),
        );
        let decision = run_user_prompt_submit_hooks(&hooks, &turn_ctx(), "hello".into()).await;
        match decision {
            UserPromptDecision::Block {
                reason,
                user_message,
            } => {
                assert_eq!(reason, "nope");
                assert_eq!(user_message.as_deref(), Some("blocked"));
            }
            other => panic!("expected Block, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn user_prompt_submit_block_via_nonzero_exit() {
        // Empty stdout + non-zero exit = block (git-hook fallback).
        let hooks = build_turn_lifecycle_hooks(
            &[spec(HookEvent::UserPromptSubmit, OnError::Warn)],
            HookEvent::UserPromptSubmit,
            dispatcher("", 1),
        );
        let decision = run_user_prompt_submit_hooks(&hooks, &turn_ctx(), "hello".into()).await;
        assert!(matches!(decision, UserPromptDecision::Block { .. }));
    }

    #[tokio::test]
    async fn user_prompt_submit_mutate_rewrites_message() {
        let hooks = build_turn_lifecycle_hooks(
            &[spec(HookEvent::UserPromptSubmit, OnError::Warn)],
            HookEvent::UserPromptSubmit,
            dispatcher(
                r#"{"decision":"mutate","patch":{"message":"rewritten"}}"#,
                0,
            ),
        );
        let decision = run_user_prompt_submit_hooks(&hooks, &turn_ctx(), "original".into()).await;
        assert_eq!(
            decision,
            UserPromptDecision::Continue {
                message: "rewritten".into()
            }
        );
    }

    #[tokio::test]
    async fn user_prompt_submit_error_with_on_error_block_blocks() {
        // Non-JSON stdout => executor Error; on_error=Block => Block.
        let hooks = build_turn_lifecycle_hooks(
            &[spec(HookEvent::UserPromptSubmit, OnError::Block)],
            HookEvent::UserPromptSubmit,
            dispatcher("not json at all", 0),
        );
        let decision = run_user_prompt_submit_hooks(&hooks, &turn_ctx(), "hello".into()).await;
        assert!(matches!(decision, UserPromptDecision::Block { .. }));
    }

    #[tokio::test]
    async fn user_prompt_submit_error_with_on_error_warn_continues() {
        let hooks = build_turn_lifecycle_hooks(
            &[spec(HookEvent::UserPromptSubmit, OnError::Warn)],
            HookEvent::UserPromptSubmit,
            dispatcher("not json", 0),
        );
        let decision = run_user_prompt_submit_hooks(&hooks, &turn_ctx(), "hello".into()).await;
        assert_eq!(
            decision,
            UserPromptDecision::Continue {
                message: "hello".into()
            }
        );
    }

    #[tokio::test]
    async fn user_prompt_submit_chain_threads_mutations_then_blocks() {
        // First hook rewrites, second hook blocks. Order preserved.
        let specs = [
            {
                let mut s = spec(HookEvent::UserPromptSubmit, OnError::Warn);
                s.id = Some("rewriter".into());
                s
            },
            {
                let mut s = spec(HookEvent::UserPromptSubmit, OnError::Warn);
                s.id = Some("blocker".into());
                s
            },
        ];
        // Separate dispatchers let the second hook observe the first hook's mutation.
        let rewriter = build_turn_lifecycle_hooks(
            &specs[..1],
            HookEvent::UserPromptSubmit,
            dispatcher(r#"{"decision":"mutate","patch":{"message":"step1"}}"#, 0),
        );
        let block_dispatcher = dispatcher(r#"{"decision":"block","reason":"stop"}"#, 0);
        let blocker = build_turn_lifecycle_hooks(
            &specs[1..],
            HookEvent::UserPromptSubmit,
            block_dispatcher.clone(),
        );
        let mut chain = rewriter;
        chain.extend(blocker);
        let decision = run_user_prompt_submit_hooks(&chain, &turn_ctx(), "orig".into()).await;
        assert!(matches!(decision, UserPromptDecision::Block { .. }));
        let payloads = block_dispatcher.payloads.lock().unwrap();
        assert_eq!(payloads.len(), 1);
        assert_eq!(payloads[0].data["message"], "step1");
    }

    #[tokio::test]
    async fn turn_end_runs_advisory_and_ignores_block() {
        let dispatch = dispatcher(r#"{"decision":"block","reason":"ignored"}"#, 0);
        let hooks = build_turn_lifecycle_hooks(
            &[
                spec(HookEvent::TurnEnd, OnError::Warn),
                spec(HookEvent::TurnEnd, OnError::Warn),
            ],
            HookEvent::TurnEnd,
            // Even a "block" decision must not abort anything here.
            dispatch.clone(),
        );
        run_turn_end_hooks(&hooks, &turn_ctx(), json!({"success": true})).await;
        let payloads = dispatch.payloads.lock().unwrap();
        assert_eq!(payloads.len(), 2);
        for payload in payloads.iter() {
            assert_eq!(payload.event, HookEvent::TurnEnd);
            assert_eq!(payload.data, json!({"success": true}));
        }
    }

    #[tokio::test]
    async fn session_lifecycle_runs_advisory() {
        let dispatch = dispatcher("", 0);
        let hooks = build_session_lifecycle_hooks(
            &[spec(HookEvent::SessionStart, OnError::Warn)],
            HookEvent::SessionStart,
            dispatch.clone(),
        );
        let ctx = SessionHookContext {
            session_id: SessionId::new(),
            org_id: None,
            agent_id: Some("agt_x".into()),
        };
        run_session_lifecycle_hooks(&hooks, &ctx, json!({"agent_id": "agt_x"})).await;
        let payloads = dispatch.payloads.lock().unwrap();
        assert_eq!(payloads.len(), 1);
        assert_eq!(payloads[0].session_id, ctx.session_id);
        assert_eq!(payloads[0].event, HookEvent::SessionStart);
        assert_eq!(payloads[0].data, json!({"agent_id": "agt_x"}));
    }

    #[tokio::test]
    async fn builders_filter_by_event() {
        let specs = vec![
            spec(HookEvent::SessionStart, OnError::Warn),
            spec(HookEvent::SessionEnd, OnError::Warn),
            spec(HookEvent::TurnEnd, OnError::Warn),
            spec(HookEvent::UserPromptSubmit, OnError::Warn),
        ];
        let d = dispatcher("", 0);
        assert_eq!(
            build_session_lifecycle_hooks(&specs, HookEvent::SessionStart, d.clone()).len(),
            1
        );
        assert_eq!(
            build_session_lifecycle_hooks(&specs, HookEvent::SessionEnd, d.clone()).len(),
            1
        );
        assert_eq!(
            build_turn_lifecycle_hooks(&specs, HookEvent::TurnEnd, d.clone()).len(),
            1
        );
        assert_eq!(
            build_turn_lifecycle_hooks(&specs, HookEvent::UserPromptSubmit, d).len(),
            1
        );
    }
}

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
        settings: std::sync::Mutex<Vec<DispatchSettings>>,
    }

    type DispatchSettings = (String, BTreeMap<String, String>, u32, usize);

    #[async_trait]
    impl BashHookDispatcher for CannedDispatcher {
        async fn dispatch(
            &self,
            payload: &HookPayload,
            command: &str,
            extra_env: &BTreeMap<String, String>,
            opts: &ExecutorOpts,
        ) -> Result<BashExecOutput, String> {
            self.payloads.lock().unwrap().push(payload.clone());
            self.settings.lock().unwrap().push((
                command.into(),
                extra_env.clone(),
                opts.timeout_ms,
                opts.max_output_bytes,
            ));
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
            settings: Default::default(),
        })
    }

    fn turn_ctx() -> TurnHookContext {
        TurnHookContext {
            session_id: SessionId::from_seed(1),
            turn_id: Some(TurnId::from_seed(2)),
            org_id: Some(OrgId::from_seed(3)),
            agent_id: Some("agent_4".into()),
        }
    }

    #[tokio::test]
    async fn prompt_decisions_apply_mutations_blocks_and_all_error_policies() {
        assert_eq!(
            run_user_prompt_submit_hooks(&[], &turn_ctx(), "original".into()).await,
            UserPromptDecision::Continue {
                message: "original".into()
            }
        );
        for (stdout, exit, policy, expected) in [
            (
                "",
                0,
                OnError::Warn,
                UserPromptDecision::Continue {
                    message: "original".into(),
                },
            ),
            (
                r#"{"decision":"mutate","patch":{"message":"rewritten α"}}"#,
                0,
                OnError::Warn,
                UserPromptDecision::Continue {
                    message: "rewritten α".into(),
                },
            ),
            (
                r#"{"decision":"mutate","patch":{"message":42}}"#,
                0,
                OnError::Warn,
                UserPromptDecision::Continue {
                    message: "original".into(),
                },
            ),
            (
                r#"{"decision":"mutate","patch":{"unrelated":true}}"#,
                0,
                OnError::Warn,
                UserPromptDecision::Continue {
                    message: "original".into(),
                },
            ),
            (
                r#"{"decision":"block","reason":"nope","user_message":"blocked"}"#,
                0,
                OnError::Allow,
                UserPromptDecision::Block {
                    reason: "nope".into(),
                    user_message: Some("blocked".into()),
                },
            ),
            (
                "",
                1,
                OnError::Warn,
                UserPromptDecision::Block {
                    reason: "hook exited non-zero".into(),
                    user_message: None,
                },
            ),
            (
                "bad",
                0,
                OnError::Block,
                UserPromptDecision::Block {
                    reason: "hook user:t errored: hook stdout is not JSON (first 80 bytes: bad)"
                        .into(),
                    user_message: None,
                },
            ),
            (
                "bad",
                0,
                OnError::Warn,
                UserPromptDecision::Continue {
                    message: "original".into(),
                },
            ),
            (
                "bad",
                0,
                OnError::Allow,
                UserPromptDecision::Continue {
                    message: "original".into(),
                },
            ),
        ] {
            let hooks = build_turn_lifecycle_hooks(
                &[spec(HookEvent::UserPromptSubmit, policy)],
                HookEvent::UserPromptSubmit,
                dispatcher(stdout, exit),
            );
            assert_eq!(
                run_user_prompt_submit_hooks(&hooks, &turn_ctx(), "original".into()).await,
                expected,
                "{stdout}/{policy:?}"
            );
        }
    }

    struct MustNotRun;
    #[async_trait]
    impl BashHookDispatcher for MustNotRun {
        async fn dispatch(
            &self,
            _: &HookPayload,
            _: &str,
            _: &BTreeMap<String, String>,
            _: &ExecutorOpts,
        ) -> Result<BashExecOutput, String> {
            panic!("hook after the first block must not run")
        }
    }

    #[tokio::test]
    async fn prompt_chain_threads_mutations_and_stops_at_explicit_or_error_block() {
        for (stdout, policy, expected) in [
            (
                r#"{"decision":"block","reason":"stop","user_message":"explain"}"#,
                OnError::Warn,
                UserPromptDecision::Block {
                    reason: "stop".into(),
                    user_message: Some("explain".into()),
                },
            ),
            (
                "bad",
                OnError::Block,
                UserPromptDecision::Block {
                    reason: "hook user:t errored: hook stdout is not JSON (first 80 bytes: bad)"
                        .into(),
                    user_message: None,
                },
            ),
        ] {
            let mut chain = build_turn_lifecycle_hooks(
                &[spec(HookEvent::UserPromptSubmit, OnError::Warn)],
                HookEvent::UserPromptSubmit,
                dispatcher(r#"{"decision":"mutate","patch":{"message":"step1"}}"#, 0),
            );
            let blocked = dispatcher(stdout, 0);
            chain.extend(build_turn_lifecycle_hooks(
                &[spec(HookEvent::UserPromptSubmit, policy)],
                HookEvent::UserPromptSubmit,
                blocked.clone(),
            ));
            chain.extend(build_turn_lifecycle_hooks(
                &[spec(HookEvent::UserPromptSubmit, OnError::Warn)],
                HookEvent::UserPromptSubmit,
                Arc::new(MustNotRun),
            ));
            assert_eq!(
                run_user_prompt_submit_hooks(&chain, &turn_ctx(), "orig".into()).await,
                expected
            );
            let payloads = blocked.payloads.lock().unwrap();
            assert_eq!(payloads.len(), 1);
            assert_eq!(payloads[0].data, json!({"message":"step1"}));
            assert_eq!(payloads[0].session_id, SessionId::from_seed(1));
            assert_eq!(
                payloads[0].turn_id.as_deref(),
                Some("turn_00000000000000000000000000000002")
            );
            assert_eq!(payloads[0].org_id, Some(OrgId::from_seed(3)));
            assert_eq!(payloads[0].agent_id.as_deref(), Some("agent_4"));
        }
    }

    #[tokio::test]
    async fn turn_end_ignores_every_outcome_and_preserves_context_order() {
        for stdout in [
            "",
            "bad",
            r#"{"decision":"block","reason":"ignored"}"#,
            r#"{"decision":"mutate","patch":{"message":"ignored"}}"#,
        ] {
            let dispatch = dispatcher(stdout, 0);
            let mut second = spec(HookEvent::TurnEnd, OnError::Block);
            second.id = Some("second".into());
            let hooks = build_turn_lifecycle_hooks(
                &[spec(HookEvent::TurnEnd, OnError::Block), second],
                HookEvent::TurnEnd,
                dispatch.clone(),
            );
            run_turn_end_hooks(
                &hooks,
                &turn_ctx(),
                json!({"success":true,"summary":"done"}),
            )
            .await;
            let payloads = dispatch.payloads.lock().unwrap();
            assert_eq!(
                payloads
                    .iter()
                    .map(|p| p.hook_id.as_str())
                    .collect::<Vec<_>>(),
                ["user:t", "user:second"]
            );
            for p in payloads.iter() {
                assert_eq!(p.event, HookEvent::TurnEnd);
                assert_eq!(p.data, json!({"success":true,"summary":"done"}));
                assert_eq!(p.session_id, SessionId::from_seed(1));
                assert_eq!(
                    p.turn_id.as_deref(),
                    Some("turn_00000000000000000000000000000002")
                );
                assert_eq!(p.org_id, Some(OrgId::from_seed(3)));
                assert_eq!(p.agent_id.as_deref(), Some("agent_4"));
            }
        }
    }

    #[tokio::test]
    async fn session_events_are_advisory_and_preserve_optional_context() {
        for event in [HookEvent::SessionStart, HookEvent::SessionEnd] {
            for stdout in [
                "",
                "bad",
                r#"{"decision":"block","reason":"ignored"}"#,
                r#"{"decision":"mutate","patch":{"message":"ignored"}}"#,
            ] {
                let dispatch = dispatcher(stdout, 0);
                let hooks = build_session_lifecycle_hooks(
                    &[spec(event, OnError::Block), spec(event, OnError::Warn)],
                    event,
                    dispatch.clone(),
                );
                let ctx = SessionHookContext {
                    session_id: SessionId::from_seed(1),
                    org_id: Some(OrgId::from_seed(3)),
                    agent_id: Some("agent_4".into()),
                };
                run_session_lifecycle_hooks(&hooks, &ctx, json!({"reason":"requested"})).await;
                let payloads = dispatch.payloads.lock().unwrap();
                assert_eq!(payloads.len(), 2);
                for p in payloads.iter() {
                    assert_eq!(p.event, event);
                    assert_eq!(p.session_id, SessionId::from_seed(1));
                    assert_eq!(p.org_id, Some(OrgId::from_seed(3)));
                    assert_eq!(p.agent_id.as_deref(), Some("agent_4"));
                    assert_eq!(p.turn_id, None);
                    assert_eq!(p.data, json!({"reason":"requested"}));
                }
            }
        }
    }

    #[tokio::test]
    async fn builders_skip_invalid_specs_keep_source_indices_and_forward_settings() {
        for (event, other, expected_id) in [
            (
                HookEvent::SessionStart,
                HookEvent::SessionEnd,
                "user:session_start_2",
            ),
            (
                HookEvent::SessionEnd,
                HookEvent::SessionStart,
                "user:session_end_2",
            ),
            (
                HookEvent::TurnEnd,
                HookEvent::UserPromptSubmit,
                "user:turn_end_2",
            ),
            (
                HookEvent::UserPromptSubmit,
                HookEvent::TurnEnd,
                "user:user_prompt_submit_2",
            ),
        ] {
            let mut invalid = spec(event, OnError::Warn);
            invalid.timeout_ms = 99;
            let mut valid = spec(event, OnError::Allow);
            valid.id = None;
            valid.timeout_ms = 1234;
            valid.executor = ExecutorSpec::Bash {
                command: "custom".into(),
                env: BTreeMap::from([("A".into(), "B".into())]),
            };
            let specs = [spec(other, OnError::Warn), invalid, valid];
            let dispatch = dispatcher("", 0);
            if matches!(event, HookEvent::SessionStart | HookEvent::SessionEnd) {
                let hooks = build_session_lifecycle_hooks(&specs, event, dispatch.clone());
                assert_eq!(hooks.len(), 1);
                assert_eq!(hooks[0].event(), event);
                assert_eq!(hooks[0].hook_id().as_str(), expected_id);
                hooks[0]
                    .fire(
                        &SessionHookContext {
                            session_id: SessionId::from_seed(1),
                            org_id: None,
                            agent_id: None,
                        },
                        json!({}),
                    )
                    .await;
            } else {
                let hooks = build_turn_lifecycle_hooks(&specs, event, dispatch.clone());
                assert_eq!(hooks.len(), 1);
                assert_eq!(hooks[0].event(), event);
                assert_eq!(hooks[0].hook_id().as_str(), expected_id);
                assert_eq!(hooks[0].on_error(), OnError::Allow);
                let ctx = TurnHookContext {
                    session_id: SessionId::from_seed(1),
                    turn_id: None,
                    org_id: None,
                    agent_id: None,
                };
                assert!(matches!(
                    hooks[0].run(&ctx, json!({})).await,
                    HookOutcome::Allow
                ));
            }
            assert_eq!(
                *dispatch.settings.lock().unwrap(),
                [(
                    "custom".into(),
                    BTreeMap::from([("A".into(), "B".into())]),
                    1234,
                    65536
                )]
            );
            let payloads = dispatch.payloads.lock().unwrap();
            assert_eq!(payloads.len(), 1);
            assert_eq!(payloads[0].org_id, None);
            assert_eq!(payloads[0].agent_id, None);
            assert_eq!(payloads[0].turn_id, None);
        }
    }
}

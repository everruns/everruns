// User-defined hooks: adapters that bridge `UserHookSpec` to the runtime's
// per-event hook traits.
//
// `HookAdapterBuilder` is the single place specs become `Arc<dyn …Hook>`.
// Capability authors return data (`Vec<UserHookSpec>`); this module owns the
// translation to executable adapters so timeout/output/sandbox limits stay
// centrally enforced.
//
// Wired adapters: `PreToolUseHookAdapter` (with `build_pre_tool_use_hooks`)
// and `PostToolUseHookAdapter` (with `build_post_tool_use_hooks`). Adapters
// for `session_*`, `user_prompt_submit`, `turn_end` land alongside their
// respective runtime wire-in PRs.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use crate::hook_executor::{
    BashHookDispatcher, BashHookExecutor, ExecutorOpts, HookExecutor, HookPayload,
};
use crate::tool_context::ToolContext;
use crate::tool_hooks::{PostToolExecHook, PreToolUseDecision, PreToolUseHook};
use crate::tool_types::{ToolCall, ToolDefinition, ToolResult};
use crate::user_hook_types::{
    ExecutorSpec, HookEvent, HookOutcome, HookSource, OnError, UserHookSpec,
};

// ============================================================================
// PostToolUseHookAdapter
// ============================================================================

/// Adapter that implements `PostToolExecHook` for a single `UserHookSpec`
/// whose event is `post_tool_use`.
///
/// Lifecycle per `after_exec` call:
///
/// 1. Skip when the matcher rejects the (tool_name, arguments) pair.
/// 2. Build a `HookPayload` from the tool call + result.
/// 3. Run the executor with the spec's `timeout_ms`.
/// 4. On `Allow`, do nothing.
/// 5. On `Mutate { patch }`, apply the patch to the `ToolResult`:
///    - `patch.result` overwrites `result.result`
///    - `patch.error` overwrites `result.error`
///    - `patch.additional_context` is appended as a `hook_context` field on
///      `result.result` so the model sees the hook's commentary.
/// 6. On `Block`, log a warning — `post_tool_use` cannot block by spec.
/// 7. On `Error`, apply the spec's `on_error` policy. `Block` policy
///    overwrites `result.error`; `Warn`/`Allow` just log.
pub struct PostToolUseHookAdapter {
    spec: UserHookSpec,
    executor: Arc<dyn HookExecutor>,
    opts: ExecutorOpts,
    /// Resolved once at construction so every payload/log line carries a
    /// stable, unique id even for specs that omit an explicit `id` (the
    /// chain position disambiguates them). See `finalize_hook_specs`.
    hook_id: crate::user_hook_types::HookId,
}

impl PostToolUseHookAdapter {
    pub fn new(spec: UserHookSpec, executor: Arc<dyn HookExecutor>) -> Self {
        Self::with_index(spec, executor, 0)
    }

    pub fn with_index(spec: UserHookSpec, executor: Arc<dyn HookExecutor>, index: usize) -> Self {
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

    fn build_payload(
        &self,
        tool_call: &ToolCall,
        result: &ToolResult,
        context: &ToolContext,
    ) -> HookPayload {
        let success = result.error.is_none();
        HookPayload {
            event: HookEvent::PostToolUse,
            hook_id: self.hook_id.clone(),
            session_id: context.session_id,
            turn_id: None,
            org_id: context.org_id,
            agent_id: None,
            ts: chrono::Utc::now().to_rfc3339(),
            data: json!({
                "tool_name": tool_call.name,
                "tool_call_id": tool_call.id,
                "arguments": tool_call.arguments,
                "result": result.result,
                "error": result.error,
                "success": success,
            }),
        }
    }
}

#[async_trait]
impl PostToolExecHook for PostToolUseHookAdapter {
    async fn after_exec(
        &self,
        tool_call: &ToolCall,
        _tool_def: &ToolDefinition,
        result: &mut ToolResult,
        context: &ToolContext,
    ) {
        if !self
            .spec
            .matcher
            .matches(&tool_call.name, &tool_call.arguments)
        {
            return;
        }

        let payload = self.build_payload(tool_call, result, context);
        let outcome = self.executor.run(payload, &self.opts).await;
        let hook_id = &self.hook_id;

        match outcome {
            HookOutcome::Allow => {}
            HookOutcome::Mutate { patch, .. } => apply_post_tool_use_patch(result, &patch),
            HookOutcome::Block { reason, .. } => {
                tracing::warn!(
                    hook_id = %hook_id.as_str(),
                    tool_call_id = %tool_call.id,
                    reason = %reason,
                    "post_tool_use hook returned Block, which is not allowed for this event; ignoring"
                );
            }
            HookOutcome::Error { message } => match self.spec.on_error {
                OnError::Block => {
                    // post_tool_use cannot block execution that already
                    // happened, but we can replace the result with an
                    // error so the model sees the failure.
                    result.error = Some(format!("hook {}: {}", hook_id.as_str(), message));
                    tracing::warn!(
                        hook_id = %hook_id.as_str(),
                        tool_call_id = %tool_call.id,
                        message = %message,
                        "post_tool_use hook errored with on_error=block; replacing tool result with error"
                    );
                }
                OnError::Warn => {
                    tracing::warn!(
                        hook_id = %hook_id.as_str(),
                        tool_call_id = %tool_call.id,
                        message = %message,
                        "post_tool_use hook errored"
                    );
                }
                OnError::Allow => {}
            },
        }
    }
}

/// Apply a `post_tool_use` `Mutate` patch to a `ToolResult` in place.
///
/// Patch shape (any subset, see `knowledge/runtime-resources/user-hooks.md`):
///   { "result": …, "error": …, "additional_context": "..." }
fn apply_post_tool_use_patch(result: &mut ToolResult, patch: &serde_json::Value) {
    if let Some(new_result) = patch.get("result") {
        result.result = Some(new_result.clone());
    }
    if let Some(new_error) = patch.get("error").and_then(|v| v.as_str()) {
        result.error = Some(new_error.to_string());
    }
    if let Some(ctx) = patch.get("additional_context").and_then(|v| v.as_str()) {
        // Append as `hook_context` so the model sees the hook's narration.
        match result.result.as_mut() {
            Some(serde_json::Value::Object(map)) => {
                map.insert("hook_context".to_string(), json!(ctx));
            }
            Some(other) => {
                let prior = other.clone();
                *other = json!({ "value": prior, "hook_context": ctx });
            }
            None => {
                result.result = Some(json!({ "hook_context": ctx }));
            }
        }
    }
}

// ============================================================================
// Factory
// ============================================================================

/// Build `PostToolExecHook` adapters from every `UserHookSpec` in `specs`
/// whose event is `PostToolUse`. `dispatcher` is the bash backend used for
/// every spec whose executor is `Bash`; other backends will land alongside
/// their own dispatchers.
///
/// Specs that fail `validate()` are silently dropped with a warning log so
/// one bad spec doesn't take down the entire chain.
pub fn build_post_tool_use_hooks(
    specs: &[UserHookSpec],
    dispatcher: Arc<dyn BashHookDispatcher>,
) -> Vec<Arc<dyn PostToolExecHook>> {
    let mut out: Vec<Arc<dyn PostToolExecHook>> = Vec::new();
    for (index, spec) in specs.iter().enumerate() {
        if spec.event != HookEvent::PostToolUse {
            continue;
        }
        if let Err(e) = spec.validate() {
            let hook_id_for_log = spec.resolve_id(index);
            tracing::warn!(
                hook_id = %hook_id_for_log.as_str(),
                error = %e,
                "skipping invalid post_tool_use hook spec"
            );
            continue;
        }
        let executor: Arc<dyn HookExecutor> = match &spec.executor {
            ExecutorSpec::Bash { command, env } => Arc::new(BashHookExecutor::with_dispatcher(
                command.clone(),
                env.clone(),
                dispatcher.clone(),
            )),
        };
        out.push(Arc::new(PostToolUseHookAdapter::with_index(
            spec.clone(),
            executor,
            index,
        )));
    }
    out
}

/// Finalize contributed hook specs before they become adapters.
///
/// Each entry in `contributions` is `(capability_id, specs)` as returned by a
/// single capability's `user_hooks_with_config`. This function:
///
/// 1. **Stamps the source namespace.** Specs from any capability other than
///    the user-facing `user_hooks` capability are stamped
///    `HookSource::Capability { capability_id }` so their resolved `HookId`
///    lands in the `{capability_id}:` namespace. (The `user_hooks` capability
///    already stamps its own entries `UserConfig`.) This guarantees correct
///    namespacing regardless of whether the capability author set `source` —
///    per the `HookSource` doc contract, the runtime owns this stamp.
/// 2. **Assigns a stable default id** (`{event}_{idx}`, indexed within the
///    contributing capability) to any spec that omits an explicit `id`, so
///    every hook is individually addressable and mutable.
/// 3. **Applies `disabled_contributions`.** Any spec whose resolved `HookId`
///    appears in `disabled` is dropped, so operators can mute
///    capability-contributed hooks (TM-HOOK-004).
pub fn finalize_hook_specs(
    contributions: Vec<(String, Vec<UserHookSpec>)>,
    disabled: &[String],
) -> Vec<UserHookSpec> {
    let disabled: std::collections::HashSet<&str> = disabled.iter().map(String::as_str).collect();
    let mut out: Vec<UserHookSpec> = Vec::new();
    for (capability_id, specs) in contributions {
        for (idx, mut spec) in specs.into_iter().enumerate() {
            // The `user_hooks` capability stamps its own entries `UserConfig`
            // (and assigns ids) in `parse_config`. Every other capability is a
            // capability contribution; stamp it here so authors can't forget.
            if capability_id != "user_hooks" {
                spec.source = HookSource::Capability {
                    capability_id: capability_id.clone(),
                };
                if spec.id.is_none() {
                    spec.id = Some(format!("{}_{}", spec.event.as_str(), idx));
                }
            }
            let resolved = spec.resolve_id(idx);
            if disabled.contains(resolved.as_str()) {
                tracing::info!(
                    hook_id = %resolved.as_str(),
                    "muting hook via disabled_contributions"
                );
                continue;
            }
            out.push(spec);
        }
    }
    out
}

/// Convenience: classify a stable hook id from a spec source. Useful for
/// audit logs that need the `{capability_id}:{name}` namespace.
pub fn hook_id_namespace(spec: &UserHookSpec) -> &'static str {
    match spec.source {
        HookSource::UserConfig => "user",
        HookSource::Capability { .. } => "capability",
    }
}

// ============================================================================
// PreToolUseHookAdapter
// ============================================================================

/// Adapter that implements `PreToolUseHook` for a single `UserHookSpec`
/// whose event is `pre_tool_use`.
///
/// Lifecycle per `before_exec` call:
///
/// 1. Skip when the matcher rejects the (tool_name, arguments) pair —
///    `Continue(tool_call)` with no mutation.
/// 2. Build a `HookPayload` from the tool call.
/// 3. Run the executor with the spec's `timeout_ms`.
/// 4. On `Allow`, return `Continue(tool_call)` unchanged.
/// 5. On `Mutate { patch }`, merge `patch.arguments` into
///    `tool_call.arguments` and return `Continue`.
/// 6. On `Block { reason, user_message }`, return `Block` — `ActAtom`
///    short-circuits the tool call.
/// 7. On `Error`, apply the spec's `on_error` policy:
///    - `Block` → `Block { reason = "hook errored: …" }`
///    - `Warn` / `Allow` → log + `Continue(tool_call)` unchanged.
pub struct PreToolUseHookAdapter {
    spec: UserHookSpec,
    executor: Arc<dyn HookExecutor>,
    opts: ExecutorOpts,
    /// Resolved once at construction; see `PostToolUseHookAdapter::hook_id`.
    hook_id: crate::user_hook_types::HookId,
}

impl PreToolUseHookAdapter {
    pub fn new(spec: UserHookSpec, executor: Arc<dyn HookExecutor>) -> Self {
        Self::with_index(spec, executor, 0)
    }

    pub fn with_index(spec: UserHookSpec, executor: Arc<dyn HookExecutor>, index: usize) -> Self {
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

    fn build_payload(&self, tool_call: &ToolCall, context: &ToolContext) -> HookPayload {
        HookPayload {
            event: HookEvent::PreToolUse,
            hook_id: self.hook_id.clone(),
            session_id: context.session_id,
            turn_id: None,
            org_id: context.org_id,
            agent_id: None,
            ts: chrono::Utc::now().to_rfc3339(),
            data: json!({
                "tool_name": tool_call.name,
                "tool_call_id": tool_call.id,
                "arguments": tool_call.arguments,
            }),
        }
    }
}

#[async_trait]
impl PreToolUseHook for PreToolUseHookAdapter {
    async fn before_exec(
        &self,
        tool_call: ToolCall,
        _tool_def: &ToolDefinition,
        context: &ToolContext,
    ) -> PreToolUseDecision {
        if !self
            .spec
            .matcher
            .matches(&tool_call.name, &tool_call.arguments)
        {
            return PreToolUseDecision::Continue(tool_call);
        }

        let payload = self.build_payload(&tool_call, context);
        let outcome = self.executor.run(payload, &self.opts).await;
        let hook_id = &self.hook_id;

        match outcome {
            HookOutcome::Allow => PreToolUseDecision::Continue(tool_call),
            HookOutcome::Mutate { patch, .. } => {
                let mutated = apply_pre_tool_use_patch(tool_call, &patch);
                PreToolUseDecision::Continue(mutated)
            }
            HookOutcome::Block {
                reason,
                user_message,
            } => PreToolUseDecision::Block {
                tool_call,
                reason,
                user_message,
            },
            HookOutcome::Error { message } => match self.spec.on_error {
                OnError::Block => PreToolUseDecision::Block {
                    tool_call,
                    reason: format!("hook {} errored: {}", hook_id.as_str(), message),
                    user_message: None,
                },
                OnError::Warn => {
                    tracing::warn!(
                        hook_id = %hook_id.as_str(),
                        tool_call_id = %tool_call.id,
                        message = %message,
                        "pre_tool_use hook errored"
                    );
                    PreToolUseDecision::Continue(tool_call)
                }
                OnError::Allow => PreToolUseDecision::Continue(tool_call),
            },
        }
    }
}

/// Apply a `pre_tool_use` `Mutate` patch to a `ToolCall`. Only the
/// `patch.arguments` object is honored; it is merged into the existing
/// arguments, with patch keys winning on conflict. Non-object patches
/// and missing fields are silently ignored.
fn apply_pre_tool_use_patch(mut tool_call: ToolCall, patch: &serde_json::Value) -> ToolCall {
    if let Some(new_args) = patch.get("arguments")
        && let Some(new_obj) = new_args.as_object()
    {
        match tool_call.arguments.as_object_mut() {
            Some(existing) => {
                for (k, v) in new_obj {
                    existing.insert(k.clone(), v.clone());
                }
            }
            None => {
                tool_call.arguments = serde_json::Value::Object(new_obj.clone());
            }
        }
    }
    tool_call
}

/// Build `PreToolUseHook` adapters from every `UserHookSpec` in `specs`
/// whose event is `PreToolUse`. Specs that fail `validate()` are dropped
/// with a warning log so one bad spec doesn't take down the chain.
pub fn build_pre_tool_use_hooks(
    specs: &[UserHookSpec],
    dispatcher: Arc<dyn BashHookDispatcher>,
) -> Vec<Arc<dyn PreToolUseHook>> {
    let mut out: Vec<Arc<dyn PreToolUseHook>> = Vec::new();
    for (index, spec) in specs.iter().enumerate() {
        if spec.event != HookEvent::PreToolUse {
            continue;
        }
        if let Err(e) = spec.validate() {
            let hook_id_for_log = spec.resolve_id(index);
            tracing::warn!(
                hook_id = %hook_id_for_log.as_str(),
                error = %e,
                "skipping invalid pre_tool_use hook spec"
            );
            continue;
        }
        let executor: Arc<dyn HookExecutor> = match &spec.executor {
            ExecutorSpec::Bash { command, env } => Arc::new(BashHookExecutor::with_dispatcher(
                command.clone(),
                env.clone(),
                dispatcher.clone(),
            )),
        };
        out.push(Arc::new(PreToolUseHookAdapter::with_index(
            spec.clone(),
            executor,
            index,
        )));
    }
    out
}

#[cfg(test)]
mod pre_tool_use_tests {
    use super::*;
    use crate::tool_types::{BuiltinTool, DeferrablePolicy, ToolHints, ToolPolicy};
    use serde_json::json;
    use std::sync::Mutex;

    fn make_spec(matcher: crate::user_hook_types::HookMatcher) -> UserHookSpec {
        UserHookSpec {
            id: Some("pre".into()),
            event: HookEvent::PreToolUse,
            matcher,
            executor: ExecutorSpec::Bash {
                command: "true".into(),
                env: Default::default(),
            },
            timeout_ms: 5000,
            on_error: OnError::Warn,
            description: None,
            source: HookSource::UserConfig,
        }
    }

    fn make_tool_call() -> ToolCall {
        ToolCall {
            id: "call_x".into(),
            name: "bash".into(),
            arguments: json!({"command": "rm -rf /"}),
        }
    }

    fn make_tool_def() -> ToolDefinition {
        ToolDefinition::Builtin(BuiltinTool {
            name: "bash".into(),
            display_name: None,
            description: "".into(),
            parameters: json!({}),
            policy: ToolPolicy::Auto,
            category: None,
            deferrable: DeferrablePolicy::Never,
            hints: ToolHints::default(),
            full_parameters: None,
        })
    }

    struct ProgrammedExecutor {
        outcome: HookOutcome,
        calls: Mutex<Vec<HookPayload>>,
    }

    #[async_trait]
    impl HookExecutor for ProgrammedExecutor {
        fn kind(&self) -> &'static str {
            "test"
        }
        async fn run(&self, payload: HookPayload, _opts: &ExecutorOpts) -> HookOutcome {
            self.calls.lock().unwrap().push(payload);
            self.outcome.clone()
        }
    }

    fn programmed(outcome: HookOutcome) -> Arc<ProgrammedExecutor> {
        Arc::new(ProgrammedExecutor {
            outcome,
            calls: Mutex::new(Vec::new()),
        })
    }

    #[tokio::test]
    async fn matcher_miss_skips_executor() {
        let exec = programmed(HookOutcome::Allow);
        let exec_arc: Arc<dyn HookExecutor> = exec.clone();
        let matcher = crate::user_hook_types::HookMatcher {
            tool_name: Some("edit_file".into()),
            ..Default::default()
        };
        let adapter = PreToolUseHookAdapter::new(make_spec(matcher), exec_arc);
        let ctx = ToolContext::new(crate::typed_id::SessionId::from_uuid(uuid::Uuid::nil()));

        let decision = adapter
            .before_exec(make_tool_call(), &make_tool_def(), &ctx)
            .await;
        assert!(matches!(decision, PreToolUseDecision::Continue(_)));
        assert_eq!(exec.calls.lock().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn allow_outcome_returns_continue_unchanged() {
        let exec_arc: Arc<dyn HookExecutor> = programmed(HookOutcome::Allow);
        let adapter = PreToolUseHookAdapter::new(make_spec(Default::default()), exec_arc);
        let ctx = ToolContext::new(crate::typed_id::SessionId::from_uuid(uuid::Uuid::nil()));
        let original = make_tool_call();

        let decision = adapter
            .before_exec(original.clone(), &make_tool_def(), &ctx)
            .await;
        match decision {
            PreToolUseDecision::Continue(tc) => assert_eq!(tc.arguments, original.arguments),
            other => panic!("expected Continue, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn block_outcome_propagates() {
        let exec_arc: Arc<dyn HookExecutor> = programmed(HookOutcome::Block {
            reason: "denied".into(),
            user_message: Some("nope".into()),
        });
        let adapter = PreToolUseHookAdapter::new(make_spec(Default::default()), exec_arc);
        let ctx = ToolContext::new(crate::typed_id::SessionId::from_uuid(uuid::Uuid::nil()));

        let decision = adapter
            .before_exec(make_tool_call(), &make_tool_def(), &ctx)
            .await;
        match decision {
            PreToolUseDecision::Block {
                reason,
                user_message,
                ..
            } => {
                assert_eq!(reason, "denied");
                assert_eq!(user_message.as_deref(), Some("nope"));
            }
            other => panic!("expected Block, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn mutate_patch_merges_arguments() {
        let exec_arc: Arc<dyn HookExecutor> = programmed(HookOutcome::Mutate {
            patch: json!({ "arguments": { "command": "ls" } }),
            reason: None,
        });
        let adapter = PreToolUseHookAdapter::new(make_spec(Default::default()), exec_arc);
        let ctx = ToolContext::new(crate::typed_id::SessionId::from_uuid(uuid::Uuid::nil()));

        let decision = adapter
            .before_exec(make_tool_call(), &make_tool_def(), &ctx)
            .await;
        match decision {
            PreToolUseDecision::Continue(tc) => {
                assert_eq!(tc.arguments["command"], "ls");
            }
            other => panic!("expected Continue, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn error_with_on_error_block_returns_block() {
        let exec_arc: Arc<dyn HookExecutor> = programmed(HookOutcome::Error {
            message: "boom".into(),
        });
        let mut spec = make_spec(Default::default());
        spec.on_error = OnError::Block;
        let adapter = PreToolUseHookAdapter::new(spec, exec_arc);
        let ctx = ToolContext::new(crate::typed_id::SessionId::from_uuid(uuid::Uuid::nil()));

        let decision = adapter
            .before_exec(make_tool_call(), &make_tool_def(), &ctx)
            .await;
        match decision {
            PreToolUseDecision::Block { reason, .. } => {
                assert!(reason.contains("boom"), "{reason}");
            }
            other => panic!("expected Block, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn error_with_on_error_warn_returns_continue() {
        let exec_arc: Arc<dyn HookExecutor> = programmed(HookOutcome::Error {
            message: "boom".into(),
        });
        let adapter = PreToolUseHookAdapter::new(make_spec(Default::default()), exec_arc);
        let ctx = ToolContext::new(crate::typed_id::SessionId::from_uuid(uuid::Uuid::nil()));

        let decision = adapter
            .before_exec(make_tool_call(), &make_tool_def(), &ctx)
            .await;
        assert!(matches!(decision, PreToolUseDecision::Continue(_)));
    }

    #[tokio::test]
    async fn factory_validates_globs_and_dispatches_only_matching_pre_tool_hooks() {
        let valid = make_spec(crate::user_hook_types::HookMatcher {
            tool_name_glob: Some(" bash* | edit_file ".into()),
            ..Default::default()
        });
        let mut invalid = valid.clone();
        invalid.matcher.tool_name_glob = Some("bash**".into());
        let specs = vec![
            valid.clone(),
            UserHookSpec {
                event: HookEvent::PostToolUse,
                ..valid
            },
            invalid,
        ];
        struct RecordingDispatcher(Mutex<Vec<HookPayload>>);
        #[async_trait]
        impl BashHookDispatcher for RecordingDispatcher {
            async fn dispatch(
                &self,
                payload: &HookPayload,
                command: &str,
                extra_env: &std::collections::BTreeMap<String, String>,
                opts: &ExecutorOpts,
            ) -> Result<crate::hook_executor::BashExecOutput, String> {
                assert_eq!(command, "true");
                assert!(extra_env.is_empty());
                assert_eq!(opts.timeout_ms, 5000);
                self.0.lock().unwrap().push(payload.clone());
                Ok(crate::hook_executor::BashExecOutput {
                    exit_code: 0,
                    stdout: r#"{"decision":"block","reason":"policy","user_message":"denied"}"#
                        .into(),
                    stderr: String::new(),
                })
            }
        }
        let dispatcher = Arc::new(RecordingDispatcher(Mutex::new(Vec::new())));
        let hooks = build_pre_tool_use_hooks(&specs, dispatcher.clone());
        assert_eq!(hooks.len(), 1);
        let ctx = ToolContext::new(crate::typed_id::SessionId::from_seed(42));
        let call = make_tool_call();
        match hooks[0]
            .before_exec(call.clone(), &make_tool_def(), &ctx)
            .await
        {
            PreToolUseDecision::Block {
                tool_call,
                reason,
                user_message,
            } => {
                assert_eq!(tool_call.id, call.id);
                assert_eq!(tool_call.arguments, call.arguments);
                assert_eq!(reason, "policy");
                assert_eq!(user_message.as_deref(), Some("denied"));
            }
            other => panic!("expected executed hook block, got {other:?}"),
        }
        let mut unrelated = call.clone();
        unrelated.name = "read_file".into();
        assert!(matches!(
            hooks[0]
                .before_exec(unrelated, &make_tool_def(), &ctx)
                .await,
            PreToolUseDecision::Continue(_)
        ));
        let payloads = dispatcher.0.lock().unwrap();
        assert_eq!(payloads.len(), 1);
        assert_eq!(payloads[0].event, HookEvent::PreToolUse);
        assert_eq!(payloads[0].session_id, ctx.session_id);
        assert_eq!(payloads[0].hook_id.as_str(), "user:pre");
        assert_eq!(
            payloads[0].data,
            json!({"tool_name":"bash","tool_call_id":"call_x","arguments":{"command":"rm -rf /"}})
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool_types::{BuiltinTool, DeferrablePolicy, ToolHints, ToolPolicy};
    use serde_json::json;
    use std::sync::Mutex;

    fn make_spec(event: HookEvent, command: &str) -> UserHookSpec {
        UserHookSpec {
            id: Some("t".into()),
            event,
            matcher: Default::default(),
            executor: ExecutorSpec::Bash {
                command: command.into(),
                env: Default::default(),
            },
            timeout_ms: 5000,
            on_error: OnError::Warn,
            description: None,
            source: HookSource::UserConfig,
        }
    }

    fn make_tool_call(name: &str) -> ToolCall {
        ToolCall {
            id: "call_1".into(),
            name: name.into(),
            arguments: json!({}),
        }
    }

    fn make_tool_def(name: &str) -> ToolDefinition {
        ToolDefinition::Builtin(BuiltinTool {
            name: name.into(),
            display_name: None,
            description: "x".into(),
            parameters: json!({}),
            policy: ToolPolicy::Auto,
            category: None,
            deferrable: DeferrablePolicy::Never,
            hints: ToolHints::default(),
            full_parameters: None,
        })
    }

    fn empty_result() -> ToolResult {
        ToolResult {
            tool_call_id: "call_1".into(),
            result: Some(json!({"out": "stuff"})),
            images: None,
            error: None,
            connection_required: None,
            raw_output: None,
        }
    }

    /// Test executor that records calls and returns a programmed outcome.
    struct ProgrammedExecutor {
        outcome: HookOutcome,
        calls: Mutex<Vec<HookPayload>>,
    }

    #[async_trait]
    impl HookExecutor for ProgrammedExecutor {
        fn kind(&self) -> &'static str {
            "test"
        }
        async fn run(&self, payload: HookPayload, _opts: &ExecutorOpts) -> HookOutcome {
            self.calls.lock().unwrap().push(payload);
            self.outcome.clone()
        }
    }

    fn programmed(outcome: HookOutcome) -> Arc<ProgrammedExecutor> {
        Arc::new(ProgrammedExecutor {
            outcome,
            calls: Mutex::new(Vec::new()),
        })
    }

    #[tokio::test]
    async fn adapter_runs_executor_when_matcher_passes() {
        let exec = programmed(HookOutcome::Allow);
        let exec_arc: Arc<dyn HookExecutor> = exec.clone();
        let adapter =
            PostToolUseHookAdapter::new(make_spec(HookEvent::PostToolUse, "true"), exec_arc);

        let tc = make_tool_call("edit_file");
        let td = make_tool_def("edit_file");
        let mut result = empty_result();
        let ctx = ToolContext::new(crate::typed_id::SessionId::from_uuid(uuid::Uuid::nil()));
        adapter.after_exec(&tc, &td, &mut result, &ctx).await;

        assert_eq!(exec.calls.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn adapter_skips_when_matcher_rejects() {
        let exec = programmed(HookOutcome::Allow);
        let mut spec = make_spec(HookEvent::PostToolUse, "true");
        spec.matcher.tool_name = Some("read_file".into()); // won't match edit_file
        let exec_arc: Arc<dyn HookExecutor> = exec.clone();
        let adapter = PostToolUseHookAdapter::new(spec, exec_arc);

        let tc = make_tool_call("edit_file");
        let td = make_tool_def("edit_file");
        let mut result = empty_result();
        let ctx = ToolContext::new(crate::typed_id::SessionId::from_uuid(uuid::Uuid::nil()));
        adapter.after_exec(&tc, &td, &mut result, &ctx).await;

        assert_eq!(exec.calls.lock().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn mutate_patch_replaces_result_and_error() {
        let exec_arc: Arc<dyn HookExecutor> = Arc::new(ProgrammedExecutor {
            outcome: HookOutcome::Mutate {
                patch: json!({
                    "result": {"replaced": true},
                    "error": "redacted",
                }),
                reason: None,
            },
            calls: Mutex::new(Vec::new()),
        });
        let adapter =
            PostToolUseHookAdapter::new(make_spec(HookEvent::PostToolUse, "true"), exec_arc);

        let tc = make_tool_call("edit_file");
        let td = make_tool_def("edit_file");
        let mut result = empty_result();
        let ctx = ToolContext::new(crate::typed_id::SessionId::from_uuid(uuid::Uuid::nil()));
        adapter.after_exec(&tc, &td, &mut result, &ctx).await;

        assert_eq!(result.result, Some(json!({"replaced": true})));
        assert_eq!(result.error.as_deref(), Some("redacted"));
    }

    #[tokio::test]
    async fn mutate_additional_context_appends_hook_context() {
        let exec_arc: Arc<dyn HookExecutor> = Arc::new(ProgrammedExecutor {
            outcome: HookOutcome::Mutate {
                patch: json!({"additional_context": "fmt clean"}),
                reason: None,
            },
            calls: Mutex::new(Vec::new()),
        });
        let adapter =
            PostToolUseHookAdapter::new(make_spec(HookEvent::PostToolUse, "true"), exec_arc);

        let tc = make_tool_call("edit_file");
        let td = make_tool_def("edit_file");
        let mut result = empty_result();
        let ctx = ToolContext::new(crate::typed_id::SessionId::from_uuid(uuid::Uuid::nil()));
        adapter.after_exec(&tc, &td, &mut result, &ctx).await;

        // Should append a hook_context key to the existing object result.
        let r = result.result.as_ref().unwrap();
        assert_eq!(r["hook_context"], "fmt clean");
        assert_eq!(r["out"], "stuff"); // original keys preserved
    }

    #[tokio::test]
    async fn block_outcome_is_ignored_for_post_tool_use() {
        let exec_arc: Arc<dyn HookExecutor> = Arc::new(ProgrammedExecutor {
            outcome: HookOutcome::Block {
                reason: "bogus".into(),
                user_message: None,
            },
            calls: Mutex::new(Vec::new()),
        });
        let adapter =
            PostToolUseHookAdapter::new(make_spec(HookEvent::PostToolUse, "true"), exec_arc);

        let tc = make_tool_call("edit_file");
        let td = make_tool_def("edit_file");
        let mut result = empty_result();
        let original = result.result.clone();
        let ctx = ToolContext::new(crate::typed_id::SessionId::from_uuid(uuid::Uuid::nil()));
        adapter.after_exec(&tc, &td, &mut result, &ctx).await;

        assert_eq!(result.result, original);
        assert!(result.error.is_none());
    }

    #[tokio::test]
    async fn error_with_on_error_block_replaces_result_with_error() {
        let exec_arc: Arc<dyn HookExecutor> = Arc::new(ProgrammedExecutor {
            outcome: HookOutcome::Error {
                message: "boom".into(),
            },
            calls: Mutex::new(Vec::new()),
        });
        let mut spec = make_spec(HookEvent::PostToolUse, "true");
        spec.on_error = OnError::Block;
        let adapter = PostToolUseHookAdapter::new(spec, exec_arc);

        let tc = make_tool_call("edit_file");
        let td = make_tool_def("edit_file");
        let mut result = empty_result();
        let ctx = ToolContext::new(crate::typed_id::SessionId::from_uuid(uuid::Uuid::nil()));
        adapter.after_exec(&tc, &td, &mut result, &ctx).await;

        let err = result.error.unwrap();
        assert!(err.contains("boom"), "{err}");
    }

    #[tokio::test]
    async fn error_with_on_error_warn_keeps_result_intact() {
        let exec_arc: Arc<dyn HookExecutor> = Arc::new(ProgrammedExecutor {
            outcome: HookOutcome::Error {
                message: "boom".into(),
            },
            calls: Mutex::new(Vec::new()),
        });
        let adapter =
            PostToolUseHookAdapter::new(make_spec(HookEvent::PostToolUse, "true"), exec_arc);

        let tc = make_tool_call("edit_file");
        let td = make_tool_def("edit_file");
        let mut result = empty_result();
        let original = result.result.clone();
        let ctx = ToolContext::new(crate::typed_id::SessionId::from_uuid(uuid::Uuid::nil()));
        adapter.after_exec(&tc, &td, &mut result, &ctx).await;

        assert_eq!(result.result, original);
        assert!(result.error.is_none());
    }

    #[test]
    fn factory_filters_to_post_tool_use_event() {
        let specs = vec![
            make_spec(HookEvent::PostToolUse, "true"),
            make_spec(HookEvent::PreToolUse, "true"),
            make_spec(HookEvent::SessionStart, "true"),
        ];
        struct NoopDispatcher;
        #[async_trait]
        impl BashHookDispatcher for NoopDispatcher {
            async fn dispatch(
                &self,
                _payload: &HookPayload,
                _command: &str,
                _extra_env: &std::collections::BTreeMap<String, String>,
                _opts: &ExecutorOpts,
            ) -> Result<crate::hook_executor::BashExecOutput, String> {
                Ok(crate::hook_executor::BashExecOutput {
                    exit_code: 0,
                    stdout: String::new(),
                    stderr: String::new(),
                })
            }
        }
        let dispatcher: Arc<dyn BashHookDispatcher> = Arc::new(NoopDispatcher);
        let hooks = build_post_tool_use_hooks(&specs, dispatcher);
        assert_eq!(hooks.len(), 1, "only the PostToolUse spec should be built");
    }
}

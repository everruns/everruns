// User-defined hooks: adapters that bridge `UserHookSpec` to the runtime's
// per-event hook traits.
//
// `HookAdapterBuilder` is the single place specs become `Arc<dyn …Hook>`.
// Capability authors return data (`Vec<UserHookSpec>`); this module owns the
// translation to executable adapters so timeout/output/sandbox limits stay
// centrally enforced.
//
// This PR ships the `PostToolUseHookAdapter` and the
// `build_post_tool_use_hooks` factory. Adapters for `pre_tool_use`,
// `session_*`, `user_prompt_submit`, `turn_end` land alongside their
// respective runtime wire-in PRs.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use crate::atoms::PostToolExecHook;
use crate::hook_executor::{
    BashHookDispatcher, BashHookExecutor, ExecutorOpts, HookExecutor, HookPayload,
};
use crate::tool_types::{ToolCall, ToolDefinition, ToolResult};
use crate::traits::ToolContext;
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
}

impl PostToolUseHookAdapter {
    pub fn new(spec: UserHookSpec, executor: Arc<dyn HookExecutor>) -> Self {
        let opts = ExecutorOpts {
            timeout_ms: spec.timeout_ms,
            max_output_bytes: 64 * 1024,
        };
        Self {
            spec,
            executor,
            opts,
        }
    }

    fn build_payload(&self, tool_call: &ToolCall, result: &ToolResult) -> HookPayload {
        let success = result.error.is_none();
        HookPayload {
            event: HookEvent::PostToolUse,
            hook_id: self.spec.resolve_id(0),
            session_id: crate::typed_id::SessionId::from_uuid(uuid::Uuid::nil()),
            turn_id: None,
            org_id: None,
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

        let mut payload = self.build_payload(tool_call, result);
        // Fill in session/turn/org context the adapter knows about now that
        // it has a live ToolContext.
        payload.session_id = context.session_id;

        let outcome = self.executor.run(payload, &self.opts).await;
        let hook_id = self.spec.resolve_id(0);

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
/// Patch shape (any subset, see `specs/user-hooks.md`):
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
    for spec in specs {
        if spec.event != HookEvent::PostToolUse {
            continue;
        }
        if let Err(e) = spec.validate() {
            let hook_id_for_log = spec.resolve_id(0);
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
        out.push(Arc::new(PostToolUseHookAdapter::new(
            spec.clone(),
            executor,
        )));
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

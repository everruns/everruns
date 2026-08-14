// Post-act hooks for ActAtom
//
// Decision: Hooks are pure functions that inspect ActResult and return
// declarative PostActActions. ActAtom interprets them (event emission, etc).
// This keeps hooks testable without mocking EventEmitter.
//
// Decision: Hooks set `waiting_for_tool_results` on ActResult so workers
// see a single generic flag — they never need to know WHY the act paused.
//
// PostToolExecHook (EVE-222): async hooks that run after each individual tool
// execution. Unlike PostActHook (runs once after all tools), these run per-tool
// and can mutate the result (e.g. persist output to VFS, inject metadata).

use crate::events::{EventContext, EventRequest, ToolCallRequestedData};
use crate::tool_types::{ToolCall, ToolDefinition, ToolResult};
use crate::{event_emitter::EventEmitter, tool_context::ToolContext};
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

use super::AtomContext;
use super::act::ActResult;

// ============================================================================
// PreToolUseHook trait (per-tool, async, can mutate or block)
// ============================================================================

/// Decision returned by a `PreToolUseHook::before_exec` call.
#[derive(Debug, Clone)]
pub enum PreToolUseDecision {
    /// Continue with the (possibly mutated) tool call.
    Continue(ToolCall),
    /// Refuse to execute this tool call. The hook supplies the error
    /// message the model and audit log will see; the tool is not invoked.
    Block {
        tool_call: ToolCall,
        reason: String,
        /// Optional user-facing message surfaced through the runtime
        /// (when the runtime knows how to render it).
        user_message: Option<String>,
    },
}

/// Hook that runs before each individual tool execution.
///
/// Unlike `PostToolExecHook`, pre-hooks can both *mutate* the tool call
/// (returning `Continue` with a modified `ToolCall`) and *block* it
/// (returning `Block`, which aborts execution for that single tool call
/// without affecting sibling calls in the batch).
///
/// Hooks chain sequentially in registration order; each hook sees the
/// previous hook's mutated `ToolCall`. The first hook to return `Block`
/// wins — subsequent hooks in the chain are not consulted for the same
/// call.
#[async_trait]
pub trait PreToolUseHook: Send + Sync {
    async fn before_exec(
        &self,
        tool_call: ToolCall,
        tool_def: &ToolDefinition,
        context: &ToolContext,
    ) -> PreToolUseDecision;
}

/// Run every registered `PreToolUseHook` against `tool_call`. Hooks chain
/// sequentially; the first `Block` aborts the chain and is returned. If
/// every hook returns `Continue`, the final (potentially mutated)
/// `ToolCall` is returned.
pub(super) async fn run_pre_tool_use_hooks(
    hooks: &[Arc<dyn PreToolUseHook>],
    mut tool_call: ToolCall,
    tool_def: &ToolDefinition,
    context: &ToolContext,
) -> PreToolUseDecision {
    for hook in hooks {
        match hook.before_exec(tool_call.clone(), tool_def, context).await {
            PreToolUseDecision::Continue(updated) => {
                tool_call = updated;
            }
            block @ PreToolUseDecision::Block { .. } => return block,
        }
    }
    PreToolUseDecision::Continue(tool_call)
}

// ============================================================================
// PostToolExecHook trait (per-tool, async)
// ============================================================================

/// Hook that runs after each individual tool execution completes.
///
/// Unlike `PostActHook` (which runs once after all tools in a batch),
/// `PostToolExecHook` runs per-tool and can:
/// - Persist tool output to session VFS (EVE-222)
/// - Inject metadata into the result (e.g. `full_output` path)
/// - Enforce hard limits on result size (EVE-225)
///
/// Hooks are async because they may perform I/O (VFS writes).
/// Capability-contributed hooks run first, then final (infrastructure) hooks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PostToolExecHookPriority {
    /// Inspect/block output before other hooks can persist or transform it.
    Guardrail = 0,
    /// Default post-tool hook ordering for mutating/observability hooks.
    Normal = 100,
}

#[async_trait]
pub trait PostToolExecHook: Send + Sync {
    /// Ordering within the capability-contributed hook phase.
    fn priority(&self) -> PostToolExecHookPriority {
        PostToolExecHookPriority::Normal
    }

    /// Called after a tool returns its result, before ActAtom emits events.
    async fn after_exec(
        &self,
        tool_call: &ToolCall,
        tool_def: &ToolDefinition,
        result: &mut ToolResult,
        context: &ToolContext,
    );
}

/// Execute post-tool-exec hooks on a single tool result.
///
/// Runs capability-contributed hooks first, then final (infrastructure) hooks.
pub(super) async fn run_post_tool_exec_hooks(
    hooks: &[Arc<dyn PostToolExecHook>],
    final_hooks: &[Arc<dyn PostToolExecHook>],
    tool_call: &ToolCall,
    tool_def: &ToolDefinition,
    result: &mut ToolResult,
    context: &ToolContext,
) {
    for hook in hooks {
        hook.after_exec(tool_call, tool_def, result, context).await;
    }
    for hook in final_hooks {
        hook.after_exec(tool_call, tool_def, result, context).await;
    }
}

// ============================================================================
// OutputHardLimitHook (EVE-225)
// ============================================================================

/// Maximum tool result size in bytes before truncation (64 KiB).
/// Large results consume context window, increase cost, and expand the
/// prompt injection surface (TM-AGENT-012).
const MAX_TOOL_RESULT_BYTES: usize = 64 * 1024;

const TRUNCATION_SUFFIX: &str =
    "\n\n[Output truncated — exceeded 64 KiB limit. Try quiet flags, pipes, or redirect to file.]";

/// Infrastructure hook that enforces a hard 64 KiB ceiling on tool result text.
///
/// Always registered as a `final_post_tool_hook` in ActAtom — cannot be removed
/// by capabilities. Runs after all capability-contributed hooks so that
/// persistence hooks (EVE-222) can capture full output before truncation.
///
/// Head-truncation with UTF-8 safety: keeps the first N bytes (on a char
/// boundary) and appends an LLM-actionable suffix.
pub struct OutputHardLimitHook;

impl OutputHardLimitHook {
    /// Truncate `text` to `MAX_TOOL_RESULT_BYTES` with a UTF-8-safe cut.
    fn truncate(text: String) -> String {
        if text.len() <= MAX_TOOL_RESULT_BYTES {
            return text;
        }
        let content_budget = MAX_TOOL_RESULT_BYTES.saturating_sub(TRUNCATION_SUFFIX.len());
        let mut end = content_budget;
        while end > 0 && !text.is_char_boundary(end) {
            end -= 1;
        }
        let mut truncated = text[..end].to_string();
        truncated.push_str(TRUNCATION_SUFFIX);
        truncated
    }
}

#[async_trait]
impl PostToolExecHook for OutputHardLimitHook {
    async fn after_exec(
        &self,
        tool_call: &ToolCall,
        _tool_def: &ToolDefinition,
        result: &mut ToolResult,
        _context: &ToolContext,
    ) {
        // Truncate the result JSON value if it exceeds the limit.
        if let Some(val) = result.result.take() {
            match val {
                serde_json::Value::String(s) => {
                    let original_len = s.len();
                    let truncated = Self::truncate(s);
                    if truncated.len() < original_len {
                        tracing::warn!(
                            tool_name = %tool_call.name,
                            tool_call_id = %tool_call.id,
                            result_bytes = original_len,
                            limit = MAX_TOOL_RESULT_BYTES,
                            "Tool result exceeded hard limit, truncated"
                        );
                    }
                    result.result = Some(serde_json::Value::String(truncated));
                }
                other => {
                    // Non-string JSON: serialize, check size, convert to
                    // truncated string if over limit.
                    let serialized = serde_json::to_string(&other).unwrap_or_default();
                    if serialized.len() > MAX_TOOL_RESULT_BYTES {
                        tracing::warn!(
                            tool_name = %tool_call.name,
                            tool_call_id = %tool_call.id,
                            result_bytes = serialized.len(),
                            limit = MAX_TOOL_RESULT_BYTES,
                            "Tool result exceeded hard limit, truncated"
                        );
                        let truncated = Self::truncate(serialized);
                        result.result = Some(serde_json::Value::String(truncated));
                    } else {
                        result.result = Some(other);
                    }
                }
            }
        }

        // Also cap error messages (unlikely to be huge, but defense in depth).
        if let Some(err) = result.error.take() {
            if err.len() > MAX_TOOL_RESULT_BYTES {
                tracing::warn!(
                    tool_name = %tool_call.name,
                    tool_call_id = %tool_call.id,
                    result_bytes = err.len(),
                    limit = MAX_TOOL_RESULT_BYTES,
                    "Tool error exceeded hard limit, truncated"
                );
            }
            result.error = Some(Self::truncate(err));
        }

        // Cap native image payloads too. These bypass `result.result` JSON size
        // checks and are appended directly as ContentPart::Image later. Enforce
        // both a per-image ceiling (no single image larger than the budget) and
        // a cumulative budget (many smaller images cannot blow past it either).
        if let Some(images) = result.images.as_mut() {
            let original_count = images.len();
            let mut cumulative = 0usize;
            images.retain(|img| {
                let len = img.base64.len();
                if len > MAX_TOOL_RESULT_BYTES {
                    return false;
                }
                match cumulative.checked_add(len) {
                    Some(total) if total <= MAX_TOOL_RESULT_BYTES => {
                        cumulative = total;
                        true
                    }
                    _ => false,
                }
            });
            let dropped = original_count.saturating_sub(images.len());
            if dropped > 0 {
                tracing::warn!(
                    tool_name = %tool_call.name,
                    tool_call_id = %tool_call.id,
                    dropped_images = dropped,
                    kept_images = images.len(),
                    kept_bytes = cumulative,
                    limit = MAX_TOOL_RESULT_BYTES,
                    "Tool images exceeded hard limit and were dropped"
                );
            }
            if images.is_empty() {
                result.images = None;
            }
        }
    }
}

// ============================================================================
// PostActHook trait
// ============================================================================

/// Action a post-act hook wants ActAtom to perform.
#[derive(Debug, Clone)]
pub enum PostActAction {
    /// Emit a `tool.call_requested` event with synthetic client-side tool calls.
    EmitToolCallRequested {
        tool_calls: Vec<ToolCall>,
        tool_definitions: Vec<ToolDefinition>,
    },
}

/// Hook that runs after ActAtom finishes executing tools.
///
/// Hooks inspect the completed results and may:
/// - Set `waiting_for_tool_results` on `ActResult`
/// - Return actions for ActAtom to execute (e.g. emit events)
///
/// Hooks are pure: they return declarative actions rather than
/// touching the event emitter directly. This makes them trivially testable.
pub trait PostActHook: Send + Sync {
    /// Inspect completed results, optionally mutate the result and return actions.
    fn on_completed(
        &self,
        result: &mut ActResult,
        tool_definitions: &[ToolDefinition],
    ) -> Vec<PostActAction>;
}

// ============================================================================
// ConnectionSetupHook
// ============================================================================

/// Hook that detects tools requiring user connection setup and emits
/// synthetic `setup_connection` tool calls so the client can prompt the user.
///
/// When any tool returns `connection_required`, this hook:
/// 1. Sets `waiting_for_tool_results = true` on ActResult
/// 2. Returns a `PostActAction::EmitToolCallRequested` with synthetic tool calls
pub struct ConnectionSetupHook;

impl PostActHook for ConnectionSetupHook {
    fn on_completed(
        &self,
        result: &mut ActResult,
        _tool_definitions: &[ToolDefinition],
    ) -> Vec<PostActAction> {
        let providers: Vec<String> = result
            .results
            .iter()
            .filter_map(|r| r.connection_required.clone())
            .collect();

        if providers.is_empty() {
            return vec![];
        }

        result.waiting_for_tool_results = true;

        let tool_calls: Vec<ToolCall> = providers
            .iter()
            .map(|provider| ToolCall {
                id: format!("setup_conn_{}", Uuid::now_v7()),
                name: "setup_connection".to_string(),
                arguments: json!({ "provider": provider }),
            })
            .collect();

        vec![PostActAction::EmitToolCallRequested {
            tool_calls,
            tool_definitions: vec![],
        }]
    }
}

// ============================================================================
// ClientSideToolHook
// ============================================================================

/// Hook that handles client-side tool calls from the ReasonResult.
///
/// When ActAtom receives tool calls that include client-side tools,
/// those tools are NOT executed (they're filtered out before execution).
/// Instead, this hook emits `tool.call_requested` events so the client
/// can execute them and return results.
///
/// This hook reads client-side tool calls stored on ActResult by ActAtom's
/// partitioning logic, then emits the appropriate event.
pub struct ClientSideToolHook;

impl PostActHook for ClientSideToolHook {
    fn on_completed(
        &self,
        result: &mut ActResult,
        _tool_definitions: &[ToolDefinition],
    ) -> Vec<PostActAction> {
        if result.client_tool_calls.is_empty() {
            return vec![];
        }

        result.waiting_for_tool_results = true;

        vec![PostActAction::EmitToolCallRequested {
            tool_calls: result.client_tool_calls.clone(),
            tool_definitions: result.client_tool_definitions.clone(),
        }]
    }
}

// ============================================================================
// Hook execution helper
// ============================================================================

/// Execute all post-act hooks and apply their actions.
///
/// This is called by ActAtom after tool execution completes. It:
/// 1. Runs each hook to collect actions
/// 2. Emits events for each action
pub(super) async fn run_post_act_hooks<E: EventEmitter>(
    hooks: &[Box<dyn PostActHook>],
    context: &AtomContext,
    result: &mut ActResult,
    tool_definitions: &[ToolDefinition],
    event_emitter: &E,
    locale: Option<&str>,
) {
    for hook in hooks {
        let actions = hook.on_completed(result, tool_definitions);
        for action in actions {
            match action {
                PostActAction::EmitToolCallRequested {
                    tool_calls,
                    tool_definitions: action_defs,
                } => {
                    let event = EventRequest::new(
                        context.session_id,
                        EventContext::from_atom_context(context),
                        ToolCallRequestedData::with_definitions_and_locale(
                            &tool_calls,
                            &action_defs,
                            locale,
                        ),
                    );
                    if let Err(e) = event_emitter.emit(event).await {
                        tracing::warn!(
                            error = %e,
                            "PostActHook: failed to emit tool.call_requested event"
                        );
                    }
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
    use crate::atoms::act::ToolCallResult;
    use crate::tool_types::ToolResult;
    use std::sync::Mutex;

    fn make_tool_call_result(connection_required: Option<&str>) -> ToolCallResult {
        ToolCallResult {
            tool_call: ToolCall {
                id: "call_1".to_string(),
                name: "some_tool".to_string(),
                arguments: json!({}),
            },
            result: ToolResult {
                tool_call_id: "call_1".to_string(),
                result: Some(json!({})),
                images: None,
                error: None,
                connection_required: connection_required.map(|s| s.to_string()),
                raw_output: None,
            },
            success: true,
            status: "success".to_string(),
            connection_required: connection_required.map(|s| s.to_string()),
            determinism_fatal: None,
        }
    }

    #[test]
    fn test_connection_setup_hook_no_connections() {
        let hook = ConnectionSetupHook;
        let mut result = ActResult {
            results: vec![make_tool_call_result(None)],
            completed: true,
            success_count: 1,
            error_count: 0,
            waiting_for_tool_results: false,
            blocked: false,
            client_tool_calls: vec![],
            client_tool_definitions: vec![],
        };

        let actions = hook.on_completed(&mut result, &[]);
        assert!(actions.is_empty());
        assert!(!result.waiting_for_tool_results);
    }

    #[test]
    fn test_connection_setup_hook_with_connection() {
        let hook = ConnectionSetupHook;
        let mut result = ActResult {
            results: vec![make_tool_call_result(Some("github"))],
            completed: true,
            success_count: 0,
            error_count: 0,
            waiting_for_tool_results: false,
            blocked: false,
            client_tool_calls: vec![],
            client_tool_definitions: vec![],
        };

        let actions = hook.on_completed(&mut result, &[]);
        assert_eq!(actions.len(), 1);
        assert!(result.waiting_for_tool_results);

        match &actions[0] {
            PostActAction::EmitToolCallRequested { tool_calls, .. } => {
                assert_eq!(tool_calls.len(), 1);
                assert_eq!(tool_calls[0].name, "setup_connection");
                assert_eq!(tool_calls[0].arguments["provider"], "github");
            }
        }
    }

    #[test]
    fn test_client_side_tool_hook_no_client_tools() {
        let hook = ClientSideToolHook;
        let mut result = ActResult {
            results: vec![],
            completed: true,
            success_count: 0,
            error_count: 0,
            waiting_for_tool_results: false,
            blocked: false,
            client_tool_calls: vec![],
            client_tool_definitions: vec![],
        };

        let actions = hook.on_completed(&mut result, &[]);
        assert!(actions.is_empty());
        assert!(!result.waiting_for_tool_results);
    }

    #[test]
    fn test_client_side_tool_hook_with_client_tools() {
        let hook = ClientSideToolHook;
        let client_call = ToolCall {
            id: "call_client".to_string(),
            name: "browser_click".to_string(),
            arguments: json!({"selector": "#btn"}),
        };

        let mut result = ActResult {
            results: vec![],
            completed: true,
            success_count: 0,
            error_count: 0,
            waiting_for_tool_results: false,
            blocked: false,
            client_tool_calls: vec![client_call.clone()],
            client_tool_definitions: vec![],
        };

        let actions = hook.on_completed(&mut result, &[]);
        assert_eq!(actions.len(), 1);
        assert!(result.waiting_for_tool_results);

        match &actions[0] {
            PostActAction::EmitToolCallRequested { tool_calls, .. } => {
                assert_eq!(tool_calls.len(), 1);
                assert_eq!(tool_calls[0].name, "browser_click");
            }
        }
    }

    // ========================================================================
    // OutputHardLimitHook tests (EVE-225)
    // ========================================================================

    use crate::tool_context::ToolContext;
    use crate::typed_id::SessionId;

    fn make_tool_call() -> ToolCall {
        ToolCall {
            id: "call_test".to_string(),
            name: "test_tool".to_string(),
            arguments: json!({}),
        }
    }

    fn make_tool_def() -> ToolDefinition {
        ToolDefinition::Builtin(crate::tool_types::BuiltinTool {
            name: "test_tool".to_string(),
            display_name: None,
            description: "test".to_string(),
            parameters: json!({}),
            policy: crate::tool_types::ToolPolicy::Auto,
            category: None,
            deferrable: crate::tool_types::DeferrablePolicy::Never,
            hints: Default::default(),
            full_parameters: None,
        })
    }

    struct MarkerHook {
        name: &'static str,
        calls: Arc<Mutex<Vec<&'static str>>>,
    }

    #[async_trait]
    impl PostToolExecHook for MarkerHook {
        async fn after_exec(
            &self,
            _tool_call: &ToolCall,
            _tool_def: &ToolDefinition,
            result: &mut ToolResult,
            _context: &ToolContext,
        ) {
            self.calls.lock().unwrap().push(self.name);
            let value = result
                .result
                .take()
                .and_then(|value| value.as_str().map(str::to_owned))
                .unwrap_or_default();
            result.result = Some(json!(format!("{value}-{}", self.name)));
        }
    }

    #[tokio::test]
    async fn capability_hooks_run_before_runtime_final_hooks() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let capability_hooks: Vec<Arc<dyn PostToolExecHook>> = vec![Arc::new(MarkerHook {
            name: "capability",
            calls: Arc::clone(&calls),
        })];
        let final_hooks: Vec<Arc<dyn PostToolExecHook>> = vec![Arc::new(MarkerHook {
            name: "final",
            calls: Arc::clone(&calls),
        })];
        let mut result = ToolResult {
            tool_call_id: "call_test".into(),
            result: Some(json!("start")),
            images: None,
            error: None,
            connection_required: None,
            raw_output: None,
        };

        run_post_tool_exec_hooks(
            &capability_hooks,
            &final_hooks,
            &make_tool_call(),
            &make_tool_def(),
            &mut result,
            &ToolContext::new(SessionId::new()),
        )
        .await;

        assert_eq!(*calls.lock().unwrap(), ["capability", "final"]);
        assert_eq!(result.result, Some(json!("start-capability-final")));
    }

    #[tokio::test]
    async fn test_output_hard_limit_passthrough_small() {
        let hook = OutputHardLimitHook;
        let tc = make_tool_call();
        let td = make_tool_def();
        let ctx = ToolContext::new(SessionId::new());
        let mut result = ToolResult {
            tool_call_id: "call_test".into(),
            result: Some(json!("hello")),
            images: None,
            error: None,
            connection_required: None,
            raw_output: None,
        };

        hook.after_exec(&tc, &td, &mut result, &ctx).await;
        assert_eq!(result.result, Some(json!("hello")));
    }

    #[tokio::test]
    async fn test_output_hard_limit_truncates_large_string() {
        let hook = OutputHardLimitHook;
        let tc = make_tool_call();
        let td = make_tool_def();
        let ctx = ToolContext::new(SessionId::new());
        let big = "x".repeat(MAX_TOOL_RESULT_BYTES + 1000);
        let mut result = ToolResult {
            tool_call_id: "call_test".into(),
            result: Some(json!(big)),
            images: None,
            error: None,
            connection_required: None,
            raw_output: None,
        };

        hook.after_exec(&tc, &td, &mut result, &ctx).await;

        let text = result.result.unwrap();
        let s = text.as_str().unwrap();
        assert!(s.len() <= MAX_TOOL_RESULT_BYTES);
        assert!(s.ends_with(TRUNCATION_SUFFIX));
    }

    #[tokio::test]
    async fn test_output_hard_limit_at_exact_limit() {
        let hook = OutputHardLimitHook;
        let tc = make_tool_call();
        let td = make_tool_def();
        let ctx = ToolContext::new(SessionId::new());
        let exact = "a".repeat(MAX_TOOL_RESULT_BYTES);
        let mut result = ToolResult {
            tool_call_id: "call_test".into(),
            result: Some(json!(exact)),
            images: None,
            error: None,
            connection_required: None,
            raw_output: None,
        };

        hook.after_exec(&tc, &td, &mut result, &ctx).await;

        let text = result.result.unwrap();
        let s = text.as_str().unwrap();
        // Should NOT be truncated (equal to limit)
        assert_eq!(s.len(), MAX_TOOL_RESULT_BYTES);
        assert!(!s.contains("[Output truncated"));
    }

    #[tokio::test]
    async fn test_output_hard_limit_multibyte_boundary() {
        let hook = OutputHardLimitHook;
        let tc = make_tool_call();
        let td = make_tool_def();
        let ctx = ToolContext::new(SessionId::new());
        let ch = "€"; // 3 bytes
        let count = MAX_TOOL_RESULT_BYTES / ch.len() + 1;
        let big = ch.repeat(count);
        let mut result = ToolResult {
            tool_call_id: "call_test".into(),
            result: Some(json!(big)),
            images: None,
            error: None,
            connection_required: None,
            raw_output: None,
        };

        hook.after_exec(&tc, &td, &mut result, &ctx).await;

        let text = result.result.unwrap();
        let s = text.as_str().unwrap();
        assert!(s.len() <= MAX_TOOL_RESULT_BYTES);
        assert!(s.contains("[Output truncated"));
    }

    #[tokio::test]
    async fn test_output_hard_limit_truncates_error() {
        let hook = OutputHardLimitHook;
        let tc = make_tool_call();
        let td = make_tool_def();
        let ctx = ToolContext::new(SessionId::new());
        let big_err = "e".repeat(MAX_TOOL_RESULT_BYTES + 500);
        let mut result = ToolResult {
            tool_call_id: "call_test".into(),
            result: None,
            images: None,
            error: Some(big_err),
            connection_required: None,
            raw_output: None,
        };

        hook.after_exec(&tc, &td, &mut result, &ctx).await;

        let err = result.error.unwrap();
        assert!(err.len() <= MAX_TOOL_RESULT_BYTES);
        assert!(err.ends_with(TRUNCATION_SUFFIX));
    }

    #[tokio::test]
    async fn test_output_hard_limit_non_string_json() {
        let hook = OutputHardLimitHook;
        let tc = make_tool_call();
        let td = make_tool_def();
        let ctx = ToolContext::new(SessionId::new());
        // Small JSON object — should pass through
        let mut result = ToolResult {
            tool_call_id: "call_test".into(),
            result: Some(json!({"key": "value", "num": 42})),
            images: None,
            error: None,
            connection_required: None,
            raw_output: None,
        };

        hook.after_exec(&tc, &td, &mut result, &ctx).await;

        // Should remain as-is (small non-string JSON)
        assert_eq!(result.result, Some(json!({"key": "value", "num": 42})));
    }

    #[tokio::test]
    async fn test_output_hard_limit_drops_oversized_images() {
        let hook = OutputHardLimitHook;
        let tc = make_tool_call();
        let td = make_tool_def();
        let ctx = ToolContext::new(SessionId::new());

        let mut result = ToolResult {
            tool_call_id: "call_test".into(),
            result: Some(json!({"ok": true})),
            images: Some(vec![
                everruns_provider::ToolResultImage {
                    base64: "a".repeat(32),
                    media_type: "image/png".to_string(),
                },
                everruns_provider::ToolResultImage {
                    base64: "b".repeat(MAX_TOOL_RESULT_BYTES + 1),
                    media_type: "image/png".to_string(),
                },
            ]),
            error: None,
            connection_required: None,
            raw_output: None,
        };

        hook.after_exec(&tc, &td, &mut result, &ctx).await;

        let images = result.images.unwrap();
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].base64.len(), 32);
    }

    #[tokio::test]
    async fn test_output_hard_limit_enforces_cumulative_image_budget() {
        let hook = OutputHardLimitHook;
        let tc = make_tool_call();
        let td = make_tool_def();
        let ctx = ToolContext::new(SessionId::new());

        // Each image is half the limit, so the third one tips the cumulative
        // budget past MAX_TOOL_RESULT_BYTES and must be dropped.
        let half = MAX_TOOL_RESULT_BYTES / 2;
        let mut result = ToolResult {
            tool_call_id: "call_test".into(),
            result: Some(json!({"ok": true})),
            images: Some(vec![
                everruns_provider::ToolResultImage {
                    base64: "a".repeat(half),
                    media_type: "image/png".to_string(),
                },
                everruns_provider::ToolResultImage {
                    base64: "b".repeat(half),
                    media_type: "image/png".to_string(),
                },
                everruns_provider::ToolResultImage {
                    base64: "c".repeat(half),
                    media_type: "image/png".to_string(),
                },
            ]),
            error: None,
            connection_required: None,
            raw_output: None,
        };

        hook.after_exec(&tc, &td, &mut result, &ctx).await;

        let images = result.images.unwrap();
        assert_eq!(
            images.len(),
            2,
            "third image should be dropped by cumulative budget"
        );
        assert!(images.iter().all(|i| i.base64.len() == half));
    }

    #[tokio::test]
    async fn test_output_hard_limit_normalizes_empty_images_to_none() {
        let hook = OutputHardLimitHook;
        let tc = make_tool_call();
        let td = make_tool_def();
        let ctx = ToolContext::new(SessionId::new());

        let mut result = ToolResult {
            tool_call_id: "call_test".into(),
            result: Some(json!({"ok": true})),
            images: Some(vec![everruns_provider::ToolResultImage {
                base64: "a".repeat(MAX_TOOL_RESULT_BYTES + 1),
                media_type: "image/png".to_string(),
            }]),
            error: None,
            connection_required: None,
            raw_output: None,
        };

        hook.after_exec(&tc, &td, &mut result, &ctx).await;

        assert!(
            result.images.is_none(),
            "images vec emptied by retain should normalize to None"
        );
    }

    #[test]
    fn test_truncate_helper_short() {
        let s = "hello".to_string();
        assert_eq!(OutputHardLimitHook::truncate(s.clone()), s);
    }

    #[test]
    fn test_truncate_helper_over() {
        let s = "a".repeat(MAX_TOOL_RESULT_BYTES + 100);
        let t = OutputHardLimitHook::truncate(s);
        assert!(t.len() <= MAX_TOOL_RESULT_BYTES);
        assert!(t.ends_with(TRUNCATION_SUFFIX));
    }
}

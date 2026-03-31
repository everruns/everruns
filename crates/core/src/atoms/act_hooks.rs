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
use crate::traits::{EventEmitter, ToolContext};
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

use super::AtomContext;
use super::act::ActResult;

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
#[async_trait]
pub trait PostToolExecHook: Send + Sync {
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
                        EventContext::turn(context.turn_id, context.input_message_id),
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
}

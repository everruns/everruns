//! ActAtom tests for hard tool-approval deferral (EVE-1140). Kept apart from
//! `act_tests.rs`, which is at the file-size ceiling.

use super::tests::ArgumentEchoTool;
use super::*;
use crate::engine::tools::ToolRegistry;
use crate::engine::typed_id::{AgentId, HarnessId, MessageId, SessionId, TurnId};
use crate::tool_context::ToolContext;
use async_trait::async_trait;
use serde_json::json;

/// Defers every call to `argument_echo` the way the hosted `tool_approval`
/// gate does when nobody has answered yet (EVE-1140).
struct DeferringApprovalHook;

#[async_trait]
impl act_hooks::PreToolUseHook for DeferringApprovalHook {
    async fn before_exec(
        &self,
        tool_call: ToolCall,
        _tool_def: &ToolDefinition,
        _context: &ToolContext,
    ) -> act_hooks::PreToolUseDecision {
        let payload = json!({
            "code": crate::engine::tool_types::TOOL_APPROVAL_REQUIRED_CODE,
            "error": "Waiting for approval",
            "tool_call_id": tool_call.id,
            "tool": tool_call.name,
            "arguments": tool_call.arguments,
            "fingerprint": "sha256:ab",
            "risk": "open_world",
            "mode": "normal",
            "asked_at": "2026-10-01T00:00:00Z",
            "expires_at": "2026-10-01T00:15:00Z",
        });
        act_hooks::PreToolUseDecision::Defer {
            result: ToolResult {
                tool_call_id: String::new(),
                result: Some(payload),
                images: None,
                error: Some("Waiting for approval".to_string()),
                connection_required: None,
                raw_output: None,
            },
            tool_call,
        }
    }
}

#[tokio::test]
async fn test_act_atom_parks_a_deferred_approval_without_running_the_tool() {
    let mut executor = ToolRegistry::new();
    executor.register(ArgumentEchoTool);
    let tool_def = executor.get("argument_echo").unwrap().to_definition();
    let emitter = crate::engine::test_fixtures::TestEventEmitter::new();
    let atom = ActAtom::new(executor, emitter.clone())
        .with_pre_tool_hooks(vec![std::sync::Arc::new(DeferringApprovalHook)]);

    let context = ExecutionContext::new(SessionId::new(), TurnId::new(), MessageId::new());
    let input = ActInput {
        org_id: Some(1),
        context,
        harness_id: HarnessId::from_seed(1),
        agent_id: Some(AgentId::new()),
        tool_calls: vec![ToolCall {
            id: "call_1".to_string(),
            name: "argument_echo".to_string(),
            arguments: json!({ "value": "send it" }),
        }],
        tool_definitions: vec![tool_def],
        locale: None,
        blueprint_id: None,
        network_access: None,
        parallel_tool_calls: None,
    };

    let result = atom.execute(input).await.unwrap();

    // The tool did not run: the recorded outcome is the gate's, not an echo.
    assert_eq!(result.results.len(), 1);
    assert!(!result.results[0].success);
    assert_eq!(result.results[0].result.tool_call_id, "call_1");
    assert!(
        crate::engine::tool_types::ToolApprovalRequired::from_tool_result(
            &result.results[0].result
        )
        .is_some()
    );

    // The act parks on one synthetic request derived from the gated call.
    assert!(result.waiting_for_tool_results);
    assert!(crate::engine::execution::has_pending_tool_approval(
        &result.client_tool_calls
    ));
    assert_eq!(result.client_tool_calls.len(), 1);
    assert_eq!(result.client_tool_calls[0].id, "tool_approval_call_1");
    assert_eq!(
        result.client_tool_calls[0].name,
        crate::engine::tool_types::APPROVE_TOOL_CALL_TOOL
    );

    let events = emitter.events().await;
    let requested = events
        .iter()
        .find(|event| event.event_type == "tool.call_requested")
        .expect("tool.call_requested event");
    let crate::engine::events::EventData::ToolCallRequested(data) = &requested.data else {
        panic!("expected tool.call_requested data");
    };
    assert_eq!(data.tool_calls.len(), 1);
    assert_eq!(data.tool_calls[0].id, "tool_approval_call_1");
}

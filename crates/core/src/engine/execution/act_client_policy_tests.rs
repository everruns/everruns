//! Pre-tool policy on client-side calls. A denial must never become
//! `tool.call_requested`; an approval must keep the hook's arguments.

use super::tests::ArgumentEchoTool;
use super::*;
use crate::engine::tool_context::ToolContext;
use crate::engine::tools::ToolRegistry;
use crate::engine::typed_id::{AgentId, HarnessId, MessageId, SessionId, TurnId};
use async_trait::async_trait;
use everruns_contracts::ClientSideTool;
use serde_json::json;

struct DenyNamedHook {
    name: &'static str,
}

#[async_trait]
impl act_hooks::PreToolUseHook for DenyNamedHook {
    async fn before_exec(
        &self,
        tool_call: ToolCall,
        _tool_def: &ToolDefinition,
        _context: &ToolContext,
    ) -> act_hooks::PreToolUseDecision {
        if tool_call.name == self.name {
            return act_hooks::PreToolUseDecision::Block {
                reason: "sensitive client capability".to_string(),
                user_message: None,
                tool_call,
            };
        }
        act_hooks::PreToolUseDecision::Continue(tool_call)
    }
}

struct RewriteNamedHook {
    name: &'static str,
}

#[async_trait]
impl act_hooks::PreToolUseHook for RewriteNamedHook {
    async fn before_exec(
        &self,
        mut tool_call: ToolCall,
        _tool_def: &ToolDefinition,
        _context: &ToolContext,
    ) -> act_hooks::PreToolUseDecision {
        if tool_call.name == self.name
            && let Some(arguments) = tool_call.arguments.as_object_mut()
        {
            arguments.insert("value".to_string(), json!("rewritten"));
        }
        act_hooks::PreToolUseDecision::Continue(tool_call)
    }
}

fn client_tool(name: &str) -> ToolDefinition {
    ToolDefinition::ClientSide(ClientSideTool::new(
        name,
        "Runs in the integrating client",
        json!({
            "type": "object",
            "properties": { "value": { "type": "string" } }
        }),
    ))
}

fn call(id: &str, name: &str, value: &str) -> ToolCall {
    ToolCall {
        id: id.to_string(),
        name: name.to_string(),
        arguments: json!({ "value": value }),
    }
}

fn act_input(tool_calls: Vec<ToolCall>, tool_definitions: Vec<ToolDefinition>) -> ActInput {
    ActInput {
        org_id: Some(1),
        context: ExecutionContext::new(SessionId::new(), TurnId::new(), MessageId::new()),
        harness_id: HarnessId::from_seed(1),
        agent_id: Some(AgentId::new()),
        tool_calls,
        tool_definitions,
        locale: None,
        blueprint_id: None,
        network_access: None,
        parallel_tool_calls: None,
    }
}

fn requested_calls(events: &[crate::engine::events::Event]) -> Vec<ToolCall> {
    events
        .iter()
        .filter(|event| event.event_type == "tool.call_requested")
        .flat_map(|event| match &event.data {
            crate::engine::events::EventData::ToolCallRequested(data) => data.tool_calls.clone(),
            _ => Vec::new(),
        })
        .collect()
}

#[tokio::test]
async fn denied_client_call_emits_no_execution_request() {
    let emitter = crate::engine::test_fixtures::TestEventEmitter::new();
    let atom = ActAtom::new(ToolRegistry::new(), emitter.clone()).with_pre_tool_hooks(vec![
        std::sync::Arc::new(DenyNamedHook {
            name: "browser_click",
        }),
    ]);

    let result = atom
        .execute(act_input(
            vec![call("call_client", "browser_click", "secret")],
            vec![client_tool("browser_click")],
        ))
        .await
        .unwrap();

    assert!(
        requested_calls(&emitter.events().await).is_empty(),
        "a denied client call must not be sent to the integrating client"
    );
    assert!(!result.waiting_for_tool_results);
    assert!(result.client_tool_calls.is_empty());
    assert_eq!(result.results.len(), 1);
    assert!(!result.results[0].success);
    assert_eq!(result.results[0].result.tool_call_id, "call_client");
    let error = result.results[0]
        .result
        .error
        .as_deref()
        .unwrap_or_default();
    assert!(
        error.contains("blocked by pre_tool_use hook"),
        "model-visible error was {error}"
    );
    assert!(
        emitter
            .events()
            .await
            .iter()
            .any(|event| event.event_type == "tool.completed"),
        "the denial has to reach the model as a tool result"
    );
}

#[tokio::test]
async fn approved_client_call_keeps_transformed_arguments() {
    let emitter = crate::engine::test_fixtures::TestEventEmitter::new();
    let atom = ActAtom::new(ToolRegistry::new(), emitter.clone()).with_pre_tool_hooks(vec![
        std::sync::Arc::new(RewriteNamedHook {
            name: "browser_type",
        }),
    ]);

    let result = atom
        .execute(act_input(
            vec![call("call_type", "browser_type", "original")],
            vec![client_tool("browser_type")],
        ))
        .await
        .unwrap();

    assert_eq!(result.client_tool_calls.len(), 1);
    assert_eq!(
        result.client_tool_calls[0].arguments,
        json!({ "value": "rewritten" })
    );
    let requested = requested_calls(&emitter.events().await);
    assert_eq!(requested.len(), 1);
    assert_eq!(requested[0].id, "call_type");
    assert_eq!(requested[0].arguments, json!({ "value": "rewritten" }));
    assert!(result.waiting_for_tool_results);
}

#[tokio::test]
async fn mixed_batch_gates_client_calls_before_the_request() {
    let mut executor = ToolRegistry::new();
    executor.register(ArgumentEchoTool);
    let server_def = executor.get("argument_echo").unwrap().to_definition();
    let emitter = crate::engine::test_fixtures::TestEventEmitter::new();
    let atom = ActAtom::new(executor, emitter.clone()).with_pre_tool_hooks(vec![
        std::sync::Arc::new(DenyNamedHook {
            name: "browser_click",
        }),
        std::sync::Arc::new(RewriteNamedHook {
            name: "browser_type",
        }),
    ]);

    let result = atom
        .execute(act_input(
            vec![
                call("call_server", "argument_echo", "server"),
                call("call_denied", "browser_click", "secret"),
                call("call_ok", "browser_type", "original"),
            ],
            vec![
                server_def,
                client_tool("browser_click"),
                client_tool("browser_type"),
            ],
        ))
        .await
        .unwrap();

    let echo = result
        .results
        .iter()
        .find(|r| r.tool_call.id == "call_server")
        .expect("server tool result");
    assert!(echo.success);

    let denied = result
        .results
        .iter()
        .find(|r| r.tool_call.id == "call_denied")
        .expect("denied client call is a tool result");
    assert!(!denied.success);

    assert_eq!(result.client_tool_calls.len(), 1);
    assert_eq!(result.client_tool_calls[0].id, "call_ok");
    assert_eq!(
        result.client_tool_calls[0].arguments,
        json!({ "value": "rewritten" })
    );

    let requested = requested_calls(&emitter.events().await);
    assert_eq!(requested.len(), 1);
    assert_eq!(requested[0].id, "call_ok");
    assert_eq!(requested[0].arguments, json!({ "value": "rewritten" }));
    assert!(requested.iter().all(|call| call.id != "call_denied"));
}

#[tokio::test]
async fn deferred_client_call_requests_approval_instead_of_the_tool() {
    let emitter = crate::engine::test_fixtures::TestEventEmitter::new();
    let atom = ActAtom::new(ToolRegistry::new(), emitter.clone())
        .with_pre_tool_hooks(vec![std::sync::Arc::new(DeferClientHook)]);

    let result = atom
        .execute(act_input(
            vec![call("call_client", "browser_click", "secret")],
            vec![client_tool("browser_click")],
        ))
        .await
        .unwrap();

    assert!(result.waiting_for_tool_results);
    assert!(crate::engine::execution::has_pending_tool_approval(
        &result.client_tool_calls
    ));
    let requested = requested_calls(&emitter.events().await);
    assert!(
        requested.iter().all(|call| call.name != "browser_click"),
        "the deferred client tool must not be sent for execution"
    );
    assert!(
        requested
            .iter()
            .any(|call| call.name == crate::engine::tool_types::APPROVE_TOOL_CALL_TOOL)
    );
}

struct DeferClientHook;

#[async_trait]
impl act_hooks::PreToolUseHook for DeferClientHook {
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

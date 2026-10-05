//! `tool.completed` records the arguments a call ran with when `pre_tool_use`
//! hooks rewrote the model-authored ones (EVE-1209). Kept apart from
//! `act_tests.rs`, which is at the file-size ceiling.

use super::tests::{ArgumentEchoTool, HumanIntentFixtureHook};
use super::*;
use crate::engine::events::EventData;
use crate::engine::tools::ToolRegistry;
use crate::engine::typed_id::{AgentId, HarnessId, MessageId, SessionId, TurnId};
use crate::tool_context::ToolContext;
use async_trait::async_trait;
use serde_json::{Value, json};

/// Replaces `value` with a fixed one, as a user `pre_tool_use` hook can.
struct RewritingHook(Value);

#[async_trait]
impl act_hooks::PreToolUseHook for RewritingHook {
    async fn before_exec(
        &self,
        mut tool_call: ToolCall,
        _tool_def: &ToolDefinition,
        _context: &ToolContext,
    ) -> act_hooks::PreToolUseDecision {
        tool_call.arguments["value"] = self.0.clone();
        act_hooks::PreToolUseDecision::Continue(tool_call)
    }
}

/// Blocks every call, after an earlier hook may have rewritten it.
struct BlockingHook;

#[async_trait]
impl act_hooks::PreToolUseHook for BlockingHook {
    async fn before_exec(
        &self,
        tool_call: ToolCall,
        _tool_def: &ToolDefinition,
        _context: &ToolContext,
    ) -> act_hooks::PreToolUseDecision {
        act_hooks::PreToolUseDecision::Block {
            tool_call,
            reason: "not allowed".to_string(),
            user_message: None,
        }
    }
}

/// Runs one `argument_echo` call through the act and returns the arguments
/// `tool.started` carried, the `tool.completed` payload, and the tool result.
async fn run_echo(
    hooks: Vec<std::sync::Arc<dyn act_hooks::PreToolUseHook>>,
    arguments: Value,
) -> (Value, ToolCompletedData, Option<Value>) {
    let mut executor = ToolRegistry::new();
    executor.register(ArgumentEchoTool);
    let tool_def = executor.get("argument_echo").unwrap().to_definition();
    let emitter = crate::engine::test_fixtures::TestEventEmitter::new();
    let atom = ActAtom::new(executor, emitter.clone())
        .with_tool_call_hooks(vec![std::sync::Arc::new(HumanIntentFixtureHook)])
        .with_pre_tool_hooks(hooks);
    let input = ActInput {
        org_id: Some(1),
        context: ExecutionContext::new(SessionId::new(), TurnId::new(), MessageId::new()),
        harness_id: HarnessId::from_seed(1),
        agent_id: Some(AgentId::new()),
        tool_calls: vec![ToolCall {
            id: "call_1".to_string(),
            name: "argument_echo".to_string(),
            arguments,
        }],
        tool_definitions: vec![tool_def],
        locale: None,
        blueprint_id: None,
        network_access: None,
        parallel_tool_calls: None,
    };
    let result = atom.execute(input).await.unwrap();

    let events = emitter.events().await;
    let started = events
        .iter()
        .find_map(|event| match &event.data {
            EventData::ToolStarted(data) => Some(data.tool_call.arguments.clone()),
            _ => None,
        })
        .expect("tool.started event");
    let completed = events
        .iter()
        .find_map(|event| match &event.data {
            EventData::ToolCompleted(data) => Some(data.clone()),
            _ => None,
        })
        .expect("tool.completed event");
    (started, completed, result.results[0].result.result.clone())
}

#[tokio::test]
async fn completed_event_records_hook_rewritten_arguments_next_to_the_original() {
    let (started, completed, ran_with) = run_echo(
        vec![std::sync::Arc::new(RewritingHook(json!(
            "destroy everything"
        )))],
        json!({ "value": "benign", "human_intent": "Echoing" }),
    )
    .await;

    // The tool really ran with the rewrite.
    assert_eq!(ran_with, Some(json!({ "value": "destroy everything" })));
    // tool.started keeps the model-authored call; tool.completed says what ran.
    assert_eq!(
        started,
        json!({ "value": "benign", "human_intent": "Echoing" })
    );
    assert!(completed.success);
    assert_eq!(
        completed.executed_arguments,
        Some(json!({ "value": "destroy everything" }))
    );
    assert!(!completed.executed_arguments_truncated);
    let wire = serde_json::to_value(&completed).unwrap();
    assert_eq!(
        wire["executed_arguments"],
        json!({ "value": "destroy everything" })
    );
    assert!(wire.get("executed_arguments_truncated").is_none());
}

#[tokio::test]
async fn completed_event_is_unchanged_when_no_hook_rewrites_the_call() {
    // Stripping `human_intent` for execution is not a hook rewrite.
    let (_, completed, _) = run_echo(
        vec![std::sync::Arc::new(RewritingHook(json!("benign")))],
        json!({ "value": "benign", "human_intent": "Echoing" }),
    )
    .await;

    assert!(completed.success);
    assert_eq!(completed.executed_arguments, None);
    let wire = serde_json::to_value(&completed).unwrap();
    assert!(wire.get("executed_arguments").is_none());
    assert!(wire.get("executed_arguments_truncated").is_none());
}

#[tokio::test]
async fn blocked_call_records_no_executed_arguments() {
    let (_, completed, _) = run_echo(
        vec![
            std::sync::Arc::new(RewritingHook(json!("destroy everything"))),
            std::sync::Arc::new(BlockingHook),
        ],
        json!({ "value": "benign" }),
    )
    .await;

    assert!(!completed.success);
    assert_eq!(completed.executed_arguments, None);
}

#[tokio::test]
async fn large_rewritten_arguments_are_bounded_like_the_approval_preview() {
    let (_, completed, _) = run_echo(
        vec![std::sync::Arc::new(RewritingHook(json!(
            "x".repeat(20_000)
        )))],
        json!({ "value": "benign" }),
    )
    .await;

    assert!(completed.executed_arguments_truncated);
    let preview = completed.executed_arguments.expect("executed arguments");
    assert!(
        preview
            .as_str()
            .expect("truncated preview is a string")
            .len()
            <= crate::engine::tool_types::TOOL_ARGUMENTS_PREVIEW_BYTES
    );
}

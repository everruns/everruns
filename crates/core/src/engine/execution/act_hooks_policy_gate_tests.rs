//! Policy gates decide on the final call (EVE-1184). Kept apart from the
//! inline `act_hooks` tests, which are near the file-size ceiling.

use super::*;
use crate::engine::tool_types::BuiltinTool;
use crate::tool_hooks::PolicyGate;
use std::sync::Mutex;

fn tool_def() -> ToolDefinition {
    ToolDefinition::Builtin(BuiltinTool {
        name: "shell".to_string(),
        display_name: None,
        description: String::new(),
        parameters: json!({}),
        policy: Default::default(),
        category: None,
        deferrable: Default::default(),
        hints: Default::default(),
        full_parameters: None,
    })
}

fn call(command: &str) -> ToolCall {
    ToolCall {
        id: "call_1".to_string(),
        name: "shell".to_string(),
        arguments: json!({ "command": command }),
    }
}

/// Rewrites `command` to a fixed value.
struct Rewrite(&'static str);

#[async_trait]
impl PreToolUseHook for Rewrite {
    async fn before_exec(
        &self,
        mut tool_call: ToolCall,
        _tool_def: &ToolDefinition,
        _context: &ToolContext,
    ) -> PreToolUseDecision {
        tool_call.arguments = json!({ "command": self.0 });
        PreToolUseDecision::Continue(tool_call)
    }
}

/// Blocks any `command` containing `rm`, and records what it decided on.
#[derive(Default)]
struct DenyRm {
    seen: Mutex<Vec<String>>,
}

#[async_trait]
impl PreToolUseHook for DenyRm {
    async fn before_exec(
        &self,
        tool_call: ToolCall,
        _tool_def: &ToolDefinition,
        _context: &ToolContext,
    ) -> PreToolUseDecision {
        let command = tool_call.arguments["command"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        self.seen.lock().unwrap().push(command.clone());
        if command.contains("rm") {
            PreToolUseDecision::Block {
                tool_call,
                reason: "rm is denied".to_string(),
                user_message: None,
            }
        } else {
            PreToolUseDecision::Continue(tool_call)
        }
    }
}

/// Forwards to a shared `DenyRm` so a test can inspect it after the chain.
struct SharedGate(Arc<DenyRm>);

#[async_trait]
impl PreToolUseHook for SharedGate {
    async fn before_exec(
        &self,
        tool_call: ToolCall,
        tool_def: &ToolDefinition,
        context: &ToolContext,
    ) -> PreToolUseDecision {
        self.0.before_exec(tool_call, tool_def, context).await
    }
}

async fn run(hooks: Vec<Arc<dyn PreToolUseHook>>, command: &str) -> PreToolUseDecision {
    let context = ToolContext::new(crate::engine::typed_id::SessionId::new());
    run_pre_tool_use_hooks(&hooks, call(command), &tool_def(), &context).await
}

#[tokio::test]
async fn a_gate_declared_first_still_decides_on_the_rewritten_call() {
    let gate = Arc::new(DenyRm::default());
    let decision = run(
        vec![
            Arc::new(PolicyGate(SharedGate(gate.clone()))),
            Arc::new(Rewrite("rm -rf /")),
        ],
        "ls",
    )
    .await;

    assert!(
        matches!(decision, PreToolUseDecision::Block { .. }),
        "{decision:?}"
    );
    assert_eq!(*gate.seen.lock().unwrap(), vec!["rm -rf /".to_string()]);
}

#[tokio::test]
async fn safe_rewrites_reach_the_gate_and_run() {
    let gate = Arc::new(DenyRm::default());
    let decision = run(
        vec![
            Arc::new(PolicyGate(SharedGate(gate.clone()))),
            Arc::new(Rewrite("ls -la")),
        ],
        "ls",
    )
    .await;

    let PreToolUseDecision::Continue(final_call) = decision else {
        panic!("expected Continue, got {decision:?}");
    };
    assert_eq!(final_call.arguments, json!({ "command": "ls -la" }));
    assert_eq!(*gate.seen.lock().unwrap(), vec!["ls -la".to_string()]);
}

#[tokio::test]
async fn a_transform_block_stops_the_chain_before_any_gate() {
    let gate = Arc::new(DenyRm::default());
    let decision = run(
        vec![
            Arc::new(PolicyGate(SharedGate(gate.clone()))),
            Arc::new(DenyRm::default()),
        ],
        "rm x",
    )
    .await;

    assert!(matches!(decision, PreToolUseDecision::Block { .. }));
    assert!(gate.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn gates_decide_again_when_a_gate_rewrites_the_call() {
    let first = Arc::new(DenyRm::default());
    let decision = run(
        vec![
            Arc::new(PolicyGate(SharedGate(first.clone()))),
            Arc::new(PolicyGate(Rewrite("rm -rf /"))),
        ],
        "ls",
    )
    .await;

    assert!(
        matches!(decision, PreToolUseDecision::Block { .. }),
        "{decision:?}"
    );
    assert_eq!(
        *first.seen.lock().unwrap(),
        vec!["ls".to_string(), "rm -rf /".to_string()]
    );
}

/// Rewrites to a different value on every call, so gates never settle.
#[derive(Default)]
struct Unsettled(Mutex<usize>);

#[async_trait]
impl PreToolUseHook for Unsettled {
    async fn before_exec(
        &self,
        mut tool_call: ToolCall,
        _tool_def: &ToolDefinition,
        _context: &ToolContext,
    ) -> PreToolUseDecision {
        let mut count = self.0.lock().unwrap();
        *count += 1;
        tool_call.arguments = json!({ "command": format!("echo {count}") });
        PreToolUseDecision::Continue(tool_call)
    }
}

#[tokio::test]
async fn gates_that_never_settle_fail_closed() {
    let decision = run(vec![Arc::new(PolicyGate(Unsettled::default()))], "ls").await;

    let PreToolUseDecision::Block { reason, .. } = decision else {
        panic!("expected Block, got {decision:?}");
    };
    assert!(reason.contains("did not settle"), "{reason}");
}

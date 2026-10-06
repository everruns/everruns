//! Code mode runs each nested call through the turn's tool policy (EVE-1210).

use crate::lua::LuaTool;
use crate::lua::tests::EmptyFileStore;
use crate::lua::tools::{Tool, ToolExecutionResult};
use crate::lua::typed_id::SessionId;
use async_trait::async_trait;
use everruns_contracts::runtime::tool_context::ToolContext;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

/// Stands in for the act phase's chains (EVE-1210): blocks `blocked`
/// for one tool name, rewrites the arguments of every other call, and
/// records what the post-tool chain saw.
pub(crate) struct ScriptedPolicy {
    blocked_tool: &'static str,
    after_exec_seen: Mutex<Vec<(String, Value)>>,
}

impl ScriptedPolicy {
    pub(crate) fn blocking(tool: &'static str) -> Arc<Self> {
        Arc::new(Self {
            blocked_tool: tool,
            after_exec_seen: Mutex::new(Vec::new()),
        })
    }
}

#[async_trait]
impl crate::lua::tool_hooks::NestedToolPolicy for ScriptedPolicy {
    async fn authorize(
        &self,
        mut tool_call: crate::lua::tool_types::ToolCall,
        _tool_def: &crate::lua::tool_types::ToolDefinition,
        _context: &ToolContext,
    ) -> Result<crate::lua::tool_types::ToolCall, crate::lua::tool_types::ToolResult> {
        if tool_call.name == self.blocked_tool {
            return Err(crate::lua::tool_types::ToolResult {
                tool_call_id: tool_call.id.clone(),
                result: None,
                images: None,
                error: Some("blocked by pre_tool_use hook: no echo".to_string()),
                connection_required: None,
                raw_output: None,
            });
        }
        tool_call.arguments["rewritten"] = json!(true);
        Ok(tool_call)
    }

    async fn after_exec(
        &self,
        tool_call: &crate::lua::tool_types::ToolCall,
        _tool_def: &crate::lua::tool_types::ToolDefinition,
        result: &mut crate::lua::tool_types::ToolResult,
        _context: &ToolContext,
    ) {
        self.after_exec_seen.lock().unwrap().push((
            tool_call.name.clone(),
            result.result.clone().unwrap_or(Value::Null),
        ));
        if let Some(Value::Object(map)) = result.result.as_mut() {
            map.insert("post_hook".to_string(), json!(true));
        }
    }
}

fn code_mode_ctx(policy: Option<Arc<ScriptedPolicy>>) -> ToolContext {
    let mut registry = crate::lua::tools::ToolRegistry::new();
    registry.register(EchoTool);
    registry.register(StrictTool);
    let mut ctx = ToolContext::new(SessionId::new());
    ctx.file_store = Some(Arc::new(EmptyFileStore));
    ctx.tool_registry = Some(Arc::new(registry));
    ctx.tool_call_id = Some("call_outer".to_string());
    if let Some(policy) = policy {
        ctx = ctx.with_nested_tool_policy(policy);
    }
    ctx
}

#[tokio::test]
async fn code_mode_nested_call_denied_by_policy_does_not_run() {
    // THREAT[TM-LUA-009]: a call the pre-tool chain blocks when made
    // directly is blocked from code mode too (EVE-1210).
    let policy = ScriptedPolicy::blocking("echo");
    let ctx = code_mode_ctx(Some(policy.clone()));
    let result = LuaTool
        .execute_with_context(json!({ "script": "return tools.echo({ n = 1 })" }), &ctx)
        .await;
    assert!(
        matches!(&result, ToolExecutionResult::ToolError(msg) if msg.contains("blocked by pre_tool_use hook")),
        "expected the policy block, got {result:?}"
    );
    assert!(
        policy.after_exec_seen.lock().unwrap().is_empty(),
        "a blocked call never executed, so nothing reaches the post chain"
    );
}

#[tokio::test]
async fn code_mode_runs_hook_rewritten_arguments_and_post_hooks_see_result() {
    let policy = ScriptedPolicy::blocking("nothing");
    let ctx = code_mode_ctx(Some(policy.clone()));
    let v = run_with_ctx("return tools.echo({ n = 5 })", &ctx).await;
    assert_eq!(v["result"]["n"], json!(5));
    assert_eq!(
        v["result"]["rewritten"],
        json!(true),
        "the hook-rewritten arguments are what ran"
    );
    assert_eq!(
        v["result"]["post_hook"],
        json!(true),
        "the post chain's edit is what the script gets"
    );
    let seen = policy.after_exec_seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].0, "echo");
    assert_eq!(seen[0].1["rewritten"], json!(true));
}

#[tokio::test]
async fn code_mode_refuses_without_the_turn_policy() {
    let ctx = code_mode_ctx(None);
    let result = LuaTool
        .execute_with_context(json!({ "script": "return tools.echo({ n = 1 })" }), &ctx)
        .await;
    assert!(
        matches!(&result, ToolExecutionResult::ToolError(msg) if msg.contains("tool policy")),
        "expected a refusal without the policy, got {result:?}"
    );
}

#[tokio::test]
async fn code_mode_validates_arguments_against_target_schema() {
    let policy = ScriptedPolicy::blocking("nothing");
    let ctx = code_mode_ctx(Some(policy.clone()));
    let result = LuaTool
        .execute_with_context(
            json!({ "script": r#"return tools.strict({ count = "many" })"# }),
            &ctx,
        )
        .await;
    assert!(
        matches!(&result, ToolExecutionResult::ToolError(msg) if msg.contains("invalid_tool_arguments")),
        "expected a schema refusal, got {result:?}"
    );
    assert!(policy.after_exec_seen.lock().unwrap().is_empty());
}

/// Requires an integer `count`; succeeds only when given one.
struct StrictTool;

#[async_trait]
impl Tool for StrictTool {
    fn name(&self) -> &str {
        "strict"
    }
    fn description(&self) -> &str {
        "Takes an integer count."
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "count": { "type": "integer" } },
            "required": ["count"]
        })
    }
    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::success(arguments)
    }
}

/// Echoes its JSON argument back as the result.
struct EchoTool;

#[async_trait]
impl Tool for EchoTool {
    fn name(&self) -> &str {
        "echo"
    }
    fn description(&self) -> &str {
        "Echo the argument."
    }
    fn parameters_schema(&self) -> Value {
        json!({ "type": "object" })
    }
    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::success(arguments)
    }
}

async fn run_with_ctx(script: &str, ctx: &ToolContext) -> Value {
    match LuaTool
        .execute_with_context(json!({ "script": script }), ctx)
        .await
    {
        ToolExecutionResult::Success(v) => v,
        other => panic!("expected success, got {other:?}"),
    }
}

#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use everruns_contracts::{
    runtime::{tool_hooks::NestedToolPolicy, tools::ToolRegistry},
    tool_types::{ToolCall, ToolDefinition, ToolResult},
    typed_id::SessionId,
};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

fn private_directory() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    dir
}
fn context() -> ToolContext {
    ToolContext::new(SessionId::new())
}

struct Echo {
    calls: Arc<AtomicUsize>,
}
#[async_trait]
impl Tool for Echo {
    fn name(&self) -> &str {
        "echo"
    }
    fn description(&self) -> &str {
        "Return the structured argument"
    }
    fn parameters_schema(&self) -> Value {
        json!({"type":"object","properties":{"value":{"type":"integer"}},"required":["value"],"additionalProperties":false})
    }
    async fn execute(&self, _: Value) -> ToolExecutionResult {
        panic!("must receive context")
    }
    async fn execute_with_context(&self, args: Value, ctx: &ToolContext) -> ToolExecutionResult {
        assert!(ctx.tool_registry.is_none());
        assert!(ctx.idempotency_key.is_some());
        self.calls.fetch_add(1, Ordering::SeqCst);
        ToolExecutionResult::success(args)
    }
}
struct Policy {
    approved: AtomicBool,
    rewrite: bool,
}
#[async_trait]
impl NestedToolPolicy for Policy {
    async fn authorize(
        &self,
        mut call: ToolCall,
        _: &ToolDefinition,
        _: &ToolContext,
    ) -> Result<ToolCall, ToolResult> {
        if !self.approved.load(Ordering::SeqCst) && call.arguments["value"] == 2 {
            return Err(ToolExecutionResult::success(json!({"code":"tool_approval_required", "error":"Approval required", "tool_call_id":call.id,"tool":call.name,"arguments":call.arguments,"fingerprint":"test-fingerprint","risk":"policy","mode":"normal","asked_at":"2026-11-25T00:00:00Z","expires_at":"2026-11-26T00:00:00Z"})).into_tool_result(&call.id, &call.name));
        }
        if self.rewrite {
            call.arguments["value"] = json!(7);
        }
        Ok(call)
    }
    async fn after_exec(
        &self,
        _: &ToolCall,
        _: &ToolDefinition,
        result: &mut ToolResult,
        _: &ToolContext,
    ) {
        if let Some(value) = result.result.as_mut() {
            value["post_policy"] = json!(true);
        }
    }
}
fn with_tools(
    mut ctx: ToolContext,
    approved: bool,
) -> (ToolContext, Arc<AtomicUsize>, Arc<Policy>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut registry = ToolRegistry::new();
    registry.register(Echo {
        calls: calls.clone(),
    });
    registry.register(PythonTool::new(MontyConfig::default()));
    let policy = Arc::new(Policy {
        approved: AtomicBool::new(approved),
        rewrite: false,
    });
    ctx.tool_registry = Some(Arc::new(registry));
    ctx.visible_tool_names = Some(Arc::new(["echo".to_string(), "python".to_string()].into()));
    ctx.nested_tool_policy = Some(policy.clone());
    ctx.tool_call_id = Some("outer-python".into());
    ctx.idempotency_key = Some("outer-python-idempotency".into());
    (ctx, calls, policy)
}

#[test]
fn values_are_structured_and_lossless() {
    let json = json!({"a":[1,true,null,"β"],"b":1.25});
    assert_eq!(
        value::from_python(value::to_python(&json).unwrap().as_ref()).unwrap(),
        json
    );
    assert!(value::to_python(&json!(u64::MAX)).is_err());
    assert!(value::from_python(MontyObject::float(f64::NAN).as_ref()).is_err());
    assert!(
        value::from_python(
            MontyObject::dict([(MontyObject::int(1), MontyObject::none())]).as_ref()
        )
        .is_err()
    );
}
#[test]
fn values_output_and_tokens_are_bounded() {
    assert!(
        value::from_python(MontyObject::string("a".repeat(value::MAX_VALUE_BYTES + 1)).as_ref())
            .is_err()
    );
    let mut output = "a".repeat(MAX_OUTPUT - 1);
    append_output(&mut output, "ββ");
    assert_eq!(output.len(), MAX_OUTPUT - 1);
    for token in ["", "../secret:0", "a:1", &format!("{}:129", "a".repeat(64))] {
        assert!(parse_token(token).is_err());
    }
}
#[test]
fn discovery_and_execution_share_turn_authority() {
    let (mut ctx, _, _) = with_tools(context(), true);
    let catalog = nested::catalog(&ctx, "").unwrap();
    assert_eq!(catalog["tools"].as_array().unwrap().len(), 1);
    assert_eq!(catalog["tools"][0]["name"], "echo");
    ctx.visible_tool_names = None;
    assert!(
        nested::catalog(&ctx, "").unwrap()["tools"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
#[tokio::test]
async fn journal_is_private_and_persistent() {
    let dir = private_directory();
    let ctx = context();
    let j = journal::resolve(&ctx, Some(dir.path())).unwrap();
    journal::save(&j, "checkpoint", json!({"snapshot":"private"}))
        .await
        .unwrap();
    let reopened = journal::resolve(&ctx, Some(dir.path())).unwrap();
    assert_eq!(
        journal::load(&reopened, "checkpoint").await.unwrap(),
        Some(json!({"snapshot":"private"}))
    );
    assert!(
        journal::load(
            &journal::resolve(&context(), Some(dir.path())).unwrap(),
            "checkpoint"
        )
        .await
        .unwrap()
        .is_none()
    );
    assert!(
        journal::save(&j, "checkpoint", json!({"snapshot":"tampered"}))
            .await
            .is_err()
    );
}
#[tokio::test]
async fn nested_call_runs_policies_and_replays_without_repeating_effect() {
    let dir = private_directory();
    let (mut ctx, calls, _) = with_tools(context(), true);
    ctx.nested_tool_policy = Some(Arc::new(Policy {
        approved: AtomicBool::new(true),
        rewrite: true,
    }));
    let j = journal::resolve(&ctx, Some(dir.path())).unwrap();
    for _ in 0..2 {
        let nested::Outcome::Result(value) =
            nested::invoke(&ctx, &j, "effect:0", "echo", json!({"value":1}))
                .await
                .unwrap()
        else {
            panic!("not paused")
        };
        assert_eq!(value["result"]["value"], 7);
        assert_eq!(value["result"]["post_policy"], true);
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn unknown_effect_and_invalid_arguments_fail_closed() {
    let dir = private_directory();
    let (ctx, calls, _) = with_tools(context(), true);
    let j = journal::resolve(&ctx, Some(dir.path())).unwrap();
    assert!(
        nested::invoke(&ctx, &j, "bad", "echo", json!({"value":"wrong"}))
            .await
            .is_err()
    );
    let fingerprint = journal::digest(
        serde_json::to_vec(&json!({"tool":"echo","arguments":{"value":1}})).unwrap(),
    );
    j.store
        .try_claim_tool_call(&j.namespace, "unknown", "echo", &fingerprint)
        .await
        .unwrap();
    assert!(
        nested::invoke(&ctx, &j, "unknown", "echo", json!({"value":1}))
            .await
            .unwrap_err()
            .contains("unknown")
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn missing_policy_and_registry_supersets_cannot_authorize() {
    let dir = private_directory();
    let (mut ctx, calls, _) = with_tools(context(), true);
    let j = journal::resolve(&ctx, Some(dir.path())).unwrap();
    ctx.visible_tool_names = Some(Arc::new(Default::default()));
    assert!(
        nested::invoke(&ctx, &j, "x", "echo", json!({"value":1}))
            .await
            .is_err()
    );
    ctx.visible_tool_names = Some(Arc::new(["echo".into()].into()));
    ctx.nested_tool_policy = None;
    assert!(
        nested::invoke(&ctx, &j, "x", "echo", json!({"value":1}))
            .await
            .is_err()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

struct ExhaustedBudget;
#[async_trait]
impl everruns_contracts::runtime::tool_execution::BudgetChecker for ExhaustedBudget {
    async fn check_budgets(
        &self,
        _: &str,
    ) -> everruns_contracts::error::Result<everruns_contracts::runtime::budget::BudgetToolResponse>
    {
        Ok(everruns_contracts::runtime::budget::BudgetToolResponse {
            status: "exhausted".into(),
            budgets: vec![],
            hint: None,
        })
    }
}
#[tokio::test]
async fn exhausted_budget_prevents_nested_effects() {
    let dir = private_directory();
    let (mut ctx, calls, _) = with_tools(context(), true);
    ctx.budget_checker = Some(Arc::new(ExhaustedBudget));
    let j = journal::resolve(&ctx, Some(dir.path())).unwrap();
    assert!(
        nested::invoke(&ctx, &j, "budget", "echo", json!({"value":1}))
            .await
            .unwrap_err()
            .contains("budget")
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}
#[test]
fn json_escape_amplification_is_bounded_before_conversion() {
    assert!(value::from_python(MontyObject::string("\u{1}".repeat(50_000)).as_ref()).is_err());
    assert!(value::to_python(&json!("\u{1}".repeat(50_000))).is_err());
}

#[cfg(feature = "monty-runtime-tests")]
mod runtime {
    use super::*;
    fn tool(dir: &std::path::Path) -> PythonTool {
        PythonTool::new(MontyConfig {
            binary: std::env::var_os("EVERRUNS_MONTY_TEST_BIN")
                .expect("set EVERRUNS_MONTY_TEST_BIN to the hardened launcher")
                .into(),
            state_directory: Some(dir.to_path_buf()),
            max_processes: 2,
        })
    }
    async fn execute(tool: &PythonTool, ctx: &ToolContext, script: &str) -> Value {
        match tool
            .execute_with_context(json!({"script":script}), ctx)
            .await
        {
            ToolExecutionResult::Success(v) => v,
            other => panic!("{other:?}"),
        }
    }
    #[tokio::test]
    async fn calculations_prints_and_json_values() {
        let dir = private_directory();
        let result = execute(
            &tool(dir.path()),
            &context(),
            "print('hello')\n{'squares': [x*x for x in range(6)], 'ok': True}",
        )
        .await;
        assert_eq!(result["result"]["squares"], json!([0, 1, 4, 9, 16, 25]));
        assert_eq!(result["stdout"], "hello\n");
    }
    #[tokio::test]
    async fn effects_without_stable_host_identity_fail_before_dispatch() {
        let dir = private_directory();
        let (mut ctx, calls, _) = with_tools(context(), true);
        ctx.idempotency_key = None;
        let result = tool(dir.path())
            .execute_with_context(json!({"script": "tool('echo', {'value': 1})"}), &ctx)
            .await;
        let ToolExecutionResult::ToolError(error) = result else {
            panic!("{result:?}")
        };
        assert!(error.contains("idempotency key"), "{error}");
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
    #[tokio::test]
    async fn suspension_limit_blocks_the_next_effect_before_dispatch() {
        let dir = private_directory();
        let (ctx, calls, _) = with_tools(context(), true);
        let result = tool(dir.path()).execute_with_context(
            json!({"script": "for i in range(128):\n    tools('echo')\ntool('echo', {'value': 1})"}), &ctx
        ).await;
        let ToolExecutionResult::ToolError(error) = result else {
            panic!("{result:?}")
        };
        assert!(error.contains("suspension limit"), "{error}");
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
    #[tokio::test]
    async fn durable_approval_resumes_in_new_worker_without_repeating_earlier_effect() {
        let dir = private_directory();
        let (ctx, calls, policy) = with_tools(context(), false);
        let first = tool(dir.path()).execute_with_context(json!({"script":"a = tool('echo', {'value': 1})\nb = tool('echo', {'value': 2})\na['result']['value'] + b['result']['value']"}), &ctx).await;
        let ToolExecutionResult::PolicyOutcome(result) = first else {
            panic!("{first:?}")
        };
        assert!(
            everruns_contracts::tool_types::ToolApprovalRequired::from_tool_result(&result)
                .is_some()
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let token = result.result.unwrap()["python_resume"]
            .as_str()
            .unwrap()
            .to_string();
        policy.approved.store(true, Ordering::SeqCst);
        // Different host tool and process pool; only persistent records survive.
        let next = tool(dir.path());
        let resumed = next
            .execute_with_context(json!({"resume":token}), &ctx)
            .await;
        let ToolExecutionResult::Success(value) = resumed else {
            panic!("{resumed:?}")
        };
        assert_eq!(value["result"], 3);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert!(matches!(
            next.execute_with_context(json!({"resume":token}), &ctx)
                .await,
            ToolExecutionResult::Success(_)
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert!(matches!(
            next.execute_with_context(json!({"resume":token}), &context())
                .await,
            ToolExecutionResult::ToolError(_)
        ));
    }
    #[tokio::test]
    async fn native_os_network_process_and_environment_are_unavailable() {
        let dir = private_directory();
        let python = tool(dir.path());
        for script in [
            "open('/etc/passwd').read()",
            "import socket\nsocket.socket()",
            "import subprocess\nsubprocess.run(['id'])",
            "import os\nos.environ['PATH']",
            "from pathlib import Path\nPath('/etc/passwd').read_text()",
        ] {
            assert!(
                matches!(
                    python
                        .execute_with_context(json!({"script":script}), &context())
                        .await,
                    ToolExecutionResult::ToolError(_)
                ),
                "{script}"
            );
        }
    }
    #[tokio::test]
    async fn limits_discard_worker_and_next_call_is_clean() {
        let dir = private_directory();
        let python = tool(dir.path());
        for script in ["'a' * 200_000_000", "while True:\n    pass"] {
            assert!(matches!(
                python
                    .execute_with_context(json!({"script":script}), &context())
                    .await,
                ToolExecutionResult::ToolError(_)
            ));
        }
        assert_eq!(execute(&python, &context(), "21 * 2").await["result"], 42);
    }
    #[tokio::test]
    async fn cycles_and_output_amplification_are_bounded() {
        let dir = private_directory();
        let python = tool(dir.path());
        assert!(matches!(
            python
                .execute_with_context(json!({"script":"x=[]\nx.append(x)\nx"}), &context())
                .await,
            ToolExecutionResult::ToolError(_)
        ));
        let result = execute(&python, &context(), "print('x' * 100000)\n1").await;
        assert_eq!(result["stdout"].as_str().unwrap().len(), MAX_OUTPUT);
        assert_eq!(result["stdout_truncated"], true);
    }
    #[tokio::test]
    async fn permission_changes_are_rechecked_on_resume() {
        let dir = private_directory();
        let (mut ctx, calls, _) = with_tools(context(), false);
        let python = tool(dir.path());
        let first = python
            .execute_with_context(json!({"script":"tool('echo', {'value': 2})"}), &ctx)
            .await;
        let ToolExecutionResult::PolicyOutcome(result) = first else {
            panic!("{first:?}")
        };
        let token = result.result.unwrap()["python_resume"].clone();
        ctx.visible_tool_names = Some(Arc::new(Default::default()));
        assert!(matches!(
            python
                .execute_with_context(json!({"resume":token}), &ctx)
                .await,
            ToolExecutionResult::ToolError(_)
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
    #[tokio::test]
    async fn bashkit_keeps_cpython_and_cannot_route_into_python_code_mode() {
        use crate::bashkit::BashkitShellCapability;
        let mut ctx = context();
        ctx.file_store = Some(Arc::new(
            everruns_core::host::InMemorySessionFileStore::new(),
        ));
        let shell = BashkitShellCapability.tools().remove(0);
        let result = shell
            .execute_with_context(
                json!({"commands":"python3 -c 'import sys; print(sys.implementation.name)'"}),
                &ctx,
            )
            .await;
        let rendered = format!("{result:?}");
        assert!(rendered.contains("cpython"), "{rendered}");
        assert!(!nested::eligible(shell.as_ref()));
        assert!(
            everruns_contracts::runtime::tool_surface::NATIVE_ONLY_TOOL_NAMES.contains(&"python")
        );
    }
    #[tokio::test]
    async fn network_tools_fail_closed_without_an_egress_service() {
        let dir = private_directory();
        let (mut ctx, _, _) = with_tools(context(), true);
        let mut registry = ToolRegistry::new();
        registry.register(crate::web_fetch::WebFetchTool::new(false));
        ctx.tool_registry = Some(Arc::new(registry));
        ctx.visible_tool_names = Some(Arc::new(["web_fetch".into()].into()));
        let result = execute(
            &tool(dir.path()),
            &ctx,
            "tool('web_fetch', {'url': 'https://example.com'})",
        )
        .await;
        assert!(!result["result"]["error"].is_null(), "{result}");
        assert!(result.to_string().contains("egress"), "{result}");
    }
    #[tokio::test]
    async fn network_policy_and_ssrf_guards_apply_inside_python() {
        let dir = private_directory();
        let (mut ctx, _, _) = with_tools(context(), true);
        let mut registry = ToolRegistry::new();
        registry.register(crate::web_fetch::WebFetchTool::new(false));
        ctx.tool_registry = Some(Arc::new(registry));
        ctx.visible_tool_names = Some(Arc::new(["web_fetch".into()].into()));
        ctx.egress_service = Some(Arc::new(everruns_core::host::DirectEgressService::new()));
        let python = tool(dir.path());
        let private = execute(
            &python,
            &ctx,
            "tool('web_fetch', {'url': 'http://127.0.0.1:9', 'method': 'POST'})",
        )
        .await;
        let private_error = private["result"]["error"].as_str().unwrap();
        assert!(
            private_error.contains("private") || private_error.contains("blocked"),
            "{private}"
        );
        ctx.idempotency_key = Some("another-python-call".into());
        ctx.network_access = Some(
            everruns_contracts::runtime::network_access::NetworkAccessList::allow_only([
                "allowed.example",
            ]),
        );
        let denied = execute(
            &python,
            &ctx,
            "tool('web_fetch', {'url': 'https://example.com', 'method': 'POST'})",
        )
        .await;
        assert!(!denied["result"]["error"].is_null(), "{denied}");
        assert!(
            denied["result"]["error"]
                .as_str()
                .unwrap()
                .contains("network access policy"),
            "{denied}"
        );
    }
}

use super::*;
use crate::bashkit::BashTool;
use everruns_contracts::runtime::tool_hooks::NestedToolPolicy;
use everruns_contracts::runtime::tools::ToolRegistry;
use everruns_contracts::tool_types::{BuiltinTool, ToolResult};
use std::sync::Mutex;

/// Echoes its input, so a test sees exactly the object the command built.
struct EchoTool {
    name: &'static str,
    hints: ToolHints,
    policy: ToolPolicy,
}

impl EchoTool {
    fn named(name: &'static str) -> Self {
        Self {
            name,
            hints: ToolHints::default(),
            policy: ToolPolicy::Auto,
        }
    }
}

#[async_trait]
impl Tool for EchoTool {
    fn name(&self) -> &str {
        self.name
    }
    fn description(&self) -> &str {
        "Echo the input back. Second sentence."
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {"repo": {"type": "string"}, "number": {"type": "integer"}},
            "additionalProperties": false
        })
    }
    fn policy(&self) -> ToolPolicy {
        self.policy.clone()
    }
    fn hints(&self) -> ToolHints {
        self.hints.clone()
    }
    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::Success(json!({"tool": self.name, "input": arguments}))
    }
}

/// Lets every call through except `blocked`, and counts post-chain calls.
#[derive(Default)]
struct Policy {
    blocked: Option<&'static str>,
    approval_for: Option<&'static str>,
    after: Mutex<Vec<String>>,
}

#[async_trait]
impl NestedToolPolicy for Policy {
    async fn authorize(
        &self,
        tool_call: ToolCall,
        _tool_def: &ToolDefinition,
        _context: &ToolContext,
    ) -> Result<ToolCall, ToolResult> {
        let refuse = |result, error| ToolResult {
            tool_call_id: tool_call.id.clone(),
            result,
            images: None,
            error,
            connection_required: None,
            raw_output: None,
        };
        if Some(tool_call.name.as_str()) == self.blocked {
            return Err(refuse(None, Some("blocked by guardrail".to_string())));
        }
        if Some(tool_call.name.as_str()) == self.approval_for {
            return Err(refuse(
                Some(json!({"code": everruns_contracts::TOOL_APPROVAL_REQUIRED_CODE})),
                None,
            ));
        }
        Ok(tool_call)
    }

    async fn after_exec(
        &self,
        tool_call: &ToolCall,
        _tool_def: &ToolDefinition,
        _result: &mut ToolResult,
        _context: &ToolContext,
    ) {
        self.after.lock().unwrap().push(tool_call.name.clone());
    }
}

fn context(policy: Option<Arc<Policy>>, marker: bool) -> ToolContext {
    let (mut context, _) = crate::bashkit::tests::create_context_with_mock_store();
    let mut registry = ToolRegistry::new();
    registry.register(EchoTool::named("mcp_github__list_pulls"));
    registry.register(EchoTool::named("mcp_github__get_issue"));
    registry.register(EchoTool::named("web_fetch"));
    registry.register(EchoTool {
        hints: ToolHints::default().with_destructive(true),
        ..EchoTool::named("delete_everything")
    });
    if marker {
        registry.register(ToolsMarkerTool);
    }
    context.tool_registry = Some(Arc::new(registry));
    context.tool_call_id = Some("call_outer".to_string());
    if let Some(policy) = policy {
        context = context.with_nested_tool_policy(policy);
    }
    context
}

async fn run(script: &str, context: &ToolContext) -> Value {
    match BashTool::default()
        .execute_with_context(json!({"commands": script}), context)
        .await
    {
        ToolExecutionResult::Success(output) => output,
        other => panic!("bash failed: {other:?}"),
    }
}

fn error_code(output: &Value) -> String {
    let stderr = output["stderr"].as_str().unwrap_or_default();
    let line = stderr.lines().find(|l| l.starts_with('{')).unwrap_or("{}");
    serde_json::from_str::<Value>(line).unwrap()["error"]["code"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

#[tokio::test]
async fn mcp_tool_runs_by_server_and_name_and_pipes_into_jq() {
    let policy = Arc::new(Policy::default());
    let context = context(Some(policy.clone()), true);
    let output = run(
        r#"tools github list-pulls '{"repo":"a/b"}' number=7 | jq -c .input"#,
        &context,
    )
    .await;
    assert_eq!(output["exit_code"], 0, "{output}");
    assert_eq!(output["stdout"], "{\"number\":7,\"repo\":\"a/b\"}\n");
    assert_eq!(
        policy.after.lock().unwrap().as_slice(),
        ["mcp_github__list_pulls"],
        "the post-tool chain saw the call"
    );
}

#[tokio::test]
async fn top_level_tool_reads_stdin_json() {
    let context = context(Some(Arc::new(Policy::default())), true);
    let output = run(
        r#"echo '{"repo":"x"}' | tools web-fetch | jq -r .tool"#,
        &context,
    )
    .await;
    assert_eq!(output["stdout"], "web_fetch\n", "{output}");
}

#[tokio::test]
async fn help_lists_servers_and_tools_but_not_direct_ones() {
    let context = context(Some(Arc::new(Policy::default())), true);
    let output = run("tools --help", &context).await;
    let stdout = output["stdout"].as_str().unwrap();
    assert!(stdout.contains("github  (2 tools)"), "{stdout}");
    assert!(
        stdout.contains("web-fetch  Echo the input back."),
        "{stdout}"
    );
    assert!(!stdout.contains("delete-everything"), "{stdout}");
    assert!(
        !stdout.contains("  tools  "),
        "the marker is not a command: {stdout}"
    );

    let output = run("tools github get-issue --help", &context).await;
    let stdout = output["stdout"].as_str().unwrap();
    assert!(stdout.starts_with("tools github get-issue '{"), "{stdout}");
    assert!(stdout.contains("Input schema:"), "{stdout}");

    let output = run("tools search issue", &context).await;
    let stdout = output["stdout"].as_str().unwrap();
    assert!(
        stdout.starts_with("tools github get-issue"),
        "a name hit ranks first: {stdout}"
    );
}

#[tokio::test]
async fn errors_are_one_json_envelope_a_script_can_branch_on() {
    let policy = Arc::new(Policy {
        blocked: Some("web_fetch"),
        approval_for: Some("mcp_github__get_issue"),
        ..Policy::default()
    });
    let context = context(Some(policy.clone()), true);

    let cases = [
        ("tools nope", "unknown_command"),
        ("tools github nope", "unknown_command"),
        ("tools delete-everything", "unknown_command"),
        ("tools github list-pulls stray", "invalid_input"),
        ("tools github list-pulls '{bad'", "invalid_input"),
        ("tools github list-pulls unknown_key=1", "invalid_input"),
        ("tools web-fetch", "denied"),
        ("tools github get-issue", "needs_approval"),
    ];
    for (script, code) in cases {
        let output = run(script, &context).await;
        assert_eq!(output["exit_code"], 1, "{script}: {output}");
        assert_eq!(error_code(&output), code, "{script}: {output}");
    }
    assert!(
        policy.after.lock().unwrap().is_empty(),
        "nothing refused ever ran"
    );
}

#[tokio::test]
async fn a_loop_stops_at_the_call_limit() {
    let context = context(Some(Arc::new(Policy::default())), true);
    let script = format!(
        "for i in $(seq 1 {}); do tools web-fetch > /dev/null || exit 3; done",
        builtin::MAX_CALLS_PER_EXECUTION + 1
    );
    let output = run(&script, &context).await;
    assert_eq!(output["exit_code"], 3, "{output}");
    assert_eq!(error_code(&output), "call_limit");
}

#[tokio::test]
async fn no_marker_or_no_policy_means_no_command() {
    for context in [
        context(Some(Arc::new(Policy::default())), false),
        context(None, true),
    ] {
        let output = run("tools --help", &context).await;
        assert_ne!(output["exit_code"], 0, "{output}");
        assert!(
            !output["stdout"]
                .as_str()
                .unwrap_or_default()
                .contains("Usage")
        );
    }
}

fn definition(name: &str, policy: ToolPolicy, hints: ToolHints) -> ToolDefinition {
    ToolDefinition::Builtin(BuiltinTool {
        name: name.to_string(),
        display_name: None,
        description: format!("{name} tool"),
        parameters: json!({"type": "object"}),
        policy,
        category: None,
        deferrable: DeferrablePolicy::Automatic,
        hints,
        full_parameters: None,
    })
}

#[test]
fn hook_hides_reachable_tools_and_names_them_on_bash() {
    let hook = HideBehindToolsHook {
        keep_visible: HashSet::from(["kept".to_string()]),
    };
    let defs = vec![
        definition("bash", ToolPolicy::Auto, ToolHints::default()),
        definition(TOOLS_COMMAND, ToolPolicy::Auto, ToolHints::default()),
        definition(
            "mcp_github__list_pulls",
            ToolPolicy::Auto,
            ToolHints::default(),
        ),
        definition(
            "mcp_github__get_issue",
            ToolPolicy::Auto,
            ToolHints::default(),
        ),
        definition("web_fetch", ToolPolicy::Auto, ToolHints::default()),
        definition("kept", ToolPolicy::Auto, ToolHints::default()),
        definition(
            "approve_me",
            ToolPolicy::RequiresApproval,
            ToolHints::default(),
        ),
        definition(
            "rm_rf",
            ToolPolicy::Auto,
            ToolHints::default().with_destructive(true),
        ),
        definition(
            "spawn_agent",
            ToolPolicy::Auto,
            ToolHints::default().with_stays_direct(true),
        ),
        ToolDefinition::function("client_tool", "runs on the client", json!({})),
    ];
    let kept = hook.transform(defs);
    let names: Vec<&str> = kept.iter().map(ToolDefinition::name).collect();
    assert_eq!(
        names,
        [
            "bash",
            "kept",
            "approve_me",
            "rm_rf",
            "spawn_agent",
            "client_tool"
        ]
    );
    let bash = kept[0].description();
    assert!(bash.contains("github (2), web-fetch"), "{bash}");
}

#[test]
fn hook_leaves_bash_alone_when_nothing_is_hidden() {
    let hook = HideBehindToolsHook::default();
    let kept = hook.transform(vec![
        definition("bash", ToolPolicy::Auto, ToolHints::default()),
        definition(TOOLS_COMMAND, ToolPolicy::Auto, ToolHints::default()),
    ]);
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].description(), "bash tool");
}

#[test]
fn deferred_mcp_placeholders_stay_direct() {
    assert!(!goes_behind_tools(
        "mcp_github",
        false,
        &ToolPolicy::Auto,
        &DeferrablePolicy::Never,
        &ToolHints::default(),
    ));
}

#[test]
fn capability_contract() {
    let capability = ToolsInShellCapability;
    assert_eq!(capability.dependencies(), vec!["bashkit_shell"]);
    assert!(capability.supersedes().contains(&"tool_search"));
    assert_eq!(capability.tools()[0].name(), TOOLS_COMMAND);
    assert!(capability.validate_config(&Value::Null).is_ok());
    assert!(
        capability
            .validate_config(&json!({"keep_visible": ["a"]}))
            .is_ok()
    );
    assert!(capability.validate_config(&json!({"other": 1})).is_err());
    assert!(
        capability
            .validate_config(&json!({"keep_visible": [1]}))
            .is_err()
    );
    assert!(capability.validate_config(&json!([])).is_err());
}

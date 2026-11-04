use super::*;
use crate::bashkit::BashTool;
use everruns_contracts::runtime::tool_hooks::NestedToolPolicy;
use everruns_contracts::runtime::tools::ToolRegistry;
use everruns_contracts::tool_types::{BuiltinTool, ToolResult};
use std::sync::Mutex;

#[path = "deferred_tests.rs"]
mod deferred;
#[path = "plan_tests.rs"]
mod plan;
#[path = "preflight_tests.rs"]
mod preflight;
#[path = "ratings_tests.rs"]
mod ratings;
#[path = "scripts_tests.rs"]
mod scripts;
#[path = "stop_tests.rs"]
mod stop;
#[path = "timeline_tests.rs"]
mod timeline;

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
    /// Whether the gate can tell, before a script starts, that it would hold
    /// `approval_for`, as the durable approval gate can.
    previews: bool,
    /// Hold every call to a tool whose definition says it is destructive, as
    /// the approval gate does at its default level.
    approval_for_destructive: bool,
    after: Mutex<Vec<String>>,
}

/// The payload the real approval gate holds a call with.
fn approval_payload(tool_call: &ToolCall) -> Value {
    json!({
        "code": everruns_contracts::TOOL_APPROVAL_REQUIRED_CODE,
        "error": "waiting for a person",
        "tool_call_id": tool_call.id,
        "tool": tool_call.name,
        "arguments": tool_call.arguments,
        "fingerprint": "fp",
        "risk": "destructive",
        "mode": "normal",
        "asked_at": "2026-10-09T00:00:00Z",
        "expires_at": "2026-10-10T00:00:00Z",
    })
}

#[async_trait]
impl NestedToolPolicy for Policy {
    async fn authorize(
        &self,
        tool_call: ToolCall,
        tool_def: &ToolDefinition,
        _context: &ToolContext,
    ) -> Result<ToolCall, ToolResult> {
        let refuse = |result, error| ToolResult {
            details: None,
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
        if Some(tool_call.name.as_str()) == self.approval_for
            || (self.approval_for_destructive && tool_def.hints().destructive == Some(true))
        {
            return Err(refuse(Some(approval_payload(&tool_call)), None));
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

    async fn preview(
        &self,
        tool_call: &ToolCall,
        tool_def: &ToolDefinition,
        _context: &ToolContext,
    ) -> Option<ToolResult> {
        let held = Some(tool_call.name.as_str()) == self.approval_for
            || (self.approval_for_destructive && tool_def.hints().destructive == Some(true));
        (self.previews && held).then(|| ToolResult {
            details: None,
            tool_call_id: tool_call.id.clone(),
            result: Some(approval_payload(tool_call)),
            images: None,
            error: None,
            connection_required: None,
            raw_output: None,
        })
    }
}

fn context(policy: Option<Arc<Policy>>, marker: bool) -> ToolContext {
    let (mut context, _) = crate::bashkit::tests::create_context_with_mock_store();
    let mut registry = ToolRegistry::new();
    registry.register(EchoTool::named("mcp_github__list_pulls"));
    registry.register(EchoTool::named("mcp_github__get_issue"));
    registry.register(EchoTool::named("web_fetch"));
    registry.register(EchoTool {
        hints: ToolHints::default().with_stays_direct(true),
        ..EchoTool::named("ask_person")
    });
    registry.register(EchoTool {
        hints: ToolHints::default().with_readonly(true),
        ..EchoTool::named("read_notes")
    });
    if marker {
        registry.register(ToolsMarkerTool);
    }
    install_cli_surface(&mut context, &registry);
    context.tool_registry = Some(Arc::new(registry));
    context.tool_call_id = Some("call_outer".to_string());
    if let Some(policy) = policy {
        context = context.with_nested_tool_policy(policy);
    }
    context
}

fn install_cli_surface(context: &mut ToolContext, registry: &ToolRegistry) {
    let definitions: Vec<_> = registry
        .tool_names()
        .into_iter()
        .filter_map(|name| registry.get(name))
        .map(|tool| tool.to_definition())
        .collect();
    let routes = ToolsInShellCapability.tool_surface_routes_with_config(&json!({}), &definitions);
    let mut plan =
        everruns_contracts::runtime::tool_surface::ToolSurfacePlan::for_mode(ToolSurfaceMode::Cli);
    plan.extend(routes);
    context.tool_surface_plan = plan;
}

#[tokio::test]
async fn multilingual_cli_search_keeps_authorized_canonical_names() {
    let mut context = context(Some(Arc::new(Policy::default())), true);
    let mut registry = ToolRegistry::new();
    for name in ["café", "cafe\u{301}"] {
        registry.register(EchoTool::named(name));
    }
    registry.register(ToolsMarkerTool);
    install_cli_surface(&mut context, &registry);
    context.tool_registry = Some(Arc::new(registry));
    context.visible_tool_names = Some(Arc::new(std::collections::HashSet::from([
        "café".into(),
        TOOLS_COMMAND.into(),
    ])));
    let output = run("tools search cafe\u{301}", &context).await;
    let stdout = output["stdout"].as_str().unwrap();
    assert!(stdout.contains("tools café"), "{output}");
    assert!(!stdout.contains("tools cafe\u{301}"), "{output}");
}

#[tokio::test]
async fn capability_purpose_search_refines_sources_and_keeps_help_and_calls_in_scope() {
    let policy = Arc::new(Policy::default());
    let mut context = context(Some(policy), true);
    let mut registry = ToolRegistry::new();
    for (name, id, title) in [
        ("opaque_zeta", "stockroom", "Stockroom"),
        ("opaque_omega", "private_stockroom", "Private Stockroom"),
        ("mcp_atlas__opaque", "mcp_capability", "Remote Inventory"),
    ] {
        registry.register(EchoTool {
            hints: ToolHints {
                capability_id: Some(id.into()),
                capability_name: Some(title.into()),
                capability_description: Some("Warehouse fulfillment inventory".into()),
                ..Default::default()
            },
            ..EchoTool::named(name)
        });
    }
    registry.register(ToolsMarkerTool);
    install_cli_surface(&mut context, &registry);
    context.tool_registry = Some(Arc::new(registry));
    context.visible_tool_names = Some(Arc::new(std::collections::HashSet::from([
        "opaque_zeta".into(),
        "mcp_atlas__opaque".into(),
        TOOLS_COMMAND.into(),
    ])));

    let catalog = catalog::Catalog::from_context(&context);
    assert!(catalog.search("warehouse fulfillment", 10).is_empty());
    let sources = catalog.source_matches("warehouse fulfillment", 10);
    assert_eq!(sources.len(), 2);
    assert!(
        sources
            .iter()
            .any(|source| source.id == "stockroom" && source.name == "Stockroom")
    );
    assert!(sources.iter().any(|source| source.id == "atlas"));

    let output = run("tools search warehouse fulfillment", &context).await;
    let stdout = output["stdout"].as_str().unwrap();
    assert!(stdout.contains("Matching sources"), "{output}");
    assert!(stdout.contains("Stockroom (stockroom)"), "{output}");
    assert!(stdout.contains("tools atlas --help"), "{output}");
    assert!(!stdout.contains("private_stockroom"), "{output}");
    assert!(!stdout.contains("opaque-zeta"), "{output}");
    let output = run("tools search opaque-zeta", &context).await;
    assert!(
        output["stdout"]
            .as_str()
            .unwrap()
            .contains("tools opaque-zeta")
    );
    let output = run("tools opaque-zeta --help", &context).await;
    assert!(output["stdout"].as_str().unwrap().contains("Input schema"));
    let output = run("tools opaque-zeta '{\"repo\":\"inventory\"}'", &context).await;
    assert!(
        output["stdout"].as_str().unwrap().contains("inventory"),
        "{output}"
    );
    let output = run("tools opaque-omega --help", &context).await;
    assert_eq!(error_code(&output), "unknown_command", "{output}");
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
    assert!(!stdout.contains("ask-person"), "{stdout}");
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
        ("tools ask-person", "unknown_command"),
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
    // The limit stops the script itself, before `|| exit 3` can run.
    assert_eq!(output["exit_code"], 1, "{output}");
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
fn hook_hides_cli_tools_without_rewriting_bash_description() {
    let hook = HideBehindToolsHook {
        keep_visible: HashSet::from(["kept".to_string()]),
        mode: ToolSurfaceMode::Cli,
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
    // Approval-gated and destructive tools go behind: the gate judges each
    // call from a script, and one it holds stops the script.
    assert_eq!(names, ["bash", "kept", "spawn_agent", "client_tool"]);
    let bash = kept[0].description();
    assert_eq!(bash, "bash tool");
    assert!(goes_behind_tools(
        "write_todos",
        false,
        &ToolPolicy::Auto,
        &ToolHints::default(),
    ));
}

#[test]
fn hybrid_keeps_native_definitions_and_bash_schema_stable_as_catalog_changes() {
    let hook = HideBehindToolsHook {
        keep_visible: HashSet::new(),
        mode: ToolSurfaceMode::Hybrid,
    };
    let first = hook.transform(vec![
        definition("bash", ToolPolicy::Auto, ToolHints::default()),
        definition("tools", ToolPolicy::Auto, ToolHints::default()),
        definition("mcp_docs__search", ToolPolicy::Auto, ToolHints::default()),
    ]);
    let second = hook.transform(vec![
        definition("bash", ToolPolicy::Auto, ToolHints::default()),
        definition("tools", ToolPolicy::Auto, ToolHints::default()),
        definition("mcp_docs__search", ToolPolicy::Auto, ToolHints::default()),
        definition("mcp_docs__get_page", ToolPolicy::Auto, ToolHints::default()),
    ]);
    assert_eq!(first[0].description(), second[0].description());
    assert_eq!(first[0].parameters(), second[0].parameters());
    assert_eq!(
        first.iter().map(ToolDefinition::name).collect::<Vec<_>>(),
        ["bash", "mcp_docs__search"]
    );
    assert_eq!(
        second.iter().map(ToolDefinition::name).collect::<Vec<_>>(),
        ["bash", "mcp_docs__search", "mcp_docs__get_page"]
    );
}

#[test]
fn resolved_routes_separate_cli_eligibility_from_native_schema_loading() {
    let capability = ToolsInShellCapability;
    let mut eager = definition("write_todos", ToolPolicy::Auto, ToolHints::default());
    if let ToolDefinition::Builtin(tool) = &mut eager {
        tool.deferrable = DeferrablePolicy::Never;
    }
    let automatic = definition("search_docs", ToolPolicy::Auto, ToolHints::default());
    let direct = definition(
        "computer",
        ToolPolicy::Auto,
        ToolHints::default().with_stays_direct(true),
    );
    let routes = capability
        .tool_surface_routes_with_config(&json!({"mode": "hybrid"}), &[eager, automatic, direct]);
    assert_eq!(
        routes["write_todos"],
        ToolSurfaceRoute {
            surface: ToolSurface::Both,
            native_disclosure: NativeToolDisclosure::Eager,
        }
    );
    assert_eq!(
        routes["search_docs"],
        ToolSurfaceRoute {
            surface: ToolSurface::Both,
            native_disclosure: NativeToolDisclosure::Eager,
        }
    );
    assert_eq!(routes["computer"].surface, ToolSurface::Native);

    let visible = capability.tool_surface_routes_with_config(
        &json!({"mode": "cli", "keep_visible": ["search_docs"]}),
        &[definition(
            "search_docs",
            ToolPolicy::Auto,
            ToolHints::default(),
        )],
    );
    assert_eq!(visible["search_docs"].surface, ToolSurface::Both);
}

#[test]
fn bounded_native_disclosure_policy_respects_never_defer_and_native_only_tools() {
    let mut plan = everruns_contracts::runtime::tool_surface::ToolSurfacePlan::for_mode(
        ToolSurfaceMode::Native,
    );
    plan.set_native_disclosure_policy(
        everruns_contracts::runtime::tool_surface::NativeToolDisclosurePolicy {
            default: NativeToolDisclosure::Deferred,
            overrides: [("pinned_operation".to_string(), NativeToolDisclosure::Eager)].into(),
        },
    );
    let mut never_defer = definition("pinned_operation", ToolPolicy::Auto, ToolHints::default());
    if let ToolDefinition::Builtin(tool) = &mut never_defer {
        tool.deferrable = DeferrablePolicy::Never;
    }
    let definitions = [
        definition("ordinary_operation", ToolPolicy::Auto, ToolHints::default()),
        never_defer,
        definition("ask_user", ToolPolicy::Auto, ToolHints::default()),
        definition("request_approval", ToolPolicy::Auto, ToolHints::default()),
        definition("generate_image", ToolPolicy::Auto, ToolHints::default()),
    ];
    plan.extend_definitions(&definitions, |_| false);

    assert_eq!(
        plan.route_for("ordinary_operation")
            .unwrap()
            .native_disclosure,
        NativeToolDisclosure::Deferred
    );
    assert_eq!(
        plan.route_for("pinned_operation")
            .unwrap()
            .native_disclosure,
        NativeToolDisclosure::Eager
    );
    assert_eq!(
        plan.route_for("generate_image").unwrap().native_disclosure,
        NativeToolDisclosure::Eager
    );
    assert_eq!(
        plan.route_for("ask_user").unwrap().native_disclosure,
        NativeToolDisclosure::Eager
    );
    assert_eq!(
        plan.route_for("request_approval")
            .unwrap()
            .native_disclosure,
        NativeToolDisclosure::Eager
    );

    let hybrid = everruns_contracts::runtime::tool_surface::ToolSurfacePlan::for_mode(
        ToolSurfaceMode::Hybrid,
    );
    let late = definition("late_operation", ToolPolicy::Auto, ToolHints::default());
    assert_eq!(
        hybrid.route_for_definition(&late, |_| true),
        ToolSurfaceRoute {
            surface: ToolSurface::Both,
            native_disclosure: NativeToolDisclosure::Eager,
        }
    );

    let mut bounded_hybrid = everruns_contracts::runtime::tool_surface::ToolSurfacePlan::for_mode(
        ToolSurfaceMode::Hybrid,
    );
    bounded_hybrid.set_native_disclosure_policy(
        everruns_contracts::runtime::tool_surface::NativeToolDisclosurePolicy {
            default: NativeToolDisclosure::Deferred,
            overrides: Default::default(),
        },
    );
    assert_eq!(
        bounded_hybrid.route_for_definition(&late, |_| true),
        ToolSurfaceRoute {
            surface: ToolSurface::Both,
            native_disclosure: NativeToolDisclosure::Deferred,
        }
    );
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
fn shell_catalog_fails_closed_when_a_tool_has_no_resolved_cli_route() {
    let mut context = context(Some(Arc::new(Policy::default())), true);
    context.tool_surface_plan = Default::default();
    let catalog = catalog::Catalog::from_context(&context);
    assert!(catalog.search("web fetch", 10).is_empty());
    assert!(!catalog.root_help().contains("web-fetch"));
}

#[tokio::test]
async fn shell_catalog_obeys_the_full_operation_scope_even_when_registry_is_a_superset() {
    let policy = Arc::new(Policy::default());
    let mut context = context(Some(policy.clone()), true);
    context.visible_tool_names = Some(Arc::new(std::collections::HashSet::new()));

    let catalog = catalog::Catalog::from_context(&context);
    assert!(catalog.root_help().contains("No tools are available"));
    assert!(catalog.search("web fetch", 10).is_empty());

    let output = run("tools web-fetch", &context).await;
    assert_eq!(error_code(&output), "unknown_command", "{output}");
    assert!(policy.after.lock().unwrap().is_empty());
}

#[test]
fn shell_search_loads_at_most_eight_matching_deferred_servers() {
    use everruns_contracts::runtime::mcp_deferred::{
        DeferredMcpServerTool, deferred_mcp_server_definition,
    };

    let (mut context, _) = crate::bashkit::tests::create_context_with_mock_store();
    let mut registry = ToolRegistry::new();
    for index in 0..12 {
        let mut definition = deferred_mcp_server_definition(
            &format!("docs{index}"),
            Some("Product documentation server"),
        );
        let everruns_contracts::tool_types::ToolDefinition::Builtin(tool) = &mut definition else {
            unreachable!()
        };
        tool.hints.metadata = Some(json!({"mcp_server": {
            "name": format!("docs{index}"),
            "description": "Product documentation server"
        }}));
        registry.register(DeferredMcpServerTool::new(tool.clone()));
    }
    install_cli_surface(&mut context, &registry);
    context.tool_registry = Some(Arc::new(registry));

    let mut catalog = catalog::Catalog::from_context(&context);
    assert_eq!(catalog.pending_matching("product documentation").len(), 8);

    let loaded = catalog.pending_source("docs0").unwrap();
    catalog
        .add_loaded(
            &loaded.prefix,
            vec![Arc::new(EchoTool::named("mcp_docs0__search_docs"))],
        )
        .unwrap();
    assert!(
        catalog
            .pending_matching("docs11")
            .iter()
            .any(|pending| pending.prefix == "docs11"),
        "remaining pending servers stay searchable after another server loads"
    );
}

#[test]
fn capability_contract() {
    let capability = ToolsInShellCapability;
    assert_eq!(capability.dependencies(), vec!["bashkit_shell"]);
    assert!(capability.supersedes().contains(&"tool_search"));
    assert_eq!(capability.tools()[0].name(), TOOLS_COMMAND);
    assert!(capability.validate_config(&Value::Null).is_ok());
    assert_eq!(surface_mode(&json!({})), ToolSurfaceMode::Cli);
    assert_eq!(
        surface_mode(&json!({"mode": "hybrid"})),
        ToolSurfaceMode::Hybrid
    );
    assert!(
        capability
            .validate_config(&json!({"mode": "hybrid"}))
            .is_ok()
    );
    assert!(
        capability
            .validate_config(&json!({"mode": "native"}))
            .is_err()
    );
    assert!(
        capability
            .supersedes_with_config(&json!({"mode": "hybrid"}))
            .is_empty()
    );
    assert_eq!(
        capability.tool_surface_mode_with_config(&json!({"mode": "hybrid"})),
        ToolSurfaceMode::Hybrid
    );
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
    assert!(
        capability
            .validate_config(&json!({"manage_scripts": true}))
            .is_ok()
    );
    assert!(
        capability
            .validate_config(&json!({"manage_scripts": "yes"}))
            .is_err()
    );
    assert!(manage_scripts(&json!({"manage_scripts": true})));
    assert!(!manage_scripts(&json!({})));
    assert!(
        capability
            .validate_config(&json!({"rate_unlabeled_tools": false}))
            .is_ok()
    );
    assert!(
        capability
            .validate_config(&json!({"rate_unlabeled_tools": "no"}))
            .is_err()
    );
    assert!(rate_unlabeled_tools(&json!({})), "on by default");
    assert!(rate_unlabeled_tools(&Value::Null));
    assert!(!rate_unlabeled_tools(
        &json!({"rate_unlabeled_tools": false})
    ));
    let schema = capability.config_schema().unwrap();
    assert_eq!(
        schema["properties"]["rate_unlabeled_tools"]["default"],
        true
    );
}

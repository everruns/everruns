use super::*;
use crate::builtins::tool_types::{BuiltinTool, ToolPolicy};
use crate::builtins::typed_id::SessionId;

fn builtin(name: &str, description: &str, deferrable: DeferrablePolicy) -> ToolDefinition {
    ToolDefinition::Builtin(BuiltinTool {
        name: name.to_string(),
        display_name: None,
        description: description.to_string(),
        parameters: json!({
            "type": "object",
            "properties": { "path": { "type": "string" } },
            "required": ["path"]
        }),
        policy: ToolPolicy::Auto,
        category: None,
        deferrable,
        hints: ToolHints::default(),
        full_parameters: None,
    })
}

fn many_tools(n: usize) -> Vec<ToolDefinition> {
    (0..n)
        .map(|i| {
            builtin(
                &format!("tool_{i}"),
                "does something",
                DeferrablePolicy::Automatic,
            )
        })
        .collect()
}

/// A bare hook with an empty allowlist for a session with no transcript.
fn hook(threshold: usize) -> DeferSchemaHook {
    DeferSchemaHook {
        threshold,
        never_defer: Arc::new(HashSet::new()),
        disclosure: ToolDisclosure::default(),
        reveal: Reveal::Redefine,
    }
}

fn ctx_for(messages: &[RuntimeMessage]) -> SystemPromptContext {
    let mut ctx = SystemPromptContext::without_file_store(SessionId::new());
    ctx.tool_disclosure = tool_disclosure(messages);
    ctx
}

/// A `tool_search` call and its result, as stored in the transcript.
fn search_turn(call_id: &str, result: Value) -> Vec<RuntimeMessage> {
    let mut call = RuntimeMessage::assistant("");
    call.content = vec![ContentPart::ToolCall(
        everruns_contracts::runtime::message::ToolCallContentPart::new(
            call_id,
            TOOL_SEARCH_TOOL_NAME,
            json!({ "query": "q" }),
        ),
    )];
    vec![
        call,
        RuntimeMessage::tool_result(call_id, Some(result), None),
    ]
}

fn is_stubbed(tool: &ToolDefinition) -> bool {
    tool.parameters().get("properties").is_none()
}

// Metadata/tool-list constants covered by builtin_capabilities_satisfy_registry_invariants.

#[test]
fn test_hook_noop_below_threshold() {
    let hook = hook(15);
    let tools = many_tools(5);
    let out = hook.transform(tools);
    assert_eq!(out.len(), 5);
    // Schemas untouched below threshold.
    for t in &out {
        assert!(t.parameters().get("properties").is_some());
    }
}

#[test]
fn test_hook_strips_above_threshold() {
    let hook = hook(15);
    let out = hook.transform(many_tools(20));
    assert_eq!(out.len(), 20);
    for t in &out {
        // The shared capability prompt carries the progressive-disclosure
        // instruction once; each stub is only the provider-valid open object.
        assert!(t.parameters().get("properties").is_none());
        assert_eq!(t.parameters()["additionalProperties"], json!(true));
        assert!(t.parameters().get("description").is_none());
        assert!(
            t.full_parameters().get("properties").is_some(),
            "full schema should remain available for progressive disclosure"
        );
    }
}

#[test]
fn test_deferral_stays_on_when_the_tool_count_drops() {
    // A tool leaving (an MCP server dropping out, a client tool set
    // shrinking) must not flip every stub back to its full schema: that
    // rewrites the head of the cached prompt.
    // The transcript records the stubs sent, which keeps deferral on.
    let first = hook(15).transform(many_tools(20));
    assert!(first.iter().all(is_stubbed));
    let mut answer = RuntimeMessage::assistant("done");
    answer.metadata = Some(
        [(
            everruns_contracts::tool_changes::METADATA_KEY.to_string(),
            json!(ToolChanges::baseline(first)),
        )]
        .into(),
    );
    let transcript = [RuntimeMessage::user("hi"), answer];
    let cap = ToolSearchCapability::with_threshold(15);
    let hooks = cap.tool_definition_hooks_with_context(&ctx_for(&transcript), &json!({}));
    let out = hooks[0].transform(many_tools(10));
    assert!(
        out.iter().all(is_stubbed),
        "deferral stays on below threshold"
    );
    // A session that never reached the threshold still keeps full schemas.
    assert!(!hook(15).transform(many_tools(10)).iter().any(is_stubbed));
}

#[test]
fn test_hook_preserves_never_defer_and_search_tool() {
    let hook = hook(3);
    let mut tools = many_tools(3);
    tools.push(builtin("write_todos", "todos", DeferrablePolicy::Never));
    tools.push(ToolSearchTool::default().to_definition());

    let out = hook.transform(tools);

    let todos = out.iter().find(|t| t.name() == "write_todos").unwrap();
    assert!(
        todos.parameters().get("properties").is_some(),
        "never-defer tool keeps full schema"
    );
    let search = out
        .iter()
        .find(|t| t.name() == TOOL_SEARCH_TOOL_NAME)
        .unwrap();
    assert!(
        search.parameters().get("properties").is_some(),
        "search tool keeps full schema"
    );
    // Deferrable tools were stripped.
    let deferred = out.iter().find(|t| t.name() == "tool_0").unwrap();
    assert!(deferred.parameters().get("properties").is_none());
}

#[test]
fn test_never_defer_allowlist_keeps_full_schema() {
    // An embedder allowlist keeps a tool full even though its policy is
    // Automatic (i.e. the embedder does not own its definition).
    let cap = ToolSearchCapability::with_threshold(3).with_never_defer(["tool_1"]);
    let hooks = cap.tool_definition_hooks_with_context(&ctx_for(&[]), &json!({}));
    let out = hooks[0].transform(many_tools(5));

    let kept = out.iter().find(|t| t.name() == "tool_1").unwrap();
    assert!(
        !is_stubbed(kept),
        "allowlisted tool must keep its full schema"
    );
    let deferred = out.iter().find(|t| t.name() == "tool_0").unwrap();
    assert!(is_stubbed(deferred), "non-allowlisted tool must defer");
}

#[test]
fn test_config_never_defer_augments_constructor() {
    // Constructor allowlist plus a config-provided one are both honored.
    let cap = ToolSearchCapability::with_threshold(3).with_never_defer(["tool_0"]);
    let config = json!({ "never_defer": ["tool_2"] });
    let hooks = cap.tool_definition_hooks_with_context(&ctx_for(&[]), &config);
    let out = hooks[0].transform(many_tools(5));

    assert!(!is_stubbed(
        out.iter().find(|t| t.name() == "tool_0").unwrap()
    ));
    assert!(!is_stubbed(
        out.iter().find(|t| t.name() == "tool_2").unwrap()
    ));
    assert!(is_stubbed(
        out.iter().find(|t| t.name() == "tool_1").unwrap()
    ));
}

#[test]
fn test_config_threshold_override() {
    let cap = ToolSearchCapability::with_threshold(100);
    // Config lowers the threshold so deferral activates.
    let config = json!({ "threshold": 3 });
    let hooks = cap.tool_definition_hooks_with_context(&ctx_for(&[]), &config);
    let out = hooks[0].transform(many_tools(5));
    assert!(out.iter().any(is_stubbed));
}

#[test]
fn test_revealed_tool_regains_full_schema_next_pass() {
    // The end-to-end progressive-disclosure invariant: once the transcript
    // holds a search that loaded a tool, its *registered* schema (not just
    // the tool_search result text) is restored.
    let cap = ToolSearchCapability::with_threshold(3);
    let before = cap.tool_definition_hooks_with_context(&ctx_for(&[]), &json!({}))[0]
        .transform(many_tools(5));
    assert!(
        is_stubbed(before.iter().find(|t| t.name() == "tool_0").unwrap()),
        "precondition: tool_0 starts deferred"
    );

    let transcript = search_turn("call_1", json!({ "loaded": ["tool_0"] }));
    let after = cap.tool_definition_hooks_with_context(&ctx_for(&transcript), &json!({}))[0]
        .transform(many_tools(5));
    assert!(
        !is_stubbed(after.iter().find(|t| t.name() == "tool_0").unwrap()),
        "revealed tool must regain its full registered schema"
    );
    assert!(
        is_stubbed(after.iter().find(|t| t.name() == "tool_1").unwrap()),
        "unrevealed tools stay deferred"
    );
}

#[tokio::test]
async fn test_in_result_reveal_keeps_the_stub_and_says_so() {
    let cap = ToolSearchCapability::with_threshold(3);
    let config = json!({ "reveal": "in_result" });
    let transcript = search_turn("call_1", json!({ "loaded": ["tool_0"] }));
    let after = cap.tool_definition_hooks_with_context(&ctx_for(&transcript), &config)[0]
        .transform(many_tools(5));
    assert!(
        is_stubbed(after.iter().find(|t| t.name() == "tool_0").unwrap()),
        "a found tool keeps its stub, so the tool list never changes"
    );

    let ctx = ctx_for(&[]);
    let prompt = cap
        .system_prompt_contribution_with_config(&ctx, &config)
        .await
        .unwrap();
    assert!(prompt.contains("placeholder schema"));
    let default = cap
        .system_prompt_contribution_with_config(&ctx, &json!({}))
        .await
        .unwrap();
    assert_eq!(default, SYSTEM_PROMPT);

    let tool = cap.tools_with_config(&config).remove(0);
    let mut registry = crate::builtins::tools::ToolRegistry::new();
    registry.register(MiniTool);
    let mut tool_ctx = ToolContext::new(uuid::Uuid::new_v4().into());
    tool_ctx.tool_registry = Some(Arc::new(registry));
    tool_ctx.visible_tool_names = Some(Arc::new(["read_file".to_string()].into_iter().collect()));
    let ToolExecutionResult::Success(value) = tool
        .execute_with_context(json!({ "query": "file" }), &tool_ctx)
        .await
    else {
        panic!("expected success");
    };
    assert!(value["tools"][0]["parameters"]["properties"]["path"].is_object());
    assert!(
        value["message"]
            .as_str()
            .unwrap()
            .contains("follow the parameters shown here")
    );
}

#[test]
fn test_disclosure_reads_only_tool_search_results() {
    let mut transcript = search_turn(
        "call_1",
        json!({ "loaded": ["tool_0"], "loading_mcp_servers": ["docs"] }),
    );
    // Another tool's result that happens to carry `loaded` reveals nothing.
    let mut other = RuntimeMessage::assistant("");
    other.content = vec![ContentPart::ToolCall(
        everruns_contracts::runtime::message::ToolCallContentPart::new(
            "call_2",
            "read_file",
            json!({}),
        ),
    )];
    transcript.push(other);
    transcript.push(RuntimeMessage::tool_result(
        "call_2",
        Some(json!({ "loaded": ["tool_1"] })),
        None,
    ));

    let disclosure = tool_disclosure(&transcript);
    assert!(disclosure.deferring, "a search means the session deferred");
    assert_eq!(
        disclosure.revealed.into_iter().collect::<Vec<_>>(),
        ["mcp_docs__", "tool_0"]
    );
    assert_eq!(tool_disclosure(&[]), ToolDisclosure::default());
}

#[test]
fn test_hook_defers_mcp_tools_and_saves_full_schema() {
    // MCP tools are deferred like regular tools. The full schema is saved
    // in full_parameters so tool_search can return it on demand.
    let hook = hook(3);
    let mut tools = many_tools(3);
    tools.push(builtin(
        "mcp_docs__search",
        "search docs",
        DeferrablePolicy::Automatic,
    ));

    let out = hook.transform(tools);

    let mcp = out.iter().find(|t| t.name() == "mcp_docs__search").unwrap();
    // Stub is sent to the model (parameters stripped).
    assert!(
        mcp.parameters().get("properties").is_none(),
        "MCP tool schema is deferred"
    );
    // Full schema is preserved for tool_search to return.
    assert!(
        mcp.full_parameters().get("properties").is_some(),
        "MCP tool full schema is accessible via full_parameters()"
    );
}

#[test]
fn test_search_returns_full_schema_for_deferred_tools() {
    // After DeferSchemaHook strips parameters, tool_search must still return
    // the full schema (stored in full_parameters).
    let hook = hook(1);
    let tools = vec![builtin(
        "read_file",
        "Read a file",
        DeferrablePolicy::Automatic,
    )];
    let deferred = hook.transform(tools);

    let results = ToolSearchTool::search(&deferred, "read file");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["name"], "read_file");
    // full_parameters() is used, so real schema is returned — not the stub.
    assert!(
        results[0]["parameters"].get("properties").is_some(),
        "tool_search must return the full schema, not the deferred stub"
    );
}

#[test]
fn test_search_returns_full_schema_after_serde_round_trip() {
    // Durable execution serializes reason output before scheduling act. The
    // saved full schema must survive that boundary for deferred MCP proxies.
    let hook = hook(1);
    let tools = vec![builtin(
        "mcp_docs__search",
        "Search MCP docs",
        DeferrablePolicy::Automatic,
    )];
    let deferred = hook.transform(tools);
    let round_tripped: Vec<ToolDefinition> =
        serde_json::from_value(serde_json::to_value(&deferred).unwrap()).unwrap();

    let mcp = round_tripped
        .iter()
        .find(|t| t.name() == "mcp_docs__search")
        .unwrap();
    assert!(
        mcp.parameters().get("properties").is_none(),
        "visible MCP schema remains deferred after serde"
    );

    let results = ToolSearchTool::search(&round_tripped, "docs search");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["name"], "mcp_docs__search");
    assert!(
        results[0]["parameters"].get("properties").is_some(),
        "tool_search must return the full MCP schema after durable serde"
    );
}

#[test]
fn test_hook_opts_out_of_native_tool_search() {
    // Generic (client-side) deferral is mutually exclusive with hosted
    // tool_search; build() uses this to skip the hook when native is active.
    let hook = hook(15);
    assert!(!hook.applies_with_native_tool_search());
}

#[test]
fn test_search_ranks_by_keyword_overlap() {
    let defs = vec![
        builtin(
            "read_file",
            "Read the contents of a file",
            DeferrablePolicy::Automatic,
        ),
        builtin(
            "send_email",
            "Send an email message",
            DeferrablePolicy::Automatic,
        ),
        builtin(
            "write_file",
            "Write contents to a file",
            DeferrablePolicy::Automatic,
        ),
    ];

    let results = ToolSearchTool::search(&defs, "read file");
    assert_eq!(results[0]["name"], "read_file");
    // Full parameter schema is returned, not the stub.
    assert!(results[0]["parameters"].get("properties").is_some());

    let email = ToolSearchTool::search(&defs, "email");
    assert_eq!(email.len(), 1);
    assert_eq!(email[0]["name"], "send_email");
}

#[test]
fn test_search_weights_name_above_description() {
    // A name hit outranks a description-only hit for the same term, and the
    // weaker match falls outside the top score band entirely.
    let defs = vec![
        builtin("find_user", "Search the logs", DeferrablePolicy::Automatic),
        builtin("search_logs", "Find stuff", DeferrablePolicy::Automatic),
    ];
    let results = ToolSearchTool::search(&defs, "search");
    assert_eq!(results.len(), 1, "description-only match is below the band");
    assert_eq!(results[0]["name"], "search_logs");
}

#[test]
fn test_search_exact_name_match_dominates() {
    // Querying the exact tool name (as the deferred stub instructs) ranks that
    // tool first and drops near-duplicates whose name merely contains it.
    let defs = vec![
        builtin(
            "read_file_lines",
            "Read selected lines",
            DeferrablePolicy::Automatic,
        ),
        builtin("read_file", "Read a file", DeferrablePolicy::Automatic),
    ];
    let results = ToolSearchTool::search(&defs, "read_file");
    assert_eq!(results.len(), 1, "exact name match dominates the band");
    assert_eq!(results[0]["name"], "read_file");
}

#[test]
fn test_search_exact_name_match_tolerates_quoting() {
    // The deferred stub and system prompt show the tool name in quotes, so the
    // model may echo a quoted/backticked name. Wrapping punctuation must not
    // defeat the exact-name bonus (otherwise it degrades to a substring match
    // and reveals near-duplicates).
    let defs = vec![
        builtin(
            "read_file_lines",
            "Read selected lines",
            DeferrablePolicy::Automatic,
        ),
        builtin("read_file", "Read a file", DeferrablePolicy::Automatic),
    ];
    for q in ["\"read_file\"", "`read_file`", "'read_file'"] {
        let results = ToolSearchTool::search(&defs, q);
        assert_eq!(results.len(), 1, "query {q:?} should dominate");
        assert_eq!(results[0]["name"], "read_file", "query {q:?}");
    }
}

#[test]
fn test_search_caps_results_and_reveal_set() {
    // A loose query that matches many tools is capped, bounding both the
    // payload and the (sticky) reveal set.
    let mut defs = Vec::new();
    for i in 0..20 {
        defs.push(builtin(
            &format!("tool_{i}"),
            "does a thing",
            DeferrablePolicy::Automatic,
        ));
    }
    let results = ToolSearchTool::search(&defs, "thing");
    assert_eq!(results.len(), MAX_SEARCH_RESULTS);
}

#[test]
fn test_search_excludes_itself() {
    let defs = vec![
        ToolSearchTool::default().to_definition(),
        builtin("read_file", "Read a file", DeferrablePolicy::Automatic),
    ];
    let results = ToolSearchTool::search(&defs, "tool_search read");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["name"], "read_file");
}

#[tokio::test]
async fn test_execute_without_registry_errors() {
    let ctx = ToolContext::new(uuid::Uuid::new_v4().into());
    let result = ToolSearchTool::default()
        .execute_with_context(json!({ "query": "file" }), &ctx)
        .await;
    assert!(matches!(result, ToolExecutionResult::ToolError(_)));
}

struct MiniTool;
#[async_trait]
impl Tool for MiniTool {
    fn name(&self) -> &str {
        "read_file"
    }
    fn description(&self) -> &str {
        "Read the contents of a file"
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "path": { "type": "string" } },
            "required": ["path"]
        })
    }
    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::success(json!({}))
    }
}

#[tokio::test]
async fn test_execute_with_registry_returns_schemas() {
    use crate::builtins::tools::ToolRegistry;

    let mut registry = ToolRegistry::new();
    registry.register(MiniTool);
    registry.register(ToolSearchTool::default());

    let mut ctx = ToolContext::new(uuid::Uuid::new_v4().into());
    ctx.tool_registry = Some(Arc::new(registry));
    ctx.visible_tool_names = Some(Arc::new(
        ["read_file".to_string(), TOOL_SEARCH_TOOL_NAME.to_string()]
            .into_iter()
            .collect(),
    ));

    let result = ToolSearchTool::default()
        .execute_with_context(json!({ "query": "file" }), &ctx)
        .await;

    let ToolExecutionResult::Success(value) = result else {
        panic!("expected success");
    };
    let tools = value["tools"].as_array().unwrap();
    let read = tools.iter().find(|t| t["name"] == "read_file").unwrap();
    // Full schema is returned (not the deferred stub).
    assert!(read["parameters"]["properties"]["path"].is_object());
}

#[tokio::test]
async fn test_search_records_reveal_and_restores_registered_schema() {
    // The cross-cutting invariant EVE-527 asked for: a tool_search call
    // reveals the matched tool, and once its result is in the transcript
    // the hook restores the tool's registered schema.
    use crate::builtins::tools::ToolRegistry;

    let cap = ToolSearchCapability::with_threshold(3);
    let mut surface = many_tools(4);
    surface.push(builtin(
        "read_file",
        "Read the contents of a file",
        DeferrablePolicy::Automatic,
    ));
    let before = cap.tool_definition_hooks_with_context(&ctx_for(&[]), &json!({}))[0]
        .transform(surface.clone());
    assert!(is_stubbed(
        before.iter().find(|t| t.name() == "read_file").unwrap()
    ));

    // Run the capability's own tool_search tool.
    let mut registry = ToolRegistry::new();
    registry.register(MiniTool);
    let tool = &cap.tools()[0];
    let mut ctx = ToolContext::new(SessionId::new());
    ctx.tool_registry = Some(Arc::new(registry));
    ctx.visible_tool_names = Some(Arc::new(["read_file".to_string()].into_iter().collect()));

    let result = tool
        .execute_with_context(json!({ "query": "read file" }), &ctx)
        .await;
    let ToolExecutionResult::Success(value) = result else {
        panic!("expected success");
    };
    assert_eq!(value["loaded"][0], "read_file");

    // Next step: the stored result restores read_file's registered schema,
    // on whichever worker assembles the context.
    let transcript = search_turn("call_1", value);
    let after = cap.tool_definition_hooks_with_context(&ctx_for(&transcript), &json!({}))[0]
        .transform(surface);
    assert!(
        !is_stubbed(after.iter().find(|t| t.name() == "read_file").unwrap()),
        "revealed tool's registered schema must be restored after tool_search"
    );
}

struct HiddenTool;
#[async_trait]
impl Tool for HiddenTool {
    fn name(&self) -> &str {
        "write_file"
    }
    fn description(&self) -> &str {
        "Write contents to a file"
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "path": { "type": "string" } },
            "required": ["path"]
        })
    }
    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::success(json!({}))
    }
}

/// Measure the prompt-size reduction from deferral on a representative
/// agent surface without importing effectful capability packages. Prints a
/// breakdown (run with `--nocapture`) and guards that deferral keeps cutting
/// the tool-list size by a wide margin.
#[test]
fn benchmark_prompt_size_reduction() {
    let representative_names = [
        "read_file",
        "write_file",
        "edit_file",
        "grep_files",
        "list_directory",
        "shell",
        "web_fetch",
        "get_session_info",
        "write_session_title",
        "kv_store",
        "secret_store",
        "write_todos",
        "get_current_time",
        "spawn_agent",
        "send_message",
        "wait_agent",
        "create_schedule",
        "list_schedules",
    ];
    let mut defs: Vec<ToolDefinition> = representative_names
        .iter()
        .map(|name| {
            ToolDefinition::Builtin(BuiltinTool {
                name: (*name).to_string(),
                display_name: None,
                description: format!("Representative schema for {name}"),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "Target path" },
                        "query": { "type": "string", "description": "Search query" },
                        "content": { "type": "string", "description": "Input content" },
                        "offset": { "type": "integer", "minimum": 0 },
                        "limit": { "type": "integer", "minimum": 1 },
                        "recursive": { "type": "boolean" },
                        "metadata": { "type": "object", "additionalProperties": true }
                    },
                    "required": ["path"]
                }),
                policy: ToolPolicy::Auto,
                category: None,
                deferrable: DeferrablePolicy::Automatic,
                hints: ToolHints::default(),
                full_parameters: None,
            })
        })
        .collect();
    // Add the search tool itself, as the live capability does.
    defs.push(ToolSearchTool::default().to_definition());

    // What the driver serializes per tool: name + description + parameters.
    let llm_view = |defs: &[ToolDefinition]| -> usize {
        defs.iter()
            .map(|d| {
                json!({
                    "name": d.name(),
                    "description": d.description(),
                    "parameters": d.parameters(),
                })
                .to_string()
                .len()
            })
            .sum()
    };

    let total = defs.len();
    let full_bytes = llm_view(&defs);

    // Raw parameter-schema bytes (what stripping actually compresses) on the
    // full surface, before deferral.
    let params_full: usize = defs.iter().map(|d| d.parameters().to_string().len()).sum();

    // First model turn at the real default threshold: a surface this size is
    // above it, so every deferrable schema is stubbed.
    let threshold = DEFAULT_TOOL_SEARCH_THRESHOLD;
    let deferred = hook(threshold).transform(defs);
    let deferred_count = deferred.iter().filter(|d| is_stubbed(d)).count();
    let deferred_bytes = llm_view(&deferred);
    let params_deferred: usize = deferred
        .iter()
        .map(|d| d.parameters().to_string().len())
        .sum();

    // Deferral must shrink the surface; guard against underflow so a
    // regression fails with a clear message instead of a subtraction panic.
    assert!(
        deferred_bytes < full_bytes && params_deferred < params_full,
        "deferral must not grow the serialized surface \
             (tool list {full_bytes}->{deferred_bytes}, params {params_full}->{params_deferred})"
    );

    let saved = full_bytes - deferred_bytes;
    let pct = (saved as f64 / full_bytes as f64) * 100.0;
    let params_pct = ((params_full - params_deferred) as f64 / params_full as f64) * 100.0;
    // ~4 chars/token is the usual rule of thumb for JSON tool schemas.
    let approx_tokens_full = full_bytes / 4;
    let approx_tokens_deferred = deferred_bytes / 4;

    eprintln!("tool-search prompt-size benchmark");
    eprintln!("  tools on surface .......... {total}");
    eprintln!("  schemas deferred .......... {deferred_count}");
    eprintln!("  full tool list ............ {full_bytes} bytes (~{approx_tokens_full} tokens)");
    eprintln!(
        "  deferred tool list ........ {deferred_bytes} bytes (~{approx_tokens_deferred} tokens)"
    );
    eprintln!("  tool-list saved ........... {saved} bytes ({pct:.0}%)");
    eprintln!(
        "  parameter schemas ......... {params_full} -> {params_deferred} bytes ({params_pct:.0}% smaller)"
    );

    // Sanity guard: a many-tool surface must shrink substantially.
    assert!(
        total >= threshold,
        "surface should meet or exceed the default threshold ({total} < {threshold})"
    );
    assert!(
        pct > 45.0,
        "deferral should cut the whole tool list by a wide margin (was {pct:.0}%)"
    );
    assert!(
        params_pct > 70.0,
        "parameter schemas should compress substantially (was {params_pct:.0}%)"
    );
}

#[tokio::test]
async fn test_execute_filters_registry_to_visible_tools() {
    use crate::builtins::tools::ToolRegistry;

    let mut registry = ToolRegistry::new();
    registry.register(MiniTool);
    registry.register(HiddenTool);
    registry.register(ToolSearchTool::default());

    let mut ctx = ToolContext::new(uuid::Uuid::new_v4().into());
    ctx.tool_registry = Some(Arc::new(registry));
    ctx.visible_tool_names = Some(Arc::new(
        ["read_file".to_string(), TOOL_SEARCH_TOOL_NAME.to_string()]
            .into_iter()
            .collect(),
    ));

    let result = ToolSearchTool::default()
        .execute_with_context(json!({ "query": "file" }), &ctx)
        .await;

    let ToolExecutionResult::Success(value) = result else {
        panic!("expected success");
    };
    let tools = value["tools"].as_array().unwrap();
    assert!(tools.iter().any(|t| t["name"] == "read_file"));
    assert!(tools.iter().all(|t| t["name"] != "write_file"));

    let result = ToolSearchTool::default()
        .execute_with_context(json!({ "query": "missing" }), &ctx)
        .await;
    let ToolExecutionResult::Success(value) = result else {
        panic!("expected success");
    };
    let available = value["available_tools"].as_array().unwrap();
    assert!(available.iter().any(|name| name == "read_file"));
    assert!(available.iter().all(|name| name != "write_file"));
}

#[test]
fn rejected_tool_schema_is_recovered_from_the_durable_transcript() {
    let mut call = RuntimeMessage::assistant("");
    call.content = vec![ContentPart::ToolCall(
        everruns_contracts::runtime::message::ToolCallContentPart::new(
            "failed-call",
            "tool_0",
            json!({}),
        ),
    )];
    let transcript = vec![
        call,
        RuntimeMessage::tool_result("failed-call", None, Some("invalid_tool_arguments".into())),
    ];
    let cap = ToolSearchCapability::with_threshold(1);
    for config in [json!({}), json!({"reveal": "in_result"})] {
        let hooks = cap.tool_definition_hooks_with_context(&ctx_for(&transcript), &config);
        let out = hooks[0].transform(many_tools(2));
        assert!(
            !is_stubbed(&out[0]),
            "failed known calls need their authoritative schema"
        );
        assert!(is_stubbed(&out[1]), "other tools remain deferred");
        let empty = cap.tool_definition_hooks_with_context(&ctx_for(&[]), &config);
        assert!(
            empty[0].transform(many_tools(2)).iter().all(is_stubbed),
            "other sessions do not inherit the failure"
        );
    }
}

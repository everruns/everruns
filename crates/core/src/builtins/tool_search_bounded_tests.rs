use super::*;
use everruns_contracts::runtime::capabilities::{CapabilityRegistry, ToolDisclosure};
use everruns_contracts::runtime::runtime_agent::RuntimeAgentBuilder;
use everruns_contracts::runtime::tools::ToolRegistry;
use everruns_contracts::typed_id::SessionId;

struct CatalogTool(ToolDefinition);

#[async_trait]
impl Tool for CatalogTool {
    fn name(&self) -> &str {
        self.0.name()
    }
    fn description(&self) -> &str {
        self.0.description()
    }
    fn parameters_schema(&self) -> Value {
        self.0.parameters().clone()
    }
    fn to_definition(&self) -> ToolDefinition {
        self.0.clone()
    }
    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::success(arguments)
    }
}

fn builtin(
    name: impl Into<String>,
    description: impl Into<String>,
    parameters: Value,
) -> ToolDefinition {
    ToolDefinition::Builtin(crate::builtins::tool_types::BuiltinTool {
        name: name.into(),
        display_name: None,
        description: description.into(),
        parameters,
        policy: Default::default(),
        category: None,
        deferrable: Default::default(),
        hints: Default::default(),
        full_parameters: None,
    })
}

fn definitions(count: usize) -> Vec<ToolDefinition> {
    (0..count)
        .map(|index| {
            builtin(
                format!("record_{index:05}"),
                "Read one record by its identifier",
                json!({"type":"object","properties": {
            "id":{"type":"integer"}},"required":["id"],"additionalProperties":false}),
            )
        })
        .collect()
}

fn context(definitions: &[ToolDefinition]) -> ToolContext {
    let mut registry = ToolRegistry::new();
    for definition in definitions {
        registry.register(CatalogTool(definition.clone()));
    }
    ToolContext::new(SessionId::new())
        .with_tool_registry(Arc::new(registry))
        .with_visible_tool_names(Arc::new(
            definitions
                .iter()
                .map(|tool| tool.name().to_owned())
                .collect(),
        ))
}

fn hook(disclosure: ToolDisclosure) -> DeferSchemaHook {
    DeferSchemaHook {
        threshold: 1,
        never_defer: Arc::new(HashSet::new()),
        reveal: Reveal::Redefine,
        disclosure,
        limits: Some(CatalogLimits {
            active_tools: 32,
            active_bytes: 65536,
            result_bytes: 32768,
        }),
    }
}

#[test]
fn bounded_config_rejects_unbounded_limits_and_untyped_reveal() {
    let capability = ToolSearchCapability::new();
    for invalid in [
        json!({"bounded":true,"reveal":"in_result"}),
        json!({"max_active_tools":0}),
        json!({"max_active_tools":129}),
        json!({"max_result_bytes":10000000}),
        json!({"bounded":"yes"}),
    ] {
        assert!(capability.validate_config(&invalid).is_err(), "{invalid}");
    }
    assert!(capability.validate_config(&json!({"bounded":true})).is_ok());
}

#[test]
fn owner_required_native_entry_points_stay_eager_without_schema_policy_hints() {
    let definitions: Vec<_> = [
        "bash",
        "ask_user",
        "request_approval",
        "generate_image",
        "record_00000",
    ]
    .into_iter()
    .map(|name| builtin(name, "Operation", json!({"type":"object"})))
    .collect();
    let projected = hook(ToolDisclosure::default()).transform(definitions);
    assert_eq!(projected.len(), 4);
    assert!(projected.iter().all(|tool| tool.name() != "record_00000"));
}

#[tokio::test]
async fn initial_surface_stays_constant_at_ten_thousand_tools_and_retains_execution_scope() {
    let mut registry = CapabilityRegistry::new();
    registry.register(ToolSearchCapability::new());
    for count in [50, 600, 2000, 10000] {
        let runtime = RuntimeAgentBuilder::new()
            .tools(definitions(count))
            .with_capability_configs(
                &[crate::AgentCapabilityConfig::with_config(
                    "tool_search",
                    json!({"bounded":true}),
                )],
                &registry,
                &SystemPromptContext::without_file_store(SessionId::new()),
            )
            .await
            .build();
        assert_eq!(runtime.tools.len(), 1);
        assert_eq!(runtime.tools[0].name(), TOOL_SEARCH_TOOL_NAME);
        assert_eq!(runtime.catalog_tools.len(), count);
        assert_eq!(runtime.execution_tools().len(), count + 1);
    }
}

#[tokio::test]
async fn actual_search_obeys_count_and_bytes_and_never_dumps_the_catalog() {
    let definitions = definitions(2000);
    let context = context(&definitions);
    let tool = ToolSearchTool {
        reveal: Reveal::Redefine,
        limits: Some(CatalogLimits {
            active_tools: 32,
            active_bytes: 65536,
            result_bytes: 1024,
        }),
    };
    for query in ["record", "unrelatedvolcanicphenomenon"] {
        let ToolExecutionResult::Success(result) = tool
            .execute_with_context(json!({"query":query}), &context)
            .await
        else {
            panic!("bounded search should return recoverable results");
        };
        assert!(serde_json::to_vec(&result).unwrap().len() <= 1024);
        assert!(result["tools"].as_array().unwrap().len() <= 8);
        assert!(result.get("available_tools").is_none());
        if query == "unrelatedvolcanicphenomenon" {
            assert!(result["tools"].as_array().unwrap().is_empty());
        }
    }
}

#[tokio::test]
async fn oversized_schema_is_not_reported_as_loaded() {
    let oversized = builtin(
        "giant_record",
        "Read a record",
        json!({"type":"object",
        "description":"large".repeat(20000),"properties":{"id":{"type":"integer"}}}),
    );
    let context = context(&[oversized]);
    let tool = ToolSearchTool {
        reveal: Reveal::Redefine,
        limits: Some(CatalogLimits {
            active_tools: 32,
            active_bytes: 65536,
            result_bytes: 32768,
        }),
    };
    let ToolExecutionResult::Success(result) = tool
        .execute_with_context(json!({"query":"giant_record"}), &context)
        .await
    else {
        panic!("search failed")
    };
    assert!(result["loaded"].as_array().unwrap().is_empty());
    assert_eq!(result["omitted_for_byte_budget"], 1);
    assert_eq!(result["omitted_tools"], json!(["giant_record"]));
}

#[tokio::test]
async fn server_description_only_search_browses_source_without_promoting_arbitrary_operations() {
    let mut definitions = definitions(600);
    for (index, definition) in definitions.iter_mut().enumerate() {
        if let ToolDefinition::Builtin(tool) = definition {
            tool.name = format!("mcp_atlas__record_{index:05}");
            tool.hints.metadata = Some(json!({"mcp_server":{
                "name":"atlas","description":"Warehouse fulfillment inventory"}}));
        }
    }
    let tool = ToolSearchTool {
        reveal: Reveal::Redefine,
        limits: Some(CatalogLimits {
            active_tools: 32,
            active_bytes: 65536,
            result_bytes: 1024,
        }),
    };
    let context = context(&definitions);
    for query in ["warehouse fulfillment", ""] {
        let ToolExecutionResult::Success(result) = tool
            .execute_with_context(json!({"query":query}), &context)
            .await
        else {
            panic!("search failed")
        };
        assert!(result["loaded"].as_array().unwrap().is_empty());
        assert!(result["tools"].as_array().unwrap().is_empty());
        assert_eq!(result["available_servers"], json!(["atlas"]));
        assert!(serde_json::to_vec(&result).unwrap().len() <= 1024);
    }
}

#[test]
fn projection_byte_bound_includes_array_and_separator_overhead() {
    let definitions = definitions(2);
    let disclosure = ToolDisclosure {
        reveal_order: definitions
            .iter()
            .map(|tool| tool.name().to_owned())
            .collect(),
        ..Default::default()
    };
    let mut hook = hook(disclosure);
    let limit = serde_json::to_vec(&definitions).unwrap().len() - 1;
    hook.limits.as_mut().unwrap().active_bytes = limit;
    let selected = hook.transform(definitions);
    assert_eq!(selected.len(), 1);
    assert!(serde_json::to_vec(&selected).unwrap().len() <= limit);
}

#[test]
fn hundred_searches_advertise_only_recent_schemas_and_use_current_definitions() {
    let mut messages = Vec::new();
    for index in 0..100 {
        let id = format!("search-{index}");
        let mut request = RuntimeMessage::assistant("");
        request.content = vec![ContentPart::ToolCall(
            everruns_contracts::runtime::message::ToolCallContentPart::new(
                &id,
                TOOL_SEARCH_TOOL_NAME,
                json!({"query":format!("record_{index:05}")}),
            ),
        )];
        messages.push(request);
        messages.push(RuntimeMessage::tool_result(
            &id,
            Some(
                json!({"bounded":true,"loaded":[format!("record_{index:05}")],
            "loading_mcp_servers":["platform"]}),
            ),
            None,
        ));
    }
    let disclosure = tool_disclosure(&messages);
    assert!(!disclosure.revealed.contains("mcp_platform__"));
    let mut current = definitions(2000);
    current[99] = builtin(
        "record_00099",
        "Updated schema",
        json!({"type":"object",
        "properties":{"id":{"type":"string"}},"required":["id"]}),
    );
    let visible = hook(disclosure).transform(current);
    assert_eq!(visible.len(), 32);
    assert!(visible.iter().all(|tool| tool.name() >= "record_00068"));
    assert_eq!(
        visible
            .iter()
            .find(|tool| tool.name() == "record_00099")
            .unwrap()
            .parameters()["properties"]["id"]["type"],
        "string"
    );
    assert!(serde_json::to_vec(&visible).unwrap().len() <= 65536);
}

#[test]
fn hosted_text_results_restore_bounded_and_legacy_deferred_disclosures() {
    fn transcript_payload(payload: String) -> Vec<RuntimeMessage> {
        let call_id = "search-1";
        let mut request = RuntimeMessage::assistant("");
        request.content = vec![ContentPart::ToolCall(
            everruns_contracts::runtime::message::ToolCallContentPart::new(
                call_id,
                TOOL_SEARCH_TOOL_NAME,
                json!({"query":"incident"}),
            ),
        )];
        vec![
            request,
            RuntimeMessage::tool_result(call_id, Some(Value::String(payload)), None),
        ]
    }
    let transcript = |result: Value| transcript_payload(serde_json::to_string(&result).unwrap());

    let bounded_name = "mcp_telemetry__get_incident";
    let bounded = tool_disclosure(&transcript(json!({
        "bounded": true,
        "loaded": [bounded_name],
        "loading_mcp_servers": []
    })));
    assert!(bounded.revealed.contains(bounded_name));
    let definitions = vec![
        builtin(bounded_name, "Get incident", json!({"type":"object"})),
        builtin(
            "mcp_telemetry__get_ticket",
            "Get ticket",
            json!({"type":"object"}),
        ),
    ];
    let bounded_visible = hook(bounded).transform(definitions.clone());
    assert_eq!(bounded_visible.len(), 1);
    assert_eq!(bounded_visible[0].name(), bounded_name);

    // Older, unbounded search results revealed a whole MCP server namespace.
    let legacy = tool_disclosure(&transcript(json!({
        "loaded": [],
        "loading_mcp_servers": ["telemetry"]
    })));
    assert!(legacy.revealed.contains("mcp_telemetry__"));
    let mut legacy_hook = hook(legacy);
    legacy_hook.limits = None;
    let legacy_visible = legacy_hook.transform(definitions.clone());
    assert_eq!(legacy_visible.len(), 2);
    assert!(
        legacy_visible
            .iter()
            .all(|tool| { tool.parameters().get("properties").is_none() })
    );

    for ignored in ["ordinary text", "{malformed", "[\"not an object\"]"] {
        let disclosure = tool_disclosure(&transcript_payload(ignored.into()));
        assert!(
            disclosure.revealed.is_empty(),
            "unexpected reveal from {ignored}"
        );
    }
}

struct RestrictingHook;
impl ToolDefinitionHook for RestrictingHook {
    fn transform(&self, tools: Vec<ToolDefinition>) -> Vec<ToolDefinition> {
        tools
            .into_iter()
            .filter(|tool| tool.name() != "record_00017")
            .collect()
    }
}

struct RestrictingCapability;
#[async_trait]
impl Capability for RestrictingCapability {
    fn id(&self) -> &str {
        "catalog_test_restriction"
    }
    fn name(&self) -> &str {
        "Restriction"
    }
    fn description(&self) -> &str {
        "Restrict one operation"
    }
    fn tool_definition_hooks(&self) -> Vec<Arc<dyn ToolDefinitionHook>> {
        vec![Arc::new(RestrictingHook)]
    }
}

#[tokio::test]
async fn later_restrictions_apply_to_hidden_catalog_scope_too() {
    let mut registry = CapabilityRegistry::new();
    registry.register(ToolSearchCapability::new());
    registry.register(RestrictingCapability);
    let runtime = RuntimeAgentBuilder::new()
        .tools(definitions(2000))
        .with_capability_configs(
            &[
                crate::AgentCapabilityConfig::with_config("tool_search", json!({"bounded":true})),
                crate::AgentCapabilityConfig::new("catalog_test_restriction"),
            ],
            &registry,
            &SystemPromptContext::without_file_store(SessionId::new()),
        )
        .await
        .build();
    assert!(
        !runtime
            .execution_tools()
            .iter()
            .any(|tool| tool.name() == "record_00017")
    );
    assert_eq!(runtime.catalog_tools.len(), 1999);
}

#[tokio::test]
async fn count_dependent_restriction_runs_once_over_the_complete_scope() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct LimitHook(Arc<AtomicUsize>);
    impl ToolDefinitionHook for LimitHook {
        fn transform(&self, mut tools: Vec<ToolDefinition>) -> Vec<ToolDefinition> {
            self.0.fetch_add(1, Ordering::SeqCst);
            tools.truncate(2);
            tools
        }
    }
    struct LimitCapability(Arc<AtomicUsize>);
    impl Capability for LimitCapability {
        fn id(&self) -> &str {
            "scope_limit_test"
        }
        fn name(&self) -> &str {
            "Scope limit"
        }
        fn description(&self) -> &str {
            "Limit the complete scope"
        }
        fn tool_definition_hooks(&self) -> Vec<Arc<dyn ToolDefinitionHook>> {
            vec![Arc::new(LimitHook(self.0.clone()))]
        }
    }
    let calls = Arc::new(AtomicUsize::new(0));
    let mut registry = CapabilityRegistry::new();
    registry.register(ToolSearchCapability::new());
    registry.register(LimitCapability(calls.clone()));
    let runtime = RuntimeAgentBuilder::new()
        .tools(definitions(50))
        .with_capability_configs(
            &[
                crate::AgentCapabilityConfig::with_config("tool_search", json!({"bounded":true})),
                crate::AgentCapabilityConfig::new("scope_limit_test"),
            ],
            &registry,
            &SystemPromptContext::without_file_store(SessionId::new()),
        )
        .await
        .build();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(runtime.execution_tools().len(), 2);
}

#[tokio::test]
async fn capability_only_queries_refine_without_disclosing_arbitrary_tools_or_hidden_sources() {
    let mut definitions = definitions(40);
    for definition in &mut definitions {
        let mut hints = definition.hints().clone();
        hints.capability_id = Some("ledger".into());
        hints.capability_name = Some("Accounting".into());
        hints.capability_description = Some("Obsidian settlement reconciliation".into());
        *definition = definition.clone().with_hints(hints);
    }
    let mut hidden = builtin("hidden_record", "Read a record", json!({"type":"object"}));
    let mut hints = hidden.hints().clone();
    hints.capability_id = Some("private_ledger".into());
    hints.capability_description = Some("Obsidian confidential".into());
    hidden = hidden.with_hints(hints);
    definitions.push(hidden);
    let mut ctx = context(&definitions);
    ctx.visible_tool_names = Some(Arc::new(
        definitions
            .iter()
            .filter(|d| d.name() != "hidden_record")
            .map(|d| d.name().to_owned())
            .collect(),
    ));
    let search = ToolSearchTool {
        reveal: Reveal::Redefine,
        limits: Some(CatalogLimits {
            active_tools: 32,
            active_bytes: 65536,
            result_bytes: 32768,
        }),
    };
    let ToolExecutionResult::Success(result) = search
        .execute_with_context(json!({"query":"obsidian"}), &ctx)
        .await
    else {
        panic!("search failed")
    };
    assert_eq!(result["tools"], json!([]));
    assert_eq!(result["loaded"], json!([]));
    assert_eq!(result["available_capabilities"], json!(["ledger"]));
    let ToolExecutionResult::Success(result) = search
        .execute_with_context(json!({"query":"obsidian record_00001"}), &ctx)
        .await
    else {
        panic!("search failed")
    };
    assert!(result["tools"].as_array().unwrap().len() <= 8);
    assert!(!result.to_string().contains("private_ledger"));
}

#[tokio::test]
async fn capability_only_deferred_matches_identify_the_server_for_refinement() {
    let definition = crate::deferred_mcp_server_definition("archive", Some("Record access"))
        .with_capability_attribution("plugin:opaque", Some("Research"))
        .with_capability_description("Obsidian observatory");
    let search = ToolSearchTool {
        reveal: Reveal::Redefine,
        limits: Some(CatalogLimits {
            active_tools: 32,
            active_bytes: 65536,
            result_bytes: 32768,
        }),
    };
    let ToolExecutionResult::Success(result) = search
        .execute_with_context(json!({"query":"obsidian"}), &context(&[definition]))
        .await
    else {
        panic!("search failed")
    };
    assert_eq!(result["tools"], json!([]));
    assert_eq!(result["loaded"], json!([]));
    assert_eq!(result["available_servers"], json!(["archive"]));
    assert_eq!(result["available_capabilities"], json!(["plugin:opaque"]));
}

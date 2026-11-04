use super::*;
use everruns_contracts::runtime::capabilities::{CapabilityRegistry, ToolDisclosure};
use everruns_contracts::runtime::runtime_agent::RuntimeAgentBuilder;
use everruns_contracts::runtime::tools::ToolRegistry;
use everruns_contracts::typed_id::SessionId;

struct CatalogTool(ToolDefinition);

#[tokio::test]
async fn explicit_bounded_search_supersedes_inherited_discovery() {
    use crate::builtins::{
        AutoToolSearchCapability, ClaudeToolSearchCapability, OpenAiToolSearchCapability,
    };
    let mut registry = CapabilityRegistry::new();
    registry.register(ToolSearchCapability::new());
    registry.register(AutoToolSearchCapability::new());
    registry.register(OpenAiToolSearchCapability::new());
    registry.register(ClaudeToolSearchCapability::new());
    for inherited in [
        "auto_tool_search",
        "openai_tool_search",
        "claude_tool_search",
    ] {
        for model in [
            "gpt-5.4",
            "meta/muse-spark-1.3-contributor",
            "claude-opus-4-8",
        ] {
            for bounded_first in [false, true] {
                let mut configs = vec![
                    crate::AgentCapabilityConfig::new(inherited),
                    crate::AgentCapabilityConfig::with_config(
                        "tool_search",
                        json!({"bounded":true,"threshold":1}),
                    ),
                ];
                if bounded_first {
                    configs.reverse();
                }
                let mut ctx = SystemPromptContext::without_file_store(SessionId::new());
                ctx.model = Some(model.into());
                let runtime = RuntimeAgentBuilder::new()
                    .tools(definitions(2000))
                    .with_capability_configs(&configs, &registry, &ctx)
                    .await
                    .model(model)
                    .build();
                assert!(runtime.tool_search.is_none(), "{inherited}/{model}");
                assert_eq!(runtime.tools.len(), 1, "{inherited}/{model}");
                assert_eq!(runtime.tools[0].name(), "tool_search");
                assert_eq!(runtime.catalog_tools.len(), 2000);
                assert_eq!(runtime.execution_tools().len(), 2001);
            }
        }
    }
}

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

fn capability_tool(name: &str, id: &str, title: &str, purpose: &str) -> ToolDefinition {
    let mut definition = builtin(name, "Process one item", json!({"type":"object"}));
    let ToolDefinition::Builtin(tool) = &mut definition else {
        unreachable!()
    };
    tool.hints.capability_id = Some(id.to_owned());
    tool.hints.capability_name = Some(title.to_owned());
    tool.hints.capability_description = Some(purpose.to_owned());
    definition
}

fn bounded_tool(result_bytes: usize) -> ToolSearchTool {
    ToolSearchTool {
        reveal: Reveal::Redefine,
        limits: Some(CatalogLimits {
            active_tools: 32,
            active_bytes: 65536,
            result_bytes,
        }),
    }
}

#[tokio::test]
async fn capability_purpose_refines_without_loading_opaque_operations() {
    let definitions: Vec<_> = (0..600)
        .map(|position| {
            capability_tool(
                &format!("opaque_{position:04}"),
                "stockroom",
                "Stockroom",
                "Warehouse fulfillment inventory",
            )
        })
        .collect();
    let context = context(&definitions);
    let tool = bounded_tool(1024);
    let ToolExecutionResult::Success(result) = tool
        .execute_with_context(json!({"query":"warehouse fulfillment"}), &context)
        .await
    else {
        panic!("source refinement should fit")
    };
    assert_eq!(result["tools"], json!([]));
    assert_eq!(result["loaded"], json!([]));
    assert_eq!(
        result["available_capabilities"],
        json!([{"id":"stockroom","name":"Stockroom"}])
    );
    assert!(serde_json::to_vec(&result).unwrap().len() <= 1024);
    assert!(!serde_json::to_string(&result).unwrap().contains("opaque_"));

    let ToolExecutionResult::Success(refined) = tool
        .execute_with_context(json!({"query":"opaque_0017"}), &context)
        .await
    else {
        panic!("exact operation refinement should fit")
    };
    assert_eq!(refined["loaded"], json!(["opaque_0017"]));
    assert!(refined["tools"][0].get("parameters").is_none());
    let mut request = RuntimeMessage::assistant("");
    request.content = vec![ContentPart::ToolCall(
        everruns_contracts::runtime::message::ToolCallContentPart::new(
            "refinement",
            TOOL_SEARCH_TOOL_NAME,
            json!({"query":"opaque_0017"}),
        ),
    )];
    let disclosure = tool_disclosure(&[
        request,
        RuntimeMessage::tool_result("refinement", Some(refined), None),
    ]);
    let projected = hook(disclosure).transform(definitions);
    assert_eq!(projected.len(), 1);
    assert_eq!(projected[0].name(), "opaque_0017");
    assert_eq!(projected[0].full_parameters(), &json!({"type":"object"}));
}

#[tokio::test]
async fn mixed_source_refinements_and_fallbacks_share_scope_count_and_byte_bounds() {
    let mut definitions = Vec::new();
    for position in 0..24 {
        definitions.push(capability_tool(
            &format!("opaque_{position:04}"),
            &format!("stockroom_{position:02}"),
            &format!("Stockroom {position:02}"),
            "Warehouse fulfillment inventory",
        ));
        let mut server = builtin(
            format!("mcp_atlas_{position:02}__opaque"),
            "Process one item",
            json!({"type":"object"}),
        );
        let ToolDefinition::Builtin(tool) = &mut server else {
            unreachable!()
        };
        tool.hints.metadata = Some(json!({"mcp_server":{
            "name":format!("atlas_{position:02}"),"description":"Warehouse fulfillment inventory"}}));
        definitions.push(server);
    }
    definitions.push(capability_tool(
        "private_opaque",
        "private_capability",
        "Private",
        "Secret documents",
    ));
    let mut context = context(&definitions);
    Arc::make_mut(context.visible_tool_names.as_mut().unwrap()).remove("private_opaque");
    for limit in [1024, 32768] {
        for query in [
            "warehouse fulfillment",
            "unknownphenomenon",
            "secret documents",
        ] {
            let ToolExecutionResult::Success(result) = bounded_tool(limit)
                .execute_with_context(json!({"query":query}), &context)
                .await
            else {
                panic!("source hints should fit")
            };
            assert_eq!(result["tools"], json!([]));
            assert_eq!(result["loaded"], json!([]));
            let servers = result["available_servers"].as_array().unwrap();
            let capabilities = result
                .get("available_capabilities")
                .and_then(Value::as_array);
            assert!(servers.len() + capabilities.map_or(0, Vec::len) <= 16);
            assert!(serde_json::to_vec(&result).unwrap().len() <= limit);
            let output = serde_json::to_string(&result).unwrap();
            assert!(!output.contains("private_capability"));
            assert!(!output.contains("private_opaque"));
            if query == "unknownphenomenon" {
                assert!(!servers.is_empty());
                assert!(!capabilities.unwrap().is_empty());
            }
        }
    }
}

#[tokio::test]
async fn capability_browse_hints_cannot_crowd_out_omitted_operation_refinement() {
    let mut definition = capability_tool(
        "opaque_zeta",
        "stockroom",
        &"界".repeat(400),
        "Warehouse fulfillment inventory",
    );
    let ToolDefinition::Builtin(tool) = &mut definition else {
        unreachable!()
    };
    tool.parameters = json!({"type":"object","description":"x".repeat(2000)});
    let context = context(&[definition]);
    let mut search = bounded_tool(1024);
    search.limits.as_mut().unwrap().active_bytes = 1024;
    let ToolExecutionResult::Success(result) = search
        .execute_with_context(json!({"query":"opaque_zeta"}), &context)
        .await
    else {
        panic!("omitted operation hint should fit")
    };
    assert_eq!(result["loaded"], json!([]));
    assert_eq!(result["omitted_tools"], json!(["opaque_zeta"]));
    assert_eq!(result["omitted_for_byte_budget"], 1);
    assert!(result.get("available_capabilities").is_none());
    assert!(serde_json::to_vec(&result).unwrap().len() <= 1024);
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

fn project_search_result(
    query: &str,
    result: Value,
    definitions: Vec<ToolDefinition>,
) -> Vec<ToolDefinition> {
    let mut request = RuntimeMessage::assistant("");
    request.content = vec![ContentPart::ToolCall(
        everruns_contracts::runtime::message::ToolCallContentPart::new(
            "search",
            TOOL_SEARCH_TOOL_NAME,
            json!({"query":query}),
        ),
    )];
    hook(tool_disclosure(&[
        request,
        RuntimeMessage::tool_result("search", Some(result), None),
    ]))
    .transform(definitions)
}

#[tokio::test]
async fn schema_larger_than_search_result_budget_is_delivered_as_a_full_native_function() {
    let schema = json!({
        "type":"object", "additionalProperties":false, "required":["id"],
        "$defs":{"identifier":{"type":"string","enum":["kept"]}},
        "properties":{"id":{"$ref":"#/$defs/identifier"}},
        "description":"x".repeat(40000)
    });
    let definitions = vec![builtin("large_record", "Read a record", schema.clone())];
    let ToolExecutionResult::Success(result) = bounded_tool(1024)
        .execute_with_context(json!({"query":"large_record"}), &context(&definitions))
        .await
    else {
        panic!("native schema fits even though search text has a smaller budget")
    };
    assert_eq!(result["loaded"], json!(["large_record"]));
    assert!(result["tools"][0].get("parameters").is_none());
    assert!(serde_json::to_vec(&result).unwrap().len() <= 1024);
    let projected = project_search_result("large_record", result, definitions);
    assert_eq!(projected.len(), 1);
    assert_eq!(projected[0].full_parameters(), &schema);
    assert!(serde_json::to_vec(&projected).unwrap().len() > 32768);
    assert!(serde_json::to_vec(&projected).unwrap().len() <= 65536);
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
        // The native baseline contains no catalog operation schemas. The
        // complete definitions remain available to the execution registry.
        assert_eq!(
            runtime.tools[0].parameters()["properties"]["query"]["type"],
            "string"
        );
        assert!(
            !serde_json::to_string(&runtime.tools)
                .unwrap()
                .contains("record_00000")
        );
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
async fn exact_search_discloses_only_the_selected_schema_at_two_thousand_and_ten_thousand() {
    for count in [2000, 10000] {
        let definitions = definitions(count);
        let context = context(&definitions);
        let tool = ToolSearchTool {
            reveal: Reveal::Redefine,
            limits: Some(CatalogLimits {
                active_tools: 32,
                active_bytes: 65536,
                result_bytes: 32768,
            }),
        };
        let ToolExecutionResult::Success(result) = tool
            .execute_with_context(json!({"query":"record_01999"}), &context)
            .await
        else {
            panic!("bounded search should return the selected schema");
        };
        assert_eq!(result["loaded"], json!(["record_01999"]));
        assert_eq!(result["tools"].as_array().unwrap().len(), 1);
        assert_eq!(result["tools"][0]["name"], "record_01999");
        assert!(
            !serde_json::to_string(&result)
                .unwrap()
                .contains("record_02000")
        );
    }
}

#[tokio::test]
async fn bounded_search_never_returns_an_exact_match_outside_the_current_allowlist() {
    let definitions = definitions(2000);
    let mut context = context(&definitions);
    context.visible_tool_names = Some(Arc::new(HashSet::from(["record_00000".to_owned()])));
    let tool = ToolSearchTool {
        reveal: Reveal::Redefine,
        limits: Some(CatalogLimits {
            active_tools: 32,
            active_bytes: 65536,
            result_bytes: 32768,
        }),
    };
    let ToolExecutionResult::Success(result) = tool
        .execute_with_context(json!({"query":"record_01999"}), &context)
        .await
    else {
        panic!("scoped search should return bounded authorized results");
    };
    let loaded = result["loaded"].as_array().unwrap();
    assert!(loaded.iter().all(|name| name == "record_00000"));
    assert!(
        result["tools"]
            .as_array()
            .unwrap()
            .iter()
            .all(|tool| tool["name"] == "record_00000")
    );
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
}

#[tokio::test]
async fn byte_omitted_operation_names_enable_exact_refinement_without_leaking_scope() {
    let definitions = vec![
        builtin(
            "update_agent_script",
            "Update an agent description.",
            json!({"type":"object","description":"x".repeat(3000)}),
        ),
        builtin(
            "update_agent",
            "Update an agent.",
            json!({"type":"object","description":"x".repeat(31000)}),
        ),
        builtin(
            "update_private_agent",
            "Update an agent description.",
            json!({"type":"object"}),
        ),
    ];
    let mut context = context(&definitions);
    context.visible_tool_names = Some(Arc::new(HashSet::from([
        "update_agent_script".to_owned(),
        "update_agent".to_owned(),
    ])));
    let tool = ToolSearchTool {
        reveal: Reveal::Redefine,
        limits: Some(CatalogLimits {
            active_tools: 32,
            active_bytes: 32768,
            result_bytes: 32768,
        }),
    };
    let ToolExecutionResult::Success(result) = tool
        .execute_with_context(json!({"query":"update agent description"}), &context)
        .await
    else {
        panic!("search failed")
    };
    assert_eq!(result["loaded"], json!(["update_agent_script"]));
    assert_eq!(result["omitted_for_byte_budget"], 1);
    assert_eq!(result["omitted_tools"], json!(["update_agent"]));
    assert!(serde_json::to_vec(&result).unwrap().len() <= 32768);
    assert!(
        !serde_json::to_string(&result)
            .unwrap()
            .contains("update_private_agent")
    );

    let ToolExecutionResult::Success(refined) = tool
        .execute_with_context(json!({"query":result["omitted_tools"][0]}), &context)
        .await
    else {
        panic!("exact refinement failed")
    };
    assert_eq!(refined["loaded"], json!(["update_agent"]));
    assert!(refined["tools"][0].get("parameters").is_none());
    let projected = project_search_result("update_agent", refined.clone(), definitions.clone());
    assert_eq!(
        projected[0].full_parameters(),
        definitions[1].full_parameters()
    );
    assert!(refined.get("omitted_tools").is_none());
    assert!(serde_json::to_vec(&refined).unwrap().len() <= 32768);
}

#[tokio::test]
async fn exact_summary_at_result_byte_boundary_keeps_its_entire_budget() {
    let name = "edge_record";
    let mut description = String::new();
    let schema = json!({"type":"object"});
    let envelope = json!({
        "query":name,"tools":[{"name":name,"description":description,
            "server_match":false,"operation_match":true}],
        "loaded":[name],"bounded":true,"loading_mcp_servers":[],"omitted_for_byte_budget":0,
        "message":"Selected schemas become native functions next step. After loading a server, search again for its operation. Refine source-only or oversized matches."
    });
    description = "x".repeat(32768 - serde_json::to_vec(&envelope).unwrap().len());
    let definitions = vec![builtin(name, description, schema.clone())];
    let context = context(&definitions);
    let tool = ToolSearchTool {
        reveal: Reveal::Redefine,
        limits: Some(CatalogLimits {
            active_tools: 32,
            active_bytes: 65536,
            result_bytes: 32768,
        }),
    };
    let ToolExecutionResult::Success(result) = tool
        .execute_with_context(json!({"query":name}), &context)
        .await
    else {
        panic!("exact summary at the byte boundary should fit")
    };
    assert_eq!(result["loaded"], json!([name]));
    assert!(result["tools"][0].get("parameters").is_none());
    let projected = project_search_result(name, result.clone(), definitions);
    assert_eq!(projected[0].full_parameters(), &schema);
    assert!(result.get("omitted_tools").is_none());
    assert_eq!(serde_json::to_vec(&result).unwrap().len(), 32768);
}

#[tokio::test]
async fn omitted_name_hints_are_bounded_complete_utf8_identities_from_the_current_scope() {
    let definitions: Vec<_> = (0..20)
        .map(|position| {
            builtin(
                format!("lookup_{}_{position:02}", "界".repeat(60)),
                "Look up a record.",
                json!({"type":"object","description":"x".repeat(2000)}),
            )
        })
        .collect();
    let mut context = context(&definitions);
    let allowed: HashSet<_> = definitions
        .iter()
        .skip(1)
        .map(|definition| definition.name().to_owned())
        .collect();
    context.visible_tool_names = Some(Arc::new(allowed.clone()));
    let tool = ToolSearchTool {
        reveal: Reveal::Redefine,
        limits: Some(CatalogLimits {
            active_tools: 32,
            active_bytes: 1024,
            result_bytes: 1024,
        }),
    };
    let ToolExecutionResult::Success(result) = tool
        .execute_with_context(json!({"query":"lookup"}), &context)
        .await
    else {
        panic!("bounded name hints should fit")
    };
    assert!(serde_json::to_vec(&result).unwrap().len() <= 1024);
    assert_eq!(result["omitted_for_byte_budget"], MAX_SEARCH_RESULTS);
    let hints = result["omitted_tools"].as_array().unwrap();
    assert!(!hints.is_empty());
    assert!(hints.len() <= MAX_SEARCH_RESULTS);
    for name in hints {
        assert!(allowed.contains(name.as_str().unwrap()));
    }
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
async fn multilingual_search_discloses_authorized_canonical_schema() {
    for (description, query) in [
        ("搜索仓库问题", "仓库"),
        ("文書を検索", "検索"),
        ("Café records", "cafe\u{301}"),
    ] {
        let definitions = vec![
            builtin(
                "allowed_record",
                description,
                json!({"type":"object","properties":{"id":{"type":"integer"}}}),
            ),
            builtin("forbidden_record", description, json!({"type":"object"})),
        ];
        let mut context = context(&definitions);
        context.visible_tool_names = Some(Arc::new(HashSet::from(["allowed_record".to_owned()])));
        let ToolExecutionResult::Success(result) = bounded_tool(32768)
            .execute_with_context(json!({"query":query}), &context)
            .await
        else {
            panic!("multilingual search failed");
        };
        assert_eq!(result["loaded"], json!(["allowed_record"]), "{query}");
        assert_eq!(result["tools"].as_array().unwrap().len(), 1);
        let projected = project_search_result(query, result, definitions);
        let tool = projected
            .iter()
            .find(|tool| tool.name() == "allowed_record")
            .unwrap();
        assert_eq!(tool.parameters()["properties"]["id"]["type"], "integer");
    }
}

use super::*;
use crate::capabilities::platform::PlatformCapability;
use crate::platform_store::tests::MockPlatformStore;
use everruns_contracts::typed_id::SessionId;
use everruns_core::capabilities::{Capability, SystemPromptContext};
use serde_json::json;
use std::{collections::BTreeSet, sync::Arc};

#[test]
fn native_surface_registers_the_entire_public_catalog_with_unique_names() {
    let tools = PlatformCapability.tools_with_config(&json!({"surface":"native"}));
    let names: BTreeSet<_> = tools.iter().map(|tool| tool.name()).collect();
    assert!(tools.len() > 300);
    assert_eq!(tools.len(), operations().len());
    assert_eq!(names.len(), tools.len());
    assert!(names.contains("platform_list_agents"));
    assert!(names.contains("platform_create_agent_credential_binding"));
    assert!(!names.contains("execute"));
    assert!(!names.contains("discover"));
    for tool in tools {
        assert!(tool.name().len() <= 64, "{}", tool.name());
        assert_eq!(tool.deferrable_policy(), Default::default());
        // Compile every authentic composed schema, including rebased local refs.
        jsonschema::validator_for(&tool.parameters_schema())
            .unwrap_or_else(|error| panic!("{}: {error}", tool.name()));
    }
}

#[test]
fn nested_arguments_and_invocation_metadata_preserve_their_types() {
    let payload =
        json!({"name":"new-agent", "capabilities":[{"ref":"current_time"}], "max_iterations":3});
    let params = dispatch_params(json!({
        "params":payload, "reason":"Requested a new agent", "context_revision":7
    }))
    .unwrap();
    assert_eq!(params["capabilities"], payload["capabilities"]);
    assert_eq!(params["max_iterations"], 3);
    assert_eq!(params["reason"], "Requested a new agent");
    assert_eq!(params["context_revision"], 7);
}

#[test]
fn malformed_envelopes_and_reserved_payload_fields_are_rejected() {
    for args in [
        json!({}),
        json!({"params":"{}"}),
        json!({"params":[], "reason":"x"}),
        json!({"params":{},"organization_id":"other"}),
        json!({"params":{"organization_id":"other"}}),
        json!({"params":{"reason":"hidden"}}),
        json!({"params":{"context_revision":2}}),
        json!({"params":{},"reason":7}),
        json!({"params":{},"context_revision":"2"}),
        json!({"params":{},"context_revision":2.5}),
    ] {
        assert!(dispatch_params(args.clone()).is_err(), "{args}");
    }
}

#[test]
fn natural_queries_find_prefixed_operations_despite_large_write_schemas() {
    use everruns_contracts::runtime::tool_catalog::ToolSearchIndex;
    let definitions: Vec<_> = tools().iter().map(|tool| tool.to_definition()).collect();
    let index = ToolSearchIndex::new(&definitions);
    for (query, expected) in [
        ("list agents", "platform_list_agents"),
        ("update agent", "platform_update_agent"),
        ("get manager context", "platform_get_manager_context"),
        (
            "create agent credential binding",
            "platform_create_agent_credential_binding",
        ),
    ] {
        let hits = index.search(query, 8, |_| true);
        assert!(
            hits.iter().any(|hit| hit.name == expected),
            "{query}: {hits:?}"
        );
    }
    assert!(
        index
            .search("platform_update_agent", 8, |name| name
                == "platform_list_agents")
            .iter()
            .all(|hit| hit.name == "platform_list_agents")
    );
}

#[tokio::test]
async fn registry_rejects_invalid_canonical_arguments_before_dispatch() {
    use everruns_contracts::runtime::tool_execution::ToolExecutor;
    use everruns_contracts::tool_types::ToolCall;
    use everruns_core::tools::ToolRegistry;
    let tool = tools()
        .into_iter()
        .find(|tool| tool.name() == "platform_list_agents")
        .unwrap();
    let definition = tool.to_definition();
    let mut registry = ToolRegistry::new();
    registry.register_boxed(tool);
    let result = registry
        .execute_with_context(
            &ToolCall {
                id: "bad_args".into(),
                name: "platform_list_agents".into(),
                arguments: json!({"params":{"limit":"five"}}),
            },
            &definition,
            &ToolContext::new(SessionId::new()),
        )
        .await
        .unwrap();
    assert!(result.error.unwrap().contains("invalid_tool_arguments"));
}

#[tokio::test]
async fn native_function_dispatches_one_canonical_operation_without_shell_interpretation() {
    let tool = tools()
        .into_iter()
        .find(|tool| tool.name() == "platform_update_agent")
        .unwrap();
    let mut context = ToolContext::new(SessionId::new());
    context
        .extensions
        .insert(Arc::new(crate::PlatformStoreExt(Arc::new(
            MockPlatformStore::new(),
        ))));
    let hostile = "a'; $(id) `x`\n--organization-id other";
    let result = tool
        .execute_with_context(
            json!({
                "params":{"id":"agent_test", "description":hostile},
                "reason":"User requested a description", "context_revision":3
            }),
            &context,
        )
        .await;
    let ToolExecutionResult::Success(output) = result else {
        panic!("{result:?}")
    };
    let output: Value = serde_json::from_str(output.as_str().unwrap()).unwrap();
    assert_eq!(output["command"], "update_agent");
    assert_eq!(output["params"]["description"], hostile);
    assert_eq!(output["params"]["reason"], "User requested a description");
    assert_eq!(output["params"]["context_revision"], 3);
}

#[tokio::test]
async fn native_prompt_describes_the_selected_interface() {
    let prompt = PlatformCapability
        .system_prompt_contribution_with_config(
            &SystemPromptContext::without_file_store(SessionId::new()),
            &json!({"surface":"native"}),
        )
        .await
        .unwrap();
    assert!(prompt.contains("platform_list_agents"));
    assert!(prompt.contains("tool_search"));
    assert!(!prompt.contains("Filesystem redirection is disabled"));
    assert!(!prompt.contains("Use `query`"));
}

#[tokio::test]
async fn native_catalog_uses_bounded_disclosure_and_preserves_the_baseline_prompt() {
    use everruns_contracts::CapabilityRef;
    use everruns_core::RuntimeAgentBuilder;
    use everruns_core::builtins::ToolSearchCapability;
    use everruns_core::capabilities::CapabilityRegistry;
    use everruns_core::tools::ToolRegistry;

    let configs = [
        CapabilityRef::with_config("platform", json!({"surface":"native"})),
        CapabilityRef::with_config("tool_search", json!({"bounded":true,"threshold":1})),
    ];
    let mut capabilities = CapabilityRegistry::new();
    capabilities.register(PlatformCapability);
    capabilities.register(ToolSearchCapability::new());
    let mut prompt_context = SystemPromptContext::without_file_store(SessionId::new());
    let initial = RuntimeAgentBuilder::new()
        .with_capability_configs(&configs, &capabilities, &prompt_context)
        .await
        .build();
    assert_eq!(initial.tools.len(), 1);
    assert_eq!(initial.tools[0].name(), "tool_search");
    assert_eq!(initial.catalog_tools.len(), operations().len());

    let mut registry = ToolRegistry::new();
    for tool in tools() {
        registry.register_boxed(tool);
    }
    for tool in ToolSearchCapability::new().tools_with_config(configs[1].config_value()) {
        registry.register_boxed(tool);
    }
    registry.set_capability_contexts(&initial.execution_tools());
    let indexed = registry.search_index();
    assert!(
        indexed
            .search_sources("authorized", 16, |_| true)
            .iter()
            .any(|source| source.id == "platform")
    );
    let exact = indexed.search_operations("update agent", 8, |_| true);
    assert_eq!(exact.len(), 1);
    assert_eq!(exact[0].name, "platform_update_agent");
    assert_eq!(
        registry
            .tool_definition("platform_update_agent")
            .unwrap()
            .hints()
            .capability_description
            .as_deref(),
        Some(PlatformCapability.description())
    );
    let mut context = ToolContext::new(prompt_context.session_id);
    context.visible_tool_names = Some(Arc::new(
        initial
            .execution_tools()
            .iter()
            .map(|tool| tool.name().to_owned())
            .collect(),
    ));
    context.tool_registry = Some(Arc::new(registry));
    let result = context
        .tool_registry
        .as_ref()
        .unwrap()
        .get("tool_search")
        .unwrap()
        .execute_with_context(json!({"query":"platform_update_agent"}), &context)
        .await;
    let ToolExecutionResult::Success(result) = result else {
        panic!("{result:?}")
    };
    assert_eq!(result["loaded"], json!(["platform_update_agent"]));
    assert!(serde_json::to_vec(&result).unwrap().len() <= 32768);

    // Reconstruct after restart from the disclosure identities in the search
    // result, using the same production collection/projection as the host.
    prompt_context.tool_disclosure.deferring = true;
    prompt_context.tool_disclosure.reveal_order = vec!["platform_update_agent".into()];
    prompt_context
        .tool_disclosure
        .revealed
        .insert("platform_update_agent".into());
    let resumed = RuntimeAgentBuilder::new()
        .with_capability_configs(&configs, &capabilities, &prompt_context)
        .await
        .build();
    assert_eq!(initial.system_prompt, resumed.system_prompt);
    assert_eq!(
        serde_json::to_value(&initial.tools[0]).unwrap(),
        serde_json::to_value(
            resumed
                .tools
                .iter()
                .find(|tool| tool.name() == "tool_search")
                .unwrap()
        )
        .unwrap()
    );
    assert_eq!(resumed.tools.len(), 2);
    assert!(
        resumed
            .tools
            .iter()
            .any(|tool| tool.name() == "platform_update_agent")
    );

    // A historical reveal does not reintroduce a detached capability.
    let detached = RuntimeAgentBuilder::new()
        .with_capability_configs(&configs[1..], &capabilities, &prompt_context)
        .await
        .build();
    assert!(
        detached
            .execution_tools()
            .iter()
            .all(|tool| !tool.name().starts_with("platform_"))
    );
}

//! Late plugin operations stay searchable without rewriting the native tools
//! until the model selects one. The recording driver observes real requests.

use super::*;
use everruns_contracts::CapabilityRef;
use everruns_core::builtins::ToolSearchCapability;

#[tokio::test]
async fn bounded_search_keeps_late_plugin_schemas_out_of_the_stable_baseline() {
    let recorder = Arc::new(Recorder::new([
        Reply::Text("baseline"),
        Reply::Text("continue without the plugin"),
        Reply::Call("tool_search", json!({"query":"dynamic_plugin_action"})),
        Reply::Call("dynamic_plugin_action", json!({"label":"selected"})),
        Reply::Text("selected plugin completed"),
    ]));
    let mut capabilities = CapabilityRegistry::new();
    capabilities.register(ToolSearchCapability::new());
    let platform = HostComposition::builder()
        .capability_registry(capabilities)
        .build();
    let harness_id = everruns_contracts::typed_id::HarnessId::from_seed(3403);
    let agent_id = everruns_contracts::typed_id::AgentId::from_seed(3403);
    let session_id = everruns_contracts::typed_id::SessionId::from_seed(3403);
    let provider = everruns_contracts::runtime_provider::Provider::new(
        "recording",
        RecordingDriver(recorder.clone()),
    );
    let runtime = everruns::batteries::runtime_builder()
        .host_composition(platform)
        .provider_with_default_model(provider, "recording-model")
        .harness(
            HarnessBuilder::new("sparse", "Stable harness.")
                .id(harness_id)
                .build(),
        )
        .agent(
            AgentBuilder::new("sparse-agent", "Stable agent.")
                .id(agent_id)
                .max_iterations(5)
                .build(),
        )
        .session(
            SessionBuilder::new(harness_id)
                .id(session_id)
                .agent(agent_id)
                .capability(CapabilityRef::with_config(
                    "tool_search",
                    json!({"bounded":true,"threshold":1}),
                ))
                .build(),
        )
        .build()
        .await
        .expect("runtime builds");
    runtime
        .run_text_turn(session_id, "Establish baseline")
        .await
        .unwrap();

    let plugin = Arc::new(PluginState::default());
    runtime
        .register_capability(Arc::new(DynamicPlugin(plugin.clone())))
        .unwrap();
    runtime
        .activate_capability(session_id, "dynamic_plugin")
        .await
        .unwrap();
    let context = runtime.load_context(session_id).await.unwrap();
    assert!(
        context
            .runtime_agent
            .catalog_tools
            .iter()
            .any(|tool| tool.name() == "dynamic_plugin_action")
    );
    assert!(
        !context
            .runtime_agent
            .tools
            .iter()
            .any(|tool| tool.name() == "dynamic_plugin_action")
    );
    runtime
        .run_text_turn(session_id, "Continue without searching")
        .await
        .unwrap();
    runtime
        .run_text_turn(session_id, "Find and use the plugin")
        .await
        .unwrap();

    let requests = recorder.requests.lock().unwrap();
    assert_eq!(requests.len(), 5);
    let first = &requests[0];
    assert_eq!(first.current_tools, requests[1].current_tools);
    assert_eq!(first.current_tools, requests[2].current_tools);
    assert!(
        !requests[2]
            .current_tools
            .to_string()
            .contains("dynamic_plugin_action")
    );
    assert!(
        requests[3]
            .current_tools
            .to_string()
            .contains("dynamic_plugin_action")
    );
    for request in &requests[1..] {
        assert_eq!(request.baseline_tools, first.baseline_tools);
        assert_eq!(
            request.baseline_prompt.as_deref(),
            Some(first.system_prompt.join("\n\n").as_str())
        );
    }
    assert!(requests[3].changes.iter().any(|change| {
        change
            .added
            .iter()
            .any(|tool| tool.name() == "dynamic_plugin_action")
    }));
    assert_eq!(*plugin.executed_versions.lock().unwrap(), vec![0]);
}

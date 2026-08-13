use everruns_core::capabilities::{
    AgentCapabilityConfig, Capability, CapabilityId, CapabilityRegistry, SystemPromptContext,
    collect_capabilities_with_configs,
};
use everruns_core::tool_narration::{ToolNarrationContext, ToolNarrationPhase};
use everruns_core::tool_types::ToolCall;
use everruns_core::{AgentId, HarnessId, SessionId};
use everruns_platform::capabilities::{
    AgentHandoffCapability, SessionScheduleCapability, SubagentCapability,
    register_hosted_capabilities,
};

const HOSTED_IDS: &[&str] = &[
    "research",
    "model_scout",
    "openrouter_workspace",
    "memory",
    "background_execution",
    "session_schedule",
    "subagents",
    "session_tasks",
    "user_hooks",
    "data_knowledge",
    "knowledge_base",
    "knowledge_index",
    "citation_retrieval",
    "citation_verification",
    "platform",
    "platform_management",
];

#[test]
fn framework_registry_does_not_advertise_hosted_capabilities() {
    let registry = CapabilityRegistry::new();
    for id in HOSTED_IDS {
        assert!(!registry.has(id), "Framework registry advertised {id}");
    }
}

#[test]
fn product_registration_adds_the_hosted_catalog() {
    let grade = everruns_core::DeploymentGrade::Dev;
    let mut registry = CapabilityRegistry::new();
    register_hosted_capabilities(&mut registry, grade);
    for id in HOSTED_IDS {
        assert!(registry.has(id), "product registry omitted {id}");
    }
}

#[test]
fn hosted_catalog_keeps_dependency_tool_and_narration_invariants() {
    let registry = everruns_platform::capabilities::hosted_capability_registry_for_grade(
        everruns_core::DeploymentGrade::Prod,
    );
    let generic_narration_allowlist = [
        "data_knowledge",
        "platform",
        "platform_management",
        "model_scout",
        "openrouter_workspace",
    ];

    for capability in registry.list() {
        for dependency in capability.dependencies() {
            assert!(
                registry.has(dependency),
                "{} depends on unregistered {dependency}",
                capability.id()
            );
        }

        let mut tool_names = std::collections::HashSet::new();
        for tool in capability.tools() {
            assert!(
                tool_names.insert(tool.name().to_string()),
                "{} exposes duplicate tool {}",
                capability.id(),
                tool.name()
            );
            if generic_narration_allowlist.contains(&capability.id())
                || tool.to_definition().hints().narration_noun.is_some()
            {
                continue;
            }
            let call = ToolCall {
                id: "call_boundary_audit".to_string(),
                name: tool.name().to_string(),
                arguments: serde_json::json!({}),
            };
            assert!(
                capability
                    .narrate(
                        Some(&tool.to_definition()),
                        &call,
                        ToolNarrationPhase::Started,
                        None,
                        ToolNarrationContext::default(),
                    )
                    .is_some(),
                "{}::{} lacks backend-authored narration",
                capability.id(),
                tool.name()
            );
        }
    }
}

#[test]
fn hosted_schedule_feature_contract_is_preserved() {
    assert_eq!(SessionScheduleCapability.features(), vec!["schedules"]);
    assert_eq!(
        everruns_core::capability_dto::CapabilityInfo::from_core(&SessionScheduleCapability)
            .features,
        vec!["schedules"]
    );
}

#[tokio::test]
async fn hosted_delegation_providers_share_one_spawn_agent_tool() {
    let mut registry = CapabilityRegistry::new();
    registry.register(SubagentCapability);
    registry.register(AgentHandoffCapability);

    let configs = vec![
        AgentCapabilityConfig::with_config(CapabilityId::new("subagents"), serde_json::json!({})),
        AgentCapabilityConfig::with_config(
            CapabilityId::new("agent_handoff"),
            serde_json::json!({
                "targets": [{
                    "id": "operator",
                    "name": "Operator",
                    "agent_id": AgentId::new(),
                    "harness_id": HarnessId::new()
                }]
            }),
        ),
    ];
    let collected = collect_capabilities_with_configs(
        &configs,
        &registry,
        &SystemPromptContext::without_file_store(SessionId::new()),
    )
    .await;

    let definitions: Vec<_> = collected
        .tool_definitions
        .iter()
        .filter(|tool| tool.name() == "spawn_agent")
        .collect();
    assert_eq!(definitions.len(), 1);
    assert_eq!(
        definitions[0].parameters()["properties"]["target"]["properties"]["type"]["enum"],
        serde_json::json!(["subagent", "agent"]),
    );
}

use everruns_capabilities::capabilities::{
    AgentHandoffCapability, SubagentCapability, register_hosted_capabilities,
};
#[cfg(feature = "environment-capabilities")]
use everruns_contracts::tool_types::ToolCall;
use everruns_contracts::typed_id::{AgentId, HarnessId, SessionId};
use everruns_contracts::{CapabilityId, CapabilityRef as AgentCapabilityConfig};
use everruns_core::capabilities::{
    CapabilityRegistry, SystemPromptContext, collect_capabilities_with_configs,
};
#[cfg(feature = "environment-capabilities")]
use everruns_core::tool_narration::{ToolNarrationContext, ToolNarrationPhase};

const HOSTED_IDS: &[&str] = &[
    "research",
    "memory",
    "background_execution",
    "session_schedule",
    "subagents",
    "session_tasks",
    "user_hooks",
    "data_knowledge",
    "knowledge",
    "citation_retrieval",
    "citation_web",
    "citation_verification",
    "openai_agents_api_runtime",
    "platform",
    "platform_management",
];

// Registered only when the `environment-capabilities` feature links the
// OpenRouter integration crate (crates/platform/Cargo.toml). Off by default
// so lightweight hosts (and a bare `cargo test -p everruns-capabilities`) do not
// pull in that dependency; checked separately so the rest of this file's
// boundary checks still run without the feature.
const ENVIRONMENT_GATED_HOSTED_IDS: &[&str] = &["model_scout", "openrouter_workspace"];

#[test]
fn framework_registry_does_not_advertise_hosted_capabilities() {
    let registry = CapabilityRegistry::new();
    for id in HOSTED_IDS.iter().chain(ENVIRONMENT_GATED_HOSTED_IDS) {
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
    assert!(!registry.has("knowledge_base"));
    assert!(!registry.has("knowledge_index"));
    #[cfg(feature = "environment-capabilities")]
    for id in ENVIRONMENT_GATED_HOSTED_IDS {
        assert!(registry.has(id), "product registry omitted {id}");
    }
    #[cfg(not(feature = "environment-capabilities"))]
    for id in ENVIRONMENT_GATED_HOSTED_IDS {
        assert!(
            !registry.has(id),
            "{id} should require the environment-capabilities feature"
        );
    }
}

// Hosted dependencies span environment capabilities (session_file_system)
// and portable builtins (skills). Server and worker enable both; validate
// their composed graph only when both catalogs are registered.
#[cfg(all(feature = "environment-capabilities", feature = "portable-builtins"))]
#[test]
fn hosted_catalog_keeps_dependency_tool_and_narration_invariants() {
    let registry = everruns_capabilities::capabilities::hosted_capability_registry_for_grade(
        everruns_core::DeploymentGrade::Prod,
    );
    let generic_narration_allowlist = ["data_knowledge", "platform"];

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

/// A retired capability stays registered so existing agent, harness, and session
/// references keep resolving, but must contribute nothing.
#[test]
fn retired_capabilities_stay_registered_and_inert() {
    let registry = everruns_capabilities::capabilities::hosted_capability_registry_for_grade(
        everruns_core::DeploymentGrade::Prod,
    );
    let capability = registry
        .get("platform_management")
        .expect("retired capability must stay registered");
    assert_eq!(
        capability.status(),
        everruns_core::capabilities::CapabilityStatus::Retired
    );
    assert!(capability.tools().is_empty());
    assert!(capability.tool_definitions().is_empty());
    assert!(capability.system_prompt_addition().is_none());
    assert!(capability.mounts().is_empty());
    assert!(capability.dependencies().is_empty());
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

#[path = "capability_boundary/slack_narration.rs"]
mod slack_narration;

#[cfg(feature = "container-sandbox")]
#[path = "capability_boundary/container_narration.rs"]
mod container_narration;

use super::*;
use everruns_contracts::runtime::capability_types::AgentCapabilityConfig;

fn layers(agent: bool) -> PlatformLayers {
    let mut harness = HarnessDefinition::new("generic", "Base rules.");
    harness.capabilities = vec![AgentCapabilityConfig::new("current_time")];
    PlatformLayers {
        harness,
        agent: agent.then(|| {
            let mut agent = AgentDefinition::new(AgentId::new(), "support", "Help customers.");
            agent.capabilities = vec![AgentCapabilityConfig::new("web_fetch")];
            agent
        }),
        sandbox_policy: None,
        model: Some(ModelSpec::on("OpenRouter", "muse-1.3-contributor")),
    }
}

/// The harness chain merges under the agent, the model is a name, and no
/// stored id survives.
#[test]
fn an_agent_compiles_with_its_harness_and_without_ids() {
    let layers = layers(true);
    let declaration = compile(&layers).unwrap();
    let package = declaration.package().unwrap();
    let manifest = package.manifest();
    assert_eq!(manifest.name, "support");
    assert_eq!(manifest.instructions, "Base rules.\n\nHelp customers.");
    let capabilities: Vec<_> = manifest.capabilities.iter().map(|c| c.id()).collect();
    assert_eq!(capabilities, ["current_time", "web_fetch"]);
    let model = manifest.model.as_ref().unwrap();
    assert_eq!(model.provider.as_str(), "openrouter");
    assert_eq!(model.model, "muse-1.3-contributor");
    let text = serde_json::to_string(&declaration).unwrap();
    let agent_id = layers.agent.as_ref().unwrap().id.to_string();
    assert!(!text.contains(&agent_id), "{text}");
}

/// A harness-only session compiles under the harness's name.
#[test]
fn a_harness_only_session_compiles() {
    let declaration = compile(&layers(false)).unwrap();
    let package = declaration.package().unwrap();
    assert_eq!(package.manifest().name, "generic");
    assert_eq!(package.manifest().instructions, "Base rules.");
}

/// Equal layers compile to equal declarations, and any edit changes it:
/// what the publisher hashes to skip no-ops.
#[test]
fn compilation_is_deterministic() {
    let first = compile(&layers(true)).unwrap();
    assert_eq!(first, compile(&layers(true)).unwrap());
    let mut edited = layers(true);
    edited.harness.system_prompt = Some("Other rules.".into());
    assert_ne!(first, compile(&edited).unwrap());
}

/// Free-text platform names become package names; the original is shown.
#[test]
fn platform_names_become_package_names() {
    assert_eq!(package_name("Support Bot (EU)"), "support-bot-eu");
    assert_eq!(package_name("  --  "), "agent");
    assert_eq!(package_name("Ünïcode Agent 2"), "n-code-agent-2");
    assert_eq!(package_name(&"a".repeat(300)).len(), 255);
    let mut named = layers(true);
    named.agent.as_mut().unwrap().name = "Support Bot".into();
    let manifest = compile(&named)
        .unwrap()
        .package()
        .unwrap()
        .manifest()
        .clone();
    assert_eq!(manifest.name, "support-bot");
    assert_eq!(manifest.display_name.as_deref(), Some("Support Bot"));
}

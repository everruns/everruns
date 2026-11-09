use super::*;
use crate::hosting::MemoryHosting;
use crate::{Engine, Model, SessionId};
use serde_json::json;

/// A revision of `helper` whose agent replies with its instructions.
fn revision(revision: &str, reply: &str) -> AgentDefinition {
    AgentDefinition {
        build: AgentBuild::new("helper", revision),
        package: json!({ "name": "helper", "instructions": reply }),
    }
}

/// Offline hydration: the package's instructions become the simulated reply.
fn replying(definition: &AgentDefinition) -> Result<Hydrated> {
    let package = definition.agent_package()?;
    let agent = package
        .builder()
        .map_err(|error| AgentLoopError::config(error.to_string()))?
        .model(Model::simulated(package.manifest().instructions.clone()))
        .build()
        .map_err(|error| AgentLoopError::config(error.to_string()))?;
    Ok(Hydrated {
        agent,
        harness: None,
    })
}

/// The one rule every authority applies: a session moves between turns,
/// and only its revision moves.
#[test]
fn a_session_moves_only_between_turns() {
    let bound = |revision: &str| crate::hosting::SessionBinding {
        framework: crate::hosting::SESSION_RUNTIME_VERSION,
        behavior: AgentBuild::new("helper", revision),
        harness: None,
        environment: Some("agent-default".into()),
        metadata: json!({ "kept": true }),
    };
    let mut binding = bound("r1");
    assert!(!advance(&mut binding, true, Some("r2")));
    assert!(!advance(&mut binding, false, None));
    assert!(!advance(&mut binding, false, Some("r1")));
    assert_eq!(binding, bound("r1"));
    assert!(advance(&mut binding, false, Some("r2")));
    assert_eq!(binding, bound("r2"));
}

/// A definition this Engine cannot build is refused before any session can
/// bind to it, and an unknown revision leaves the build missing.
#[tokio::test]
async fn an_unbuildable_definition_is_refused_at_publication() {
    let engine = Engine::builder().hosting(MemoryHosting::default()).build();
    let needs_tool = AgentDefinition {
        build: AgentBuild::new("tooling", "r1"),
        package: json!({
            "name": "tooling",
            "instructions": "Use the tool.",
            "tools": [{ "name": "lookup", "description": "Look up.", "parameters": { "type": "object" } }]
        }),
    };
    assert!(engine.publish_definition(&needs_tool).await.is_err());
    assert!(
        engine
            .create_registered(SessionId::new(), &AgentBuild::new("tooling", "r1"))
            .await
            .is_err()
    );
    // Keys stay inside the build-key alphabet.
    let mut bad = revision("r1", "one");
    bad.build.agent = "has space".into();
    assert!(bad.validate().is_err());
}

/// The default hydrator is the agent-package mapping.
#[test]
fn the_package_mapping_hydrates_a_definition() {
    let definition = AgentDefinition {
        build: AgentBuild::new("support", "r1"),
        package: json!({
            "name": "support",
            "instructions": "Help.",
            "model": { "provider": "openai", "model": "gpt-6.1-sol" },
            "harness": "conversation"
        }),
    };
    // Without host providers a provider-keyed model has nothing to serve it.
    assert!(PackageHydrator::default().hydrate(&definition).is_err());
    let services = crate::hosting::services::HostServices::new().providers(
        crate::hosting::services::HostProviders::new(
            everruns_core::host::InMemoryProviderStore::default(),
            crate::hosting::services::DriverRegistry::new(),
        ),
    );
    let hydrated = PackageHydrator::with_services(services)
        .hydrate(&definition)
        .unwrap();
    assert!(hydrated.harness.is_some());
    let package = crate::AgentPackage::parse(
        "---\nname: support\n---\nHelp.\n",
        crate::PackageFormat::Markdown,
    )
    .unwrap();
    let round_trip = AgentDefinition::from_package("support", "r2", &package).unwrap();
    assert_eq!(
        round_trip.agent_package().unwrap().manifest().name,
        "support"
    );
}

/// The hydrated cache is bounded; a dropped revision hydrates again from
/// the authority.
#[tokio::test]
async fn hydrated_revisions_are_bounded() {
    let cache = HydratedCache::new(4);
    for n in 0..10 {
        cache.insert(AgentBuild::new("a", n.to_string()), n);
    }
    assert!(cache.len() <= 4);
    assert_eq!(cache.get(&AgentBuild::new("a", "9")), Some(9));
    // First insert wins: one behavior per revision.
    assert_eq!(cache.insert(AgentBuild::new("a", "9"), 99), 9);

    let profile = MemoryHosting::default();
    let engine = Engine::builder()
        .hosting(profile.clone())
        .hydrator(replying)
        .hydrated_capacity(1)
        .build();
    let a = SessionId::new();
    let b = SessionId::new();
    let session_a = engine
        .create_from_definition(a, &revision("r1", "one"))
        .await
        .unwrap();
    let other = AgentDefinition {
        build: AgentBuild::new("other", "r1"),
        package: json!({ "name": "other", "instructions": "other" }),
    };
    let session_b = engine.create_from_definition(b, &other).await.unwrap();
    assert_eq!(
        session_b.send_and_wait("Hi").await.unwrap().response,
        "other"
    );
    assert_eq!(session_a.send_and_wait("Hi").await.unwrap().response, "one");
}

#[cfg(feature = "filesystem")]
fn with_file(revision: &str) -> AgentDefinition {
    AgentDefinition {
        build: AgentBuild::new("helper", revision),
        package: json!({
            "name": "helper",
            "instructions": "filed",
            "files": [{ "path": "notes.md", "content": "Remember." }]
        }),
    }
}

/// An edit that gives an agent its first file, and one that takes its files
/// away, apply to the agent's sessions like any other edit, and a session
/// keeps one workspace through every revision and cache eviction.
#[cfg(feature = "filesystem")]
#[tokio::test]
async fn a_session_keeps_its_workspace_when_its_agent_is_edited() {
    let engine = Engine::builder()
        .hosting(MemoryHosting::default())
        .hydrator(replying)
        .hydrated_capacity(1)
        .build();
    let id = SessionId::new();
    let session = engine
        .create_from_definition(id, &revision("r1", "one"))
        .await
        .unwrap();
    assert_eq!(session.send_and_wait("Hi").await.unwrap().response, "one");
    let workspace =
        |environment: crate::Environment| environment.workspace_head().binding().clone();
    let created = workspace(engine.hosted_environment(id).await.unwrap());

    engine.publish_definition(&with_file("r2")).await.unwrap();
    assert_eq!(session.send_and_wait("Hi").await.unwrap().response, "filed");
    assert_eq!(
        workspace(engine.hosted_environment(id).await.unwrap()),
        created
    );

    engine
        .publish_definition(&revision("r3", "three"))
        .await
        .unwrap();
    // Another agent takes the only cache slot; the session's revision is
    // hydrated again and still opens the same workspace.
    let other = AgentDefinition {
        build: AgentBuild::new("other", "r1"),
        package: json!({ "name": "other", "instructions": "other" }),
    };
    engine.publish_definition(&other).await.unwrap();
    assert_eq!(session.send_and_wait("Hi").await.unwrap().response, "three");
    assert_eq!(
        workspace(engine.hosted_environment(id).await.unwrap()),
        created
    );
    assert!(!engine.session_status(id).await.unwrap().suspended);
}

/// Sessions of more agents than the cache holds run side by side: a turn
/// keeps the revision it resolved even when another agent's activation
/// drops it from the cache.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_agents_over_a_small_cache_never_suspend() {
    let engine = Engine::builder()
        .hosting(MemoryHosting::default().without_prompts())
        .hydrator(replying)
        .hydrated_capacity(1)
        .build();
    let mut tasks = Vec::new();
    for agent in 0..4 {
        let engine = engine.clone();
        tasks.push(tokio::spawn(async move {
            let name = format!("agent-{agent}");
            let definition = AgentDefinition {
                build: AgentBuild::new(name.clone(), "r1"),
                package: json!({ "name": name, "instructions": name }),
            };
            for _ in 0..25 {
                let id = SessionId::new();
                let session = engine
                    .create_from_definition(id, &definition)
                    .await
                    .unwrap();
                assert_eq!(session.send_and_wait("Hi").await.unwrap().response, name);
                assert!(!engine.session_status(id).await.unwrap().suspended);
            }
        }));
    }
    for task in tasks {
        task.await.unwrap();
    }
}

/// A team member's build is derived from its lead's and follows the lead's
/// definitions, whatever the lead's key looks like.
#[test]
fn a_member_build_resolves_through_its_lead() {
    let lead = AgentBuild::new("support:agent:tier-1", "r1");
    let member = everruns_contracts::typed_id::AgentId::new();
    let build = lead.member(member);
    assert_eq!(build.lead(), Some((lead.clone(), member)));
    assert_eq!(build.defined_agent(), lead.agent);
    assert_eq!(lead.lead(), None);
    assert_eq!(lead.defined_agent(), lead.agent);

    let mut binding = crate::hosting::SessionBinding {
        framework: crate::hosting::SESSION_RUNTIME_VERSION,
        behavior: build,
        harness: None,
        environment: None,
        metadata: json!(null),
    };
    let mut store = DefinitionStore::default();
    for revision in ["r1", "r2"] {
        store
            .publish(&AgentDefinition {
                build: AgentBuild::new(lead.agent.clone(), revision),
                package: json!({ "name": "support", "instructions": revision }),
            })
            .unwrap();
    }
    let current = store.current(binding.behavior.defined_agent());
    assert!(advance(&mut binding, false, current));
    assert_eq!(
        binding.behavior,
        AgentBuild::new(lead.agent, "r2").member(member)
    );
}

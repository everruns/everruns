use super::*;
use crate::hosting::{MemoryHosting, SessionHosting};
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

fn engine(profile: impl SessionHosting) -> Engine {
    Engine::builder()
        .hosting(profile)
        .hydrator(replying)
        .build()
}

/// The whole contract, against any authority that keeps definitions: a
/// session runs its definition, an Engine that never saw it hydrates it from
/// the authority, and a new revision applies from the next turn.
async fn definitions_are_data(profile: impl SessionHosting + Clone) {
    let id = SessionId::new();
    let first = engine(profile.clone());
    let session = first
        .create_from_definition(id, &revision("r1", "one"))
        .await
        .unwrap();
    assert_eq!(session.send_and_wait("Hi").await.unwrap().response, "one");
    drop(session);
    drop(first);

    // Another process: nothing registered, the authority holds the revision.
    let other = engine(profile.clone());
    let session = other.resume(id).await.unwrap();
    assert_eq!(
        session.send_and_wait("Again").await.unwrap().response,
        "one"
    );

    // An edit is a new revision; the session runs it from its next turn.
    other
        .publish_definition(&revision("r2", "two"))
        .await
        .unwrap();
    assert_eq!(session.send_and_wait("Now").await.unwrap().response, "two");
    assert_eq!(
        other.session_binding(id).await.unwrap().behavior,
        AgentBuild::new("helper", "r2")
    );

    // Revisions are immutable; the same content again is fine.
    other
        .publish_definition(&revision("r2", "two"))
        .await
        .unwrap();
    assert!(
        other
            .publish_definition(&revision("r2", "changed"))
            .await
            .is_err()
    );
    assert_eq!(
        profile
            .definition(&AgentBuild::new("helper", "r1"))
            .await
            .unwrap(),
        Some(revision("r1", "one"))
    );
}

#[tokio::test]
async fn memory_authority_keeps_definitions() {
    definitions_are_data(MemoryHosting::default()).await;
}

#[cfg(feature = "local")]
#[tokio::test]
async fn sqlite_authority_keeps_definitions() {
    let dir = tempfile::tempdir().unwrap();
    definitions_are_data(crate::hosting::SqliteHosting::open(dir.path().join("a.db")).unwrap())
        .await;
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

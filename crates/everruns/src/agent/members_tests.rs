use super::*;
use crate::{Agent, Model, PackageFormat};

fn agent(name: &str) -> AgentBuilder {
    Agent::builder()
        .name(name)
        .instructions("Work.")
        .model(Model::simulated("Done."))
}

fn reason(result: Result<Agent, BuildError>) -> String {
    match result {
        Err(BuildError::InvalidCapability { id, reason }) => {
            assert!(carries_agents(&id), "unexpected capability {id}");
            reason
        }
        other => panic!("expected a delegation error, got {:?}", other.map(|_| ())),
    }
}

fn capability<'a>(agent: &'a Agent, id: &str) -> Option<&'a everruns_contracts::CapabilityRef> {
    agent.capabilities.iter().find(|c| c.id() == id)
}

#[test]
fn spawned_agents_are_listed_by_name_on_spawn_agents() {
    let lead = agent("lead")
        .spawns(
            agent("reviewer")
                .description("Reviews changes.")
                .build()
                .unwrap(),
        )
        .build()
        .unwrap();
    let spawn = capability(&lead, SPAWN_AGENTS_CAPABILITY_ID).expect("spawns adds Spawn agents");
    assert_eq!(spawn.config_value()["self"], true);
    let listed = &spawn.config_value()["agents"][0];
    assert_eq!(listed["name"], "reviewer");
    assert_eq!(listed["description"], "Reviews changes.");
    assert_eq!(
        listed["agent_id"],
        serde_json::json!(member_agent_id("reviewer"))
    );
    assert!(lead.member_on_host(member_agent_id("reviewer")).is_some());
    assert!(
        lead.member_on_host(member_agent_id("someone-else"))
            .is_none()
    );
    let solo = agent("solo").build().unwrap();
    assert!(
        capability(&solo, SPAWN_AGENTS_CAPABILITY_ID).is_none(),
        "no agents, no capability"
    );
}

#[test]
fn a_spawn_agents_capability_added_by_hand_keeps_its_settings() {
    let lead = agent("lead")
        .capability(crate::SpawnAgents::new().run_self(false).max_open(2))
        .spawns(agent("reviewer").build().unwrap())
        .build()
        .unwrap();
    let spawn = capability(&lead, SPAWN_AGENTS_CAPABILITY_ID).unwrap();
    assert_eq!(spawn.config_value()["self"], false);
    assert_eq!(spawn.config_value()["max_open"], 2);
    assert_eq!(spawn.config_value()["agents"][0]["name"], "reviewer");
}

#[cfg(feature = "local")]
#[test]
fn threads_with_lists_agents_on_threads() {
    let lead = agent("lead")
        .capability(crate::threads::Threads::new().max_open(3))
        .threads_with(agent("writer").build().unwrap())
        .build()
        .unwrap();
    let threads = capability(&lead, THREADS_CAPABILITY_ID).unwrap();
    assert_eq!(threads.config_value()["max_open"], 3);
    assert_eq!(threads.config_value()["agents"][0]["name"], "writer");
    assert!(capability(&lead, SPAWN_AGENTS_CAPABILITY_ID).is_none());
}

#[test]
fn invalid_agent_sets_are_refused() {
    let member = || agent("reviewer").build().unwrap();
    assert!(reason(agent("lead").spawns(member()).spawns(member()).build()).contains("two agents"));
    assert!(reason(agent("reviewer").spawns(member()).build()).contains("itself"));
    let nested = agent("reviewer")
        .spawns(agent("helper").build().unwrap())
        .build()
        .unwrap();
    assert!(reason(agent("lead").spawns(nested).build()).contains("one level"));
    assert!(
        reason(
            agent("lead")
                .capability(crate::CapabilityRef::new(THREADS_CAPABILITY_ID))
                .spawns(member())
                .build()
        )
        .contains("never both")
    );
}

#[cfg(feature = "local")]
#[test]
fn spawns_and_threads_with_are_never_both() {
    let error = reason(
        agent("lead")
            .spawns(agent("reviewer").build().unwrap())
            .threads_with(agent("writer").build().unwrap())
            .build(),
    );
    assert!(error.contains("never both"));
}

#[test]
fn packages_bring_bundled_agents_and_need_linked_ones() {
    let package = crate::AgentPackage::parse(
        r#"{
            "schema_version": 1,
            "name": "factory",
            "instructions": "Run the factory.",
            "members": ["everruns-review"],
            "agents": [{
                "schema_version": 1,
                "name": "everruns-triage",
                "description": "Scopes problems.",
                "instructions": "Reproduce it."
            }]
        }"#,
        PackageFormat::Json,
    )
    .unwrap();
    let builder = || {
        package
            .apply_to(Agent::builder())
            .unwrap()
            .model(Model::simulated("Done."))
    };
    assert!(reason(builder().build()).contains("links agent everruns-review"));

    let lead = builder()
        .spawns(agent("everruns-review").build().unwrap())
        .build()
        .unwrap();
    let mut names: Vec<_> = lead.members().iter().map(Agent::name).collect();
    names.sort();
    assert_eq!(names, ["everruns-review", "everruns-triage"]);
    let triage = lead
        .members()
        .iter()
        .find(|m| m.name() == "everruns-triage")
        .unwrap();
    assert_eq!(triage.description(), Some("Scopes problems."));

    let exported = lead.to_package().unwrap();
    let manifest = exported.manifest();
    assert_eq!(manifest.agents.len(), 2, "every member exports as bundled");
    assert!(manifest.members.is_none());
    assert!(
        manifest
            .capabilities
            .iter()
            .filter(|c| carries_agents(c.id()))
            .all(|c| c.config_value().get("agents").is_none()),
        "derived agent ids are not portable"
    );
}

#[cfg(feature = "local")]
#[tokio::test(flavor = "multi_thread")]
#[allow(deprecated)] // The legacy local profile lane keeps its own child runner.
async fn a_member_runs_as_itself_on_a_local_profile() {
    use crate::{LlmSimConfig, SimToolCall, SimTurn};
    let directory = tempfile::tempdir().unwrap();
    let lead = Agent::builder()
        .name("lead")
        .instructions("Hand the change to reviewer.")
        .model(Model::simulated_with_config(LlmSimConfig::scripted(vec![
            SimTurn::ToolCalls(vec![SimToolCall {
                name: "spawn_agent".into(),
                arguments: serde_json::json!({
                    "name": "Review",
                    "instructions": "Review it.",
                    "agent": "reviewer",
                    "wait": true,
                }),
                id: Some("review_1".into()),
            }]),
            SimTurn::Assistant("Reviewed.".into()),
        ])))
        .spawns(
            Agent::builder()
                .name("reviewer")
                .instructions("Review.")
                .model(Model::simulated("REVIEWED locally"))
                .build()
                .unwrap(),
        )
        .local(crate::LocalConfig::new(directory.path()))
        .build()
        .unwrap();
    let session = crate::Engine::new().create(lead);
    let turn = session.send_and_wait("Review the change.").await.unwrap();
    assert!(turn.success, "{:?}", turn.error);
    let history = session.history().page().await.unwrap();
    let rendered = serde_json::to_string(
        &history
            .messages
            .iter()
            .map(|message| &message.content)
            .collect::<Vec<_>>(),
    )
    .unwrap();
    assert!(rendered.contains("REVIEWED locally"), "{rendered}");
}

#[test]
fn an_added_agent_gets_file_tools_for_the_workspace_it_shares() {
    let dir = tempfile::tempdir().unwrap();
    let lead = agent("lead")
        .workspace(dir.path())
        .spawns(agent("writer").build().unwrap())
        .build()
        .unwrap();
    let writer = lead.member_on_host(member_agent_id("writer")).unwrap();
    assert!(writer.capability_ids().contains("session_file_system"));
    assert_eq!(writer.workspace_root.as_deref(), Some(dir.path()));
}

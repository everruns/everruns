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
            assert_eq!(id, AGENTS_CAPABILITY);
            reason
        }
        other => panic!("expected an Agents error, got {:?}", other.map(|_| ())),
    }
}

#[test]
fn members_become_hand_off_targets_by_name() {
    let lead = agent("lead")
        .member(
            agent("reviewer")
                .description("Reviews changes.")
                .build()
                .unwrap(),
        )
        .build()
        .unwrap();
    let agents = lead
        .capabilities
        .iter()
        .find(|c| c.id() == AGENTS_CAPABILITY)
        .expect("members add the Agents capability");
    let target = &agents.config_value()["targets"][0];
    assert_eq!(target["id"], "reviewer");
    assert_eq!(target["description"], "Reviews changes.");
    assert_eq!(
        target["agent_id"],
        serde_json::json!(member_agent_id("reviewer"))
    );
    assert!(lead.member_on_host(member_agent_id("reviewer")).is_some());
    assert!(
        lead.member_on_host(member_agent_id("someone-else"))
            .is_none()
    );
    assert!(
        agent("solo")
            .build()
            .unwrap()
            .capabilities
            .iter()
            .all(|c| c.id() != AGENTS_CAPABILITY),
        "no members, no capability"
    );
}

#[test]
fn invalid_member_sets_are_refused() {
    let member = || agent("reviewer").build().unwrap();
    assert!(reason(agent("lead").member(member()).member(member()).build()).contains("two agents"));
    assert!(reason(agent("reviewer").member(member()).build()).contains("itself"));
    let nested = agent("reviewer")
        .member(agent("helper").build().unwrap())
        .build()
        .unwrap();
    assert!(reason(agent("lead").member(nested).build()).contains("one level"));
    assert!(
        reason(
            agent("lead")
                .capability(crate::CapabilityRef::new(AGENTS_CAPABILITY))
                .member(member())
                .build()
        )
        .contains("not both")
    );
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
        .member(agent("everruns-review").build().unwrap())
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
            .all(|c| c.id() != AGENTS_CAPABILITY),
        "member ids are not portable"
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
                    "target": {"type": "agent", "id": "reviewer"},
                    "mode": "foreground",
                }),
                id: Some("review_1".into()),
            }]),
            SimTurn::Assistant("Reviewed.".into()),
        ])))
        .member(
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

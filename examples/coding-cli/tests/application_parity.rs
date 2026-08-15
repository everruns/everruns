//! External-crate acceptance tests for promoted Framework application APIs.
//!
//! This package has exactly one Everruns dependency: the public `everruns`
//! crate. Every fixture is offline and uses temporary files.

use std::fs;

use everruns::{Agent, CompactionConfig, CompactionStrategy, InMemoryEngine, LocalConfig, Model};

#[tokio::test]
async fn compaction_policy_runs_without_host_checkpoint_types() {
    let agent = Agent::builder()
        .instructions("Keep long conversations useful.")
        .model(Model::simulated("ok"))
        .capability(
            CompactionConfig::new()
                .strategy(CompactionStrategy::ObservationMasking)
                .budget_percent(0.75),
        )
        .build()
        .expect("agent builds");
    InMemoryEngine::new()
        .create(agent)
        .run("offline turn")
        .await
        .expect("turn runs");
}

#[tokio::test]
async fn workspace_files_and_context_are_application_values() {
    let workspace = tempfile::tempdir().expect("workspace");
    let agent = Agent::builder()
        .instructions("Inspect files when asked.")
        .model(Model::simulated("ready"))
        .workspace(workspace.path())
        .file("/workspace/editable.txt", "editable")
        .readonly_file("/workspace/reference.txt", "reference")
        .build()
        .expect("agent builds");

    let session = InMemoryEngine::new().create(agent.clone());
    let before = session.inspect().await.expect("context before a turn");
    assert!(before.messages.is_empty());
    assert_eq!(before.model.model.as_str(), "llmsim-model");
    assert!(before.tools.iter().any(|tool| tool.name == "read_file"));
    assert_eq!(
        fs::read_to_string(workspace.path().join("editable.txt")).expect("seeded file"),
        "editable"
    );

    session.run("hello").await.expect("offline turn");
    assert!(
        !session
            .inspect()
            .await
            .expect("context after turn")
            .messages
            .is_empty()
    );
}

#[tokio::test]
async fn local_plugin_uses_the_framework() {
    let plugin = tempfile::tempdir().expect("plugin dir");
    fs::create_dir_all(plugin.path().join(".claude-plugin")).expect("manifest dir");
    fs::create_dir_all(plugin.path().join("agents")).expect("agents dir");
    fs::write(
        plugin.path().join(".claude-plugin/plugin.json"),
        r#"{
          "name": "offline-helper",
          "version": "0.1.0",
          "description": "Offline fixture",
          "agents": "./agents/"
        }"#,
    )
    .expect("manifest");
    fs::write(
        plugin.path().join("agents/helper.md"),
        "---\nname: helper\ndescription: Offline helper\n---\nPLUGIN_CONTEXT_SENTINEL\n",
    )
    .expect("agent contribution");

    let agent = Agent::builder()
        .instructions("Use configured integrations.")
        .model(Model::simulated("ok"))
        .plugin(plugin.path())
        .expect("plugin compiles")
        .build()
        .expect("agent builds");
    let session = InMemoryEngine::new().create(agent.clone());
    let context = session.inspect().await.expect("plugin context");
    assert!(context.instructions.contains("PLUGIN_CONTEXT_SENTINEL"));
}

#[tokio::test]
async fn local_profile_supplies_workspace_and_schedule_state() {
    let root = tempfile::tempdir().expect("local root");
    let workspace = root.path().join("workspace");
    fs::create_dir_all(&workspace).expect("workspace dir");
    let agent = Agent::builder()
        .instructions("Use local task and workspace state.")
        .model(Model::simulated("configured"))
        .local(LocalConfig::new(root.path()).workspace(&workspace))
        .capability("session_schedule")
        .build()
        .expect("agent builds");

    let session = InMemoryEngine::new().create(agent.clone());
    let context = session.inspect().await.expect("local context");
    assert!(
        context
            .tools
            .iter()
            .any(|tool| tool.name == "create_schedule")
    );
    session.run("continue").await.expect("local turn");
}

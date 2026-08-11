//! Downstream-style acceptance fixtures for promoted Framework scenarios.
//!
//! This integration crate imports only `everruns`; it uses temporary files and
//! local simulation, never credentials or the network.

use std::fs;

use everruns::{Agent, Model};

#[cfg(feature = "builtins")]
#[tokio::test]
async fn compaction_is_configured_without_checkpoint_store_types() {
    use everruns::{CompactionConfig, CompactionStrategy};

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
    agent
        .session()
        .run("offline turn")
        .await
        .expect("turn runs");
}

#[tokio::test]
async fn real_workspace_seeding_and_context_inspection_use_framework_types() {
    let workspace = tempfile::tempdir().expect("workspace");
    let agent = Agent::builder()
        .instructions("Inspect the workspace when asked.")
        .model(Model::simulated("ready"))
        .workspace(workspace.path())
        .file("/workspace/notes.txt", "facade workspace")
        .build()
        .expect("agent builds");

    let session = agent.session();
    let before = session.inspect().await.expect("context before first turn");
    assert!(before.messages.is_empty());
    assert_eq!(before.model.model.as_str(), "llmsim-model");
    assert!(before.tools.iter().any(|tool| tool.name == "read_file"));
    assert_eq!(
        fs::read_to_string(workspace.path().join("notes.txt")).expect("seeded file"),
        "facade workspace"
    );

    session.run("hello").await.expect("offline turn");
    let after = session.inspect().await.expect("context after turn");
    assert!(!after.messages.is_empty());
}

#[tokio::test]
async fn local_plugin_contributes_to_the_same_inspected_context() {
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
        .instructions("Use configured plugins.")
        .model(Model::simulated("ok"))
        .plugin(plugin.path())
        .expect("plugin compiles")
        .build()
        .expect("agent builds");
    let session = agent.session();
    let context = session.inspect().await.expect("plugin context");
    assert!(context.instructions.contains("PLUGIN_CONTEXT_SENTINEL"));
}

#[cfg(feature = "mcp-stdio")]
#[tokio::test]
async fn stdio_mcp_is_discovered_through_the_framework() {
    use everruns::McpServer;

    let server =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mcp_stdio_server.py");
    let agent = Agent::builder()
        .instructions("Use the echo MCP server.")
        .model(Model::simulated("ok"))
        .mcp_server(McpServer::stdio("echo", "python3").arg(server.to_string_lossy()))
        .build()
        .expect("agent builds");
    let session = agent.session();
    let context = session.inspect().await.expect("MCP discovery");
    assert!(
        context
            .tools
            .iter()
            .any(|tool| tool.name == "mcp_echo__echo"),
        "discovered tools: {:?}",
        context
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>()
    );
}

#[cfg(feature = "local")]
#[tokio::test]
async fn local_profile_exposes_workspace_and_schedule_tools() {
    use everruns::LocalConfig;

    let root = tempfile::tempdir().expect("local root");
    let workspace = root.path().join("workspace");
    fs::create_dir_all(&workspace).expect("workspace dir");
    let agent = Agent::builder()
        .instructions("Use the local workspace and schedule state.")
        .model(Model::simulated("configured"))
        .local(LocalConfig::new(root.path()).workspace(&workspace))
        .capability("session_schedule")
        .build()
        .expect("agent builds");

    let session = agent.session();
    let context = session.inspect().await.expect("local context");
    assert!(
        context
            .tools
            .iter()
            .any(|tool| tool.name == "create_schedule")
    );
    session.run("continue").await.expect("local turn");
}

#[test]
fn fixture_sources_import_only_the_framework() {
    let source = include_str!("application_parity.rs");
    for forbidden in [
        ["everruns", "core"].join("_"),
        ["everruns", "runtime"].join("_"),
    ] {
        assert!(
            !source.contains(&forbidden),
            "fixture must not import {forbidden}"
        );
    }
}

#[test]
fn facade_has_no_runtime_dependency_or_source_import() {
    let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let runtime_package = ["everruns", "runtime"].join("-");
    let runtime_module = ["everruns", "runtime"].join("_");
    let manifest = fs::read_to_string(crate_root.join("Cargo.toml")).expect("facade manifest");
    assert!(
        !manifest.lines().any(|line| {
            let line = line.trim_start();
            line.starts_with(&format!("{runtime_package}."))
                || line.starts_with(&format!("{runtime_package} ="))
        }),
        "everruns must depend directly on the canonical host, not runtime compatibility"
    );

    for entry in fs::read_dir(crate_root.join("src")).expect("facade source directory") {
        let path = entry.expect("facade source entry").path();
        if path.extension().is_some_and(|extension| extension == "rs") {
            let contents = fs::read_to_string(&path).expect("facade source file");
            assert!(
                !contents.contains(&runtime_module),
                "{} imports runtime compatibility instead of the canonical host",
                path.display()
            );
        }
    }
}

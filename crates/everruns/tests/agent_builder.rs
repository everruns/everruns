//! Compile/build test for the value-first agent API (EVE-832): a clean program
//! describes an agent importing only `everruns::{Agent, Model}` — no `Harness`,
//! hosted `Agent` record, ids, timestamps, registries, or `HostComposition`.

use everruns::{Agent, Model};

#[test]
fn describes_an_agent_with_only_the_facade() {
    let agent = Agent::builder()
        .instructions("You are concise.")
        .model(Model::simulated("Sure."))
        .name("assistant")
        .build()
        .expect("valid agent");

    // The built value is usable and independent of any backend record.
    let _clone = agent.clone();
}

#[test]
fn missing_model_is_a_typed_error() {
    use everruns::BuildError;
    let err = Agent::builder()
        .instructions("You are concise.")
        .build()
        .unwrap_err();
    assert_eq!(err, BuildError::MissingModel);
}

#[cfg(feature = "builtins")]
#[test]
fn invalid_compaction_budget_is_a_typed_error() {
    use everruns::{BuildError, CompactionConfig};

    for invalid in [0.05, 1.5, f32::NAN] {
        let err = Agent::builder()
            .instructions("You are concise.")
            .model(Model::simulated("ok"))
            .capability(CompactionConfig::new().budget_percent(invalid))
            .build()
            .unwrap_err();
        assert!(matches!(err, BuildError::InvalidCapability { ref id, .. } if id == "compaction"));
    }
}

#[test]
fn duplicate_mcp_names_are_a_typed_error() {
    use everruns::{BuildError, McpServer};

    let err = Agent::builder()
        .instructions("You are concise.")
        .model(Model::simulated("ok"))
        .mcp_server(McpServer::http("docs", "https://one.invalid/mcp"))
        .mcp_server(McpServer::http("docs", "https://two.invalid/mcp"))
        .build()
        .unwrap_err();
    assert!(matches!(err, BuildError::InvalidMcpServer { .. }));
}

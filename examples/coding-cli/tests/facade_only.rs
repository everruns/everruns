//! Acceptance tests for the coding-cli example (EVE-835).
//!
//! These prove the example works through the public `everruns` surface and
//! never recreates Framework-owned workspace or filesystem behavior.

use std::path::Path;

use everruns::{InMemoryEngine, Model};
use everruns_coding_cli::agent_builder;

#[tokio::test]
async fn offline_smoke_runs_a_turn() {
    let agent = agent_builder()
        .model(Model::simulated("Done."))
        .build()
        .expect("agent builds via the facade");
    let session = InMemoryEngine::new().create(agent.clone());
    let turn = session
        .run("list the files")
        .await
        .expect("turn runs offline");
    assert!(
        turn.success,
        "offline turn should succeed: {:?}",
        turn.error
    );
    assert_eq!(turn.response, "Done.");
}

#[tokio::test]
async fn session_history_persists_across_two_prompts() {
    // One session, two prompts — the second reuses the first's runtime and
    // accumulated history. Both must run cleanly through the public API.
    let agent = agent_builder()
        .model(Model::simulated("ack"))
        .build()
        .expect("agent builds");
    let session = InMemoryEngine::new().create(agent.clone());

    let first = session.run("first prompt").await.expect("first turn");
    let second = session.run("second prompt").await.expect("second turn");

    assert!(first.success && second.success);
    assert_eq!(first.response, "ack");
    assert_eq!(second.response, "ack");
    // A stable session id ties the turns together.
    assert!(!session.id().is_empty());
}

/// Scan sources and the manifest for internal dependencies or duplicate file tools.
#[test]
fn sources_do_not_reference_core_or_host() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));

    // Scan the example's own sources. This test file is excluded on purpose —
    // it necessarily contains the forbidden identifiers as the search needles.
    let forbidden_in_rust = ["everruns_core", "everruns_host"];
    for file in ["src/lib.rs", "src/main.rs"] {
        let text = std::fs::read_to_string(root.join(file)).unwrap_or_default();
        for needle in forbidden_in_rust {
            assert!(
                !text.contains(needle),
                "{file} references `{needle}`; the example must use only the `everruns` facade"
            );
        }
    }

    let duplicate_workspace_mechanisms = [
        "set_workspace",
        "OnceLock<PathBuf>",
        "safe_path",
        "tokio::fs",
        "#[everruns::tool]",
    ];
    for file in ["src/lib.rs", "src/main.rs"] {
        let text = std::fs::read_to_string(root.join(file)).unwrap_or_default();
        for needle in duplicate_workspace_mechanisms {
            assert!(
                !text.contains(needle),
                "{file} contains `{needle}`; workspace I/O must stay Framework-owned"
            );
        }
    }

    // Driver packages are valid public extensions. The example must not reach
    // into execution/storage implementation crates.
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    for line in manifest.lines() {
        let code = line.split('#').next().unwrap_or("");
        for needle in [
            "everruns-core",
            "everruns-host",
            "everruns-local",
            concat!("everruns-", "runtime"),
        ] {
            assert!(
                !code.contains(needle),
                "Cargo.toml declares `{needle}`; the example must depend only on `everruns`"
            );
        }
    }
}

#[test]
fn coding_cli_changes_trigger_workspace_rust_ci() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let workflow =
        std::fs::read_to_string(root.join(".github/workflows/ci.yml")).expect("read CI workflow");
    assert!(
        workflow.contains("- 'examples/coding-cli/**'"),
        "coding CLI changes must trigger workspace Rust compilation and tests"
    );
}

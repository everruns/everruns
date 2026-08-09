//! Acceptance tests for the coding-cli example (EVE-835).
//!
//! These prove the example works through the public `everruns` surface and,
//! crucially, that it never reaches for `everruns-core`/`everruns-runtime` —
//! the whole point of the example.

use std::path::Path;

use everruns::Model;
use everruns_coding_cli::agent_builder;

#[tokio::test]
async fn offline_smoke_runs_a_turn() {
    let agent = agent_builder()
        .model(Model::simulated("Done."))
        .build()
        .expect("agent builds via the facade");
    let mut session = agent.session();
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
    let mut session = agent.session();

    let first = session.run("first prompt").await.expect("first turn");
    let second = session.run("second prompt").await.expect("second turn");

    assert!(first.success && second.success);
    assert_eq!(first.response, "ack");
    assert_eq!(second.response, "ack");
    // A stable session id ties the turns together.
    assert!(!session.id().is_empty());
}

/// The example must depend only on `everruns`. Scan its own sources and manifest
/// for any direct use of the internal crates and fail loudly if one creeps in.
#[test]
fn sources_do_not_reference_core_or_runtime() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));

    // Scan the example's own sources. This test file is excluded on purpose —
    // it necessarily contains the forbidden identifiers as the search needles.
    let forbidden_in_rust = ["everruns_core", "everruns_runtime"];
    for file in ["src/lib.rs", "src/main.rs"] {
        let text = std::fs::read_to_string(root.join(file)).unwrap_or_default();
        for needle in forbidden_in_rust {
            assert!(
                !text.contains(needle),
                "{file} references `{needle}`; the example must use only the `everruns` facade"
            );
        }
    }

    // Check dependency declarations, not prose — strip `#` comments per line so a
    // crate named in an explanatory comment is not a false positive.
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    for line in manifest.lines() {
        let code = line.split('#').next().unwrap_or("");
        for needle in ["everruns-core", "everruns-runtime", "everruns-anthropic"] {
            assert!(
                !code.contains(needle),
                "Cargo.toml declares `{needle}`; the example must depend only on `everruns`"
            );
        }
    }
}

//! EVE-837 dependency-direction guard.
//!
//! The allowed identity-crate direction is `everruns-platform -> everruns-core`.
//! `everruns-core` must never gain a dependency edge back on
//! `everruns-platform`; if it did, the two crates would form a cycle and the
//! platform aggregates could no longer be carved out of core. This test fails
//! the core build the moment a manifest edit introduces the reverse edge.

use std::path::Path;

#[test]
fn core_manifest_has_no_edge_to_platform() {
    let manifest_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let manifest = std::fs::read_to_string(&manifest_path)
        .unwrap_or_else(|e| panic!("read {}: {e}", manifest_path.display()));

    assert!(
        !declares_platform_edge(&manifest),
        "everruns-core must not depend on everruns-platform \
         (allowed direction is platform -> core)"
    );
}

/// Whether the manifest *declares* a dependency on `everruns-platform`.
///
/// Comments are stripped first: prose that merely names the crate — such as a
/// note recording where a module moved — documents the boundary rather than
/// crossing it, and must not fail the build.
fn declares_platform_edge(manifest: &str) -> bool {
    manifest
        .lines()
        .map(|line| line.split_once('#').map_or(line, |(code, _)| code))
        .any(|code| code.contains("everruns-platform"))
}

#[test]
fn comments_naming_the_crate_are_not_edges() {
    assert!(!declares_platform_edge(
        "# email senders moved out of core (everruns-mcp, everruns-platform)\nserde = \"1\"\n"
    ));
    assert!(declares_platform_edge(
        "everruns-platform = { path = \"../platform\" }"
    ));
    assert!(declares_platform_edge(
        "everruns-platform = { path = \"../platform\" } # still an edge"
    ));
}

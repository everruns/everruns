//! Dependency-direction guard for the neutral host implementation boundary.

use std::path::Path;

#[test]
fn host_manifest_has_no_edge_to_facades_or_adapters() {
    let manifest_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let manifest = std::fs::read_to_string(&manifest_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", manifest_path.display()));

    for forbidden in [
        "everruns-runtime",
        "everruns-local",
        "everruns-worker",
        "everruns-server",
        "everruns-durable",
    ] {
        assert!(
            !manifest.contains(forbidden),
            "everruns-host must not depend on {forbidden}; adapters and facades depend on host"
        );
    }
}

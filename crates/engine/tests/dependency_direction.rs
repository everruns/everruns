//! EVE-840 dependency-direction guard.
//!
//! The allowed direction is `everruns-engine -> core/provider/capability`.
//! Planning is sans I/O and the execution kernel reaches effects only through
//! injected contracts. Deployment hosts and backends depend on engine, never
//! the reverse.

use std::path::Path;

#[test]
fn engine_manifest_has_no_edge_to_hosts_or_backends() {
    let manifest_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let manifest = std::fs::read_to_string(&manifest_path)
        .unwrap_or_else(|e| panic!("read {}: {e}", manifest_path.display()));

    for forbidden in [
        "everruns-host",
        "everruns-server",
        "everruns-worker",
        "everruns-platform",
        "everruns-durable",
        "everruns-scale",
    ] {
        assert!(
            !manifest.contains(forbidden),
            "everruns-engine must not depend on {forbidden} \
             (deployment hosts and backends depend on the shared engine, not the reverse)"
        );
    }
}

#[test]
fn core_no_longer_owns_atom_implementations() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    assert!(
        !workspace.join("crates/core/src/atoms").exists(),
        "everruns-core must not regain an atom implementation directory"
    );

    let core_lib = std::fs::read_to_string(workspace.join("crates/core/src/lib.rs"))
        .expect("read core lib.rs");
    assert!(
        !core_lib.contains("pub mod atoms"),
        "everruns-core must not expose the removed atom compatibility module"
    );
}

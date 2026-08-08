//! EVE-840 dependency-direction guard.
//!
//! The allowed direction is `everruns-engine -> everruns-core`. The engine is
//! the pure, sans-IO turn planner: it must never depend on the hosts that drive
//! it (runtime/server/worker) or the backend crates (platform/durable). If it
//! did, the planning brain could no longer be shared by every host without
//! dragging a host's I/O in with it. This test fails the engine build the moment
//! a manifest edit introduces one of those reverse edges.

use std::path::Path;

#[test]
fn engine_manifest_has_no_edge_to_hosts_or_backends() {
    let manifest_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let manifest = std::fs::read_to_string(&manifest_path)
        .unwrap_or_else(|e| panic!("read {}: {e}", manifest_path.display()));

    for forbidden in [
        "everruns-runtime",
        "everruns-server",
        "everruns-worker",
        "everruns-platform",
        "everruns-durable",
    ] {
        assert!(
            !manifest.contains(forbidden),
            "everruns-engine must not depend on {forbidden} \
             (the engine is a pure, sans-IO planner; hosts depend on it, not the reverse)"
        );
    }
}

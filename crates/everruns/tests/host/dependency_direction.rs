//! Dependency-direction guard for the neutral host implementation boundary.

use std::path::Path;

#[test]
fn host_manifest_has_no_edge_to_facades_or_adapters() {
    let manifest_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../core/Cargo.toml");
    let manifest = std::fs::read_to_string(&manifest_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", manifest_path.display()));

    // Skip prose comments while retaining aliased package declarations.
    let declarations = manifest
        .lines()
        .map(|line| line.split('#').next().unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\n");

    for forbidden in [
        "everruns =",
        "everruns-worker",
        "everruns-server",
        "everruns-durable",
        "everruns-platform",
        "everruns-capabilities",
        "everruns-drivers",
        "everruns-integrations-",
    ] {
        assert!(
            !declarations.contains(forbidden),
            "the core host module must not depend on {forbidden}; adapters and facades depend on host"
        );
    }

    let engine_manifest = manifest_path
        .parent()
        .expect("host crate directory")
        .parent()
        .expect("crates directory")
        .join("core/Cargo.toml");
    let engine = std::fs::read_to_string(&engine_manifest)
        .unwrap_or_else(|error| panic!("read {}: {error}", engine_manifest.display()));
    assert!(
        !engine
            .lines()
            .any(|line| line.trim_start().starts_with("everruns-host =")),
        "the sans-I/O engine must not depend on the effectful host"
    );
}

#[test]
fn host_exposes_no_writable_message_store_contract() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../core/src/host");
    for forbidden in ["MessageStore", "PersistingEventEmitter"] {
        for entry in std::fs::read_dir(&source).expect("read host source") {
            let path = entry.expect("host source entry").path();
            if path.extension().is_some_and(|extension| extension == "rs") {
                let contents = std::fs::read_to_string(&path).expect("read host source file");
                assert!(
                    !contents.contains(forbidden),
                    "{} must not define legacy writable history marker {forbidden}",
                    path.display()
                );
            }
        }
    }

    let backends =
        std::fs::read_to_string(source.join("backends.rs")).expect("read canonical backend bundle");
    assert!(
        !backends.contains("message_store"),
        "HostBackends must use EventLog, never a writable message store"
    );
}

#[test]
fn direct_egress_is_host_owned_without_a_standalone_http_crate() {
    let host_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let crates_dir = host_dir.parent().expect("crates directory");
    let repo = crates_dir.parent().expect("repository root");

    assert!(
        !crates_dir.join("http/Cargo.toml").exists(),
        "direct egress belongs to everruns-host, not a generic everruns-http package"
    );
    for manifest in [
        repo.join("Cargo.toml"),
        crates_dir.join("mcp/Cargo.toml"),
        crates_dir.join("ard/Cargo.toml"),
    ] {
        let contents = std::fs::read_to_string(&manifest)
            .unwrap_or_else(|error| panic!("read {}: {error}", manifest.display()));
        assert!(
            !contents.contains("everruns-http"),
            "{} must not restore the removed HTTP package edge",
            manifest.display()
        );
    }
}

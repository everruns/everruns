//! EVE-904 source/API guard for the focused execution-contract boundary.

use std::fs;
use std::path::Path;

const CONTRACT_MODULES: &[&str] = &[
    "connection_services.rs",
    "delegation_services.rs",
    "durability.rs",
    "event_emitter.rs",
    "execution_loading.rs",
    "image_services.rs",
    "provider_resolution.rs",
    "session_files.rs",
    "session_services.rs",
    "tool_context.rs",
    "tool_execution.rs",
];

#[test]
fn contracts_stay_focused_and_host_composition_stays_out() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    assert!(!src.join("traits.rs").exists(), "traits.rs must not return");

    let lib = fs::read_to_string(src.join("lib.rs")).expect("read core lib.rs");
    assert!(
        !lib.contains("mod traits"),
        "a traits module must not return"
    );

    for module in CONTRACT_MODULES {
        let path = src.join(module);
        let text = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read focused module {}: {error}", path.display()));
        let lines = text.lines().count();
        assert!(
            lines < 1_000,
            "{} has grown to {lines} lines; split it by execution concern before it becomes a new service bag",
            path.display()
        );
        for forbidden in [
            "pub struct Noop",
            "pub struct InMemory",
            "pub struct Postgres",
        ] {
            assert!(
                !text.contains(forbidden),
                "{} contains backend/test implementation marker {forbidden:?}",
                path.display()
            );
        }
    }

    let all_contracts = CONTRACT_MODULES
        .iter()
        .map(|module| fs::read_to_string(src.join(module)).expect("read contract module"))
        .collect::<Vec<_>>()
        .join("\n");
    for host_only in [
        "SessionFileSystemFactory",
        "SessionFileSystemFactoryContext",
        "DisabledSessionFileSystemFactory",
    ] {
        assert!(
            !all_contracts.contains(host_only),
            "host-only composition contract {host_only} must stay in everruns-host"
        );
    }
}

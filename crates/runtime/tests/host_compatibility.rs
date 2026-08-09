//! Compile-time proof that the 0.17 runtime host paths are aliases for the
//! canonical implementation in `everruns-host`, not a parallel contract.

use everruns_core::traits::SessionStore;

fn requires_canonical_host<T: everruns_host::RuntimeHostAdapter>() {}

fn runtime_bound_implies_canonical_bound<T: everruns_runtime::RuntimeHostAdapter>() {
    requires_canonical_host::<T>();
}

fn context_round_trip(
    context: everruns_runtime::RuntimeHostTurnContext,
) -> everruns_host::RuntimeHostTurnContext {
    context
}

#[test]
fn runtime_host_paths_are_canonical_host_reexports() {
    let _ = runtime_bound_implies_canonical_bound::<everruns_runtime::InProcessRuntime>;
    let _ = context_round_trip;

    fn session_store_type_checks<T: SessionStore + ?Sized>() {}
    let _ = session_store_type_checks::<dyn SessionStore>;
}

#[test]
fn runtime_has_no_host_implementation_file() {
    let runtime_src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    assert!(
        !runtime_src.join("host.rs").exists(),
        "host implementation must live in everruns-host"
    );

    let lib = std::fs::read_to_string(runtime_src.join("lib.rs")).expect("read runtime lib.rs");
    assert!(
        lib.contains("pub use everruns_host"),
        "runtime must preserve its host paths by re-exporting everruns-host"
    );
}

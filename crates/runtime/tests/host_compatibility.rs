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
fn runtime_source_is_compatibility_glue_only() {
    let runtime_src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut source_files = std::fs::read_dir(&runtime_src)
        .expect("read runtime source directory")
        .map(|entry| entry.expect("runtime source entry").file_name())
        .collect::<Vec<_>>();
    source_files.sort();
    assert_eq!(
        source_files,
        [std::ffi::OsString::from("lib.rs")],
        "everruns-runtime may contain compatibility glue only"
    );

    let lib = std::fs::read_to_string(runtime_src.join("lib.rs")).expect("read runtime lib.rs");
    assert!(
        lib.contains("pub use everruns_host"),
        "runtime must preserve its host paths by re-exporting everruns-host"
    );
    for forbidden in [
        "struct InProcessRuntime",
        "struct RuntimeBackends",
        "fn plan_next_host_turn",
        "fn execute_reason_activity",
        "mod runtime",
        "mod backends",
    ] {
        assert!(
            !lib.contains(forbidden),
            "runtime compatibility glue must not own implementation marker {forbidden}"
        );
    }
}

async fn execution_observation(
    runtime: everruns_host::InProcessRuntime,
) -> (String, Vec<String>, Vec<String>) {
    let session_id = runtime.default_session_id().expect("single session");
    let result = runtime
        .run_text_turn(session_id, "hello")
        .await
        .expect("turn executes");
    let messages = runtime
        .messages(session_id)
        .await
        .expect("history projects from events")
        .into_iter()
        .map(|message| format!("{:?}:{}", message.role, message.text().unwrap_or_default()))
        .collect();
    let event_types = runtime
        .events()
        .await
        .expect("canonical events replay")
        .into_iter()
        .map(|event| event.event_type)
        .collect();
    (result.response, messages, event_types)
}

#[tokio::test]
async fn old_and_new_paths_execute_the_same_canonical_host_behavior() {
    let old_path: everruns_host::InProcessRuntime =
        everruns_runtime::InProcessRuntimeBuilder::new()
            .llm_sim(everruns_core::llmsim_driver::LlmSimConfig::fixed(
                "same response",
            ))
            .single_session(|session| {
                session
                    .harness("compat", "compatibility")
                    .agent("compat-agent", "respond")
            })
            .build()
            .await
            .expect("runtime compatibility builder");
    let new_path = everruns_host::InProcessRuntimeBuilder::new()
        .llm_sim(everruns_core::llmsim_driver::LlmSimConfig::fixed(
            "same response",
        ))
        .single_session(|session| {
            session
                .harness("compat", "compatibility")
                .agent("compat-agent", "respond")
        })
        .build()
        .await
        .expect("canonical host builder");

    assert_eq!(
        execution_observation(old_path).await,
        execution_observation(new_path).await
    );
}

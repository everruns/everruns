use everruns::{
    Agent, CapabilityRef, ComputeCapabilities, ContainmentLevel, Harness, HarnessBuildError,
    InMemoryEngine, Model,
};
use serde_json::json;

fn simulated_agent() -> Agent {
    Agent::builder()
        .instructions("Reply deterministically.")
        .model(Model::simulated("ok"))
        .capability("current_time")
        .build()
        .expect("valid simulated agent")
}

fn filesystem_harness() -> Harness {
    Harness::builder("generic")
        .capability("session_file_system")
        .build()
        .expect("valid harness")
}

#[test]
fn harness_holds_environment_requirements_and_model_default() {
    let required = ComputeCapabilities {
        native_processes: true,
        packages: true,
        ..Default::default()
    };
    let harness = Harness::builder("coding")
        .requires_capabilities(required)
        .requires_containment(ContainmentLevel::Isolated)
        .model("fallback")
        .build()
        .expect("valid harness");

    assert_eq!(harness.name(), "coding");
    assert_eq!(harness.required_capabilities(), required);
    assert_eq!(harness.required_containment(), ContainmentLevel::Isolated);
    assert!(harness.default_model().is_some());
}

#[test]
fn harness_rejects_invalid_and_duplicate_capabilities() {
    let invalid = Harness::builder("invalid")
        .capability("bad capability")
        .build()
        .unwrap_err();
    assert!(matches!(
        invalid,
        HarnessBuildError::InvalidCapability { .. }
    ));

    let duplicate = Harness::builder("duplicate")
        .capability("session_file_system")
        .capability("session_file_system")
        .build()
        .unwrap_err();
    assert_eq!(
        duplicate,
        HarnessBuildError::DuplicateCapability {
            id: "session_file_system".to_string()
        }
    );
}
#[test]
fn harness_debug_redacts_capability_configuration() {
    let harness = Harness::builder("private")
        .capability(CapabilityRef::new("vendor.custom").config(json!({
            "credential": "must-not-appear"
        })))
        .build()
        .expect("valid harness");

    assert!(!format!("{harness:?}").contains("must-not-appear"));
}

#[tokio::test]
async fn bound_harness_composes_with_agent_capabilities() {
    let engine = InMemoryEngine::new();
    let plain = engine.create(simulated_agent());
    let bound = engine
        .create(simulated_agent())
        .harness(filesystem_harness())
        .start()
        .await
        .expect("harness binds");

    let plain_context = plain.inspect().await.expect("plain context");
    let bound_context = bound.inspect().await.expect("bound context");
    assert!(
        !plain_context
            .tools
            .iter()
            .any(|tool| tool.name == "read_file")
    );
    assert!(
        plain_context
            .tools
            .iter()
            .any(|tool| tool.name == "get_current_time")
    );
    assert!(
        bound_context
            .tools
            .iter()
            .any(|tool| tool.name == "read_file")
    );
    assert!(
        bound_context
            .tools
            .iter()
            .any(|tool| tool.name == "get_current_time")
    );
}

#[tokio::test]
async fn harness_binding_survives_engine_resume() {
    let engine = InMemoryEngine::new();
    let session = engine
        .create(simulated_agent())
        .harness(filesystem_harness())
        .start()
        .await
        .expect("harness binds");
    let session_id = session.session_id();
    drop(session);

    let resumed = engine.resume(session_id).await.expect("session resumes");
    let context = resumed.inspect().await.expect("resumed context");
    assert!(context.tools.iter().any(|tool| tool.name == "read_file"));
}

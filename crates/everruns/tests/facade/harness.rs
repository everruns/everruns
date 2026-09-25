use everruns::{
    Agent, CapabilityRef, ComputeCapabilities, ContainmentLevel, Harness, HarnessBuildError,
    InMemoryEngine, Model, SessionEnvironmentError,
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
    assert_eq!(harness.default_model(), Some("fallback"));

    let without_default = Harness::builder("minimal").build().expect("valid harness");
    assert_eq!(without_default.default_model(), None);
}

#[test]
fn harness_rejects_invalid_and_duplicate_capabilities() {
    for name in ["", " ", "\t\n"] {
        assert_eq!(
            Harness::builder(name).build().unwrap_err(),
            HarnessBuildError::BlankName
        );
    }
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
fn harness_definition_round_trips_without_runtime_identity() {
    let required = ComputeCapabilities {
        native_processes: true,
        packages: true,
        pty: true,
        ..Default::default()
    };
    let harness = Harness::builder("coding")
        .requires_capabilities(required)
        .requires_containment(ContainmentLevel::Isolated)
        .capability(CapabilityRef::new("vendor.custom").config(json!({
            "profile": "portable"
        })))
        .model("fallback")
        .build()
        .expect("valid harness");

    let value = serde_json::to_value(&harness).expect("harness serializes");
    assert!(value.get("id").is_none());
    let round_tripped: Harness = serde_json::from_value(value).expect("harness deserializes");

    assert_eq!(round_tripped.name(), "coding");
    assert_eq!(round_tripped.required_capabilities(), required);
    assert_eq!(
        round_tripped.required_containment(),
        ContainmentLevel::Isolated
    );
    assert_eq!(round_tripped.default_model(), Some("fallback"));
    assert_eq!(round_tripped.capabilities().len(), 1);
    assert_eq!(round_tripped.capabilities()[0].id(), "vendor.custom");
    assert_eq!(
        round_tripped.capabilities()[0].config_value(),
        &json!({ "profile": "portable" })
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

#[tokio::test]
async fn default_environment_negotiation_returns_typed_public_error() {
    let engine = InMemoryEngine::new();
    let session = engine.create(simulated_agent());
    let session_id = session.session_id();
    let harness = Harness::builder("coding")
        .requires_capabilities(ComputeCapabilities {
            native_processes: true,
            ..Default::default()
        })
        .build()
        .expect("valid harness");

    let error = session
        .harness(harness)
        .start()
        .await
        .err()
        .expect("default file-only environment lacks native processes");

    assert_eq!(
        error,
        SessionEnvironmentError::MissingHarnessCapability {
            capability: "native_processes"
        }
    );

    let resumed = engine
        .resume(session_id)
        .await
        .expect("failed negotiation leaves the session reopenable");
    assert!(
        resumed.workspace_head().is_none(),
        "failed negotiation must not persist an environment binding"
    );
}

use everruns_core::llmsim_driver::{LlmSimConfig, LlmSimDriver};
use everruns_core::{DriverId, DriverRegistry, InputMessage, ModelSpec, Provider, ResolvedModel};
use everruns_runtime::InProcessRuntimeBuilder;

fn registry(response: &str) -> DriverRegistry {
    let mut registry = DriverRegistry::new();
    registry
        .register_provider(Provider::new(
            "llmsim",
            LlmSimDriver::new(LlmSimConfig::fixed(response)),
        ))
        .unwrap();
    registry
}

fn builder(registry: DriverRegistry) -> InProcessRuntimeBuilder {
    InProcessRuntimeBuilder::new()
        .driver_registry(registry)
        .single_session(|session| {
            session
                .harness("assistant", "Be concise.")
                .agent("assistant-agent", "Answer.")
        })
}

#[tokio::test]
async fn legacy_and_canonical_setup_share_provider_execution() {
    let legacy = builder(registry("same-path"))
        .default_model(ResolvedModel {
            model: "llmsim-model".into(),
            provider_type: DriverId::LlmSim,
            api_key: None,
            base_url: None,
            provider_metadata: None,
        })
        .build()
        .await
        .unwrap();
    let canonical = builder(registry("same-path"))
        .model_spec(ModelSpec::on("llmsim", "llmsim-model"))
        .build()
        .await
        .unwrap();

    let legacy_result = legacy
        .run_turn(
            legacy.default_session_id().unwrap(),
            InputMessage::user("hello"),
        )
        .await
        .unwrap();
    let canonical_result = canonical
        .run_turn(
            canonical.default_session_id().unwrap(),
            InputMessage::user("hello"),
        )
        .await
        .unwrap();

    assert_eq!(legacy_result.response, canonical_result.response);
    assert_eq!(legacy_result.response, "same-path");
}

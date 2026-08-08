//! Facade smoke test (EVE-830): a clean program that depends only on `everruns`
//! must be able to build one session and run one turn without importing
//! `everruns-core` or `everruns-runtime`.

use everruns::{DriverId, InProcessRuntimeBuilder, InputMessage, LlmSimConfig, ResolvedModel};

#[tokio::test]
async fn facade_runs_one_simulated_turn() {
    let runtime = InProcessRuntimeBuilder::new()
        .single_session(|s| {
            s.harness("assistant", "You are a helpful assistant.")
                .harness_display_name("Assistant")
                .agent("assistant-agent", "Answer the user.")
                .agent_display_name("Assistant Agent")
                .agent_max_iterations(4)
                .session_title("Facade Smoke")
        })
        .llm_sim(LlmSimConfig::fixed("4"))
        .default_model(ResolvedModel {
            model: "llmsim-model".into(),
            provider_type: DriverId::LlmSim,
            api_key: Some("fake-key".into()),
            base_url: None,
            provider_metadata: None,
        })
        .build()
        .await
        .expect("runtime builds");

    let session_id = runtime
        .default_session_id()
        .expect("single_session yields a default session id");

    let result = runtime
        .run_turn(session_id, InputMessage::user("What is 2 + 2?"))
        .await
        .expect("turn runs");

    assert!(result.success, "turn should succeed: {:?}", result.error);
    assert_eq!(result.response, "4");
}

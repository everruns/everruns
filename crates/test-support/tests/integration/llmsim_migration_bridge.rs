use everruns_host::InProcessRuntimeBuilder;
use everruns_provider::model_spec::ModelSpec;
use everruns_provider::runtime_provider::Provider;
use everruns_test_support::llmsim_driver::SimTurn;
use everruns_test_support::{LlmSimConfig, LlmSimDriver, LlmSimRuntimeExt};

// Guards the 0.18 migration bridge documented on the crate root: code written
// against the 0.17 simulator paths (`everruns_test_support::{LlmSimConfig,
// LlmSimDriver}` and the `llmsim_driver` module) must keep compiling *and*
// keep working against the current `everruns_llmsim`/`everruns_host` types,
// not just type-check as a no-op construction.
#[tokio::test]
async fn zero_eighteen_bridge_preserves_zero_seventeen_simulator_paths() {
    let config = LlmSimConfig::scripted(vec![SimTurn::Assistant("ok".into())]);
    // Assigning without an explicit conversion proves the re-exported
    // `LlmSimConfig` is the same type as `everruns_llmsim::LlmSimConfig`, not
    // just a similarly-shaped duplicate.
    let direct_config: everruns_llmsim::LlmSimConfig = config.clone();

    // Path 1: construct the driver directly, bypassing the `.llm_sim*(...)`
    // sugar, and register it as an ordinary provider.
    let direct_driver_runtime = InProcessRuntimeBuilder::new()
        .provider(Provider::new("direct", LlmSimDriver::new(direct_config)))
        .default_model(ModelSpec::on("direct", "direct-model"))
        .single_session(|session| {
            session
                .harness("chat", "You are concise.")
                .agent("chat-agent", "Reply once.")
        })
        .build()
        .await
        .expect("direct-driver runtime builds");
    let direct_driver_result = direct_driver_runtime
        .run_text_turn(
            direct_driver_runtime
                .default_session_id()
                .expect("default session"),
            "hello",
        )
        .await
        .expect("turn runs");
    assert_eq!(
        direct_driver_result.response, "ok",
        "directly constructed LlmSimDriver should serve the scripted response"
    );

    // Path 2: registration-only `.llm_sim(...)` must not select a default
    // model on its own; an explicit `default_model` still wins.
    let registration_only_runtime = InProcessRuntimeBuilder::new()
        .provider(Provider::new(
            "selected",
            LlmSimDriver::new(LlmSimConfig::fixed("selected provider")),
        ))
        .default_model(ModelSpec::on("selected", "selected-model"))
        .llm_sim(config.clone())
        .single_session(|session| {
            session
                .harness("chat", "You are concise.")
                .agent("chat-agent", "Reply once.")
        })
        .build()
        .await
        .expect("registration-only runtime builds");
    let registration_only_result = registration_only_runtime
        .run_text_turn(
            registration_only_runtime
                .default_session_id()
                .expect("default session"),
            "hello",
        )
        .await
        .expect("turn runs");
    assert_eq!(
        registration_only_result.response, "selected provider",
        "registration-only .llm_sim(...) must not override an explicit default model"
    );

    // Path 3: `.llm_sim_as_default(...)` registers the simulator *and*
    // selects it as the default model.
    let explicit_default_runtime = InProcessRuntimeBuilder::new()
        .llm_sim_as_default(config)
        .single_session(|session| {
            session
                .harness("chat", "You are concise.")
                .agent("chat-agent", "Reply once.")
        })
        .build()
        .await
        .expect("explicit-default runtime builds");
    let explicit_default_result = explicit_default_runtime
        .run_text_turn(
            explicit_default_runtime
                .default_session_id()
                .expect("default session"),
            "hello",
        )
        .await
        .expect("turn runs");
    assert_eq!(
        explicit_default_result.response, "ok",
        ".llm_sim_as_default(...) should serve the scripted simulator response"
    );
}

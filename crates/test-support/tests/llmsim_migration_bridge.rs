use everruns_host::InProcessRuntimeBuilder;
use everruns_test_support::llmsim_driver::SimTurn;
use everruns_test_support::{LlmSimConfig, LlmSimDriver, LlmSimRuntimeExt};

#[test]
fn zero_eighteen_bridge_preserves_zero_seventeen_simulator_paths() {
    let config = LlmSimConfig::scripted(vec![SimTurn::Assistant("ok".into())]);
    let direct_config: everruns_llmsim::LlmSimConfig = config.clone();

    let _driver = LlmSimDriver::new(direct_config);
    let _registration_only = InProcessRuntimeBuilder::new().llm_sim(config.clone());
    let _explicit_default = InProcessRuntimeBuilder::new().llm_sim_as_default(config);
}

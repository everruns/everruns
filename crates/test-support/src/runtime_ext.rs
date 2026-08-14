// `.llm_sim(...)` convenience for the in-process host runtime builder.
//
// `everruns-host` stays free of simulation code: its builder exposes the
// neutral `provider_with_default_model` seam, and this extension trait adapts
// an `LlmSimConfig` into an `llmsim` provider on top of it.

use crate::llmsim_driver::{LlmSimConfig, LlmSimDriver};
use everruns_host::InProcessRuntimeBuilder;

/// Canonical provider name and model id used by simulated runtimes.
pub const LLMSIM_PROVIDER: &str = "llmsim";
/// Canonical model id served by the `llmsim` provider.
pub const LLMSIM_MODEL_ID: &str = "llmsim-model";

/// Build an `llmsim` provider from a simulator configuration.
///
/// Useful with builders that expose a raw provider seam (e.g.
/// `LocalRuntimeBuilder::provider_with_default_model`).
pub fn llm_sim_provider(config: LlmSimConfig) -> everruns_provider::runtime_provider::Provider {
    everruns_provider::runtime_provider::Provider::new(LLMSIM_PROVIDER, LlmSimDriver::new(config))
}

/// Deterministic-simulation sugar for runtime builders.
pub trait LlmSimRuntimeExt: Sized {
    /// Register the in-process `llmsim` driver and default the runtime model
    /// to `llmsim-model` when no other default is configured.
    fn llm_sim(self, config: LlmSimConfig) -> Self;
}

impl LlmSimRuntimeExt for InProcessRuntimeBuilder {
    fn llm_sim(self, config: LlmSimConfig) -> Self {
        self.provider_with_default_model(llm_sim_provider(config), LLMSIM_MODEL_ID)
    }
}

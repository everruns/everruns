// LLM driver factory helpers
//
// Decision: Workers use gRPC adapters for database operations, not direct DB access.
// This module only contains LLM driver factory helpers.

use everruns_provider::driver_registry::{BoxedChatDriver, DriverRegistry, ProviderConfig};
use everruns_provider::error::Result;
use everruns_provider::provider::DriverId;

/// Create and configure the driver registry with all supported LLM providers
///
/// This registers drivers for:
/// - OpenAI (Open Responses API - recommended)
/// - OpenAI Completions (Chat Completions API - backward compatibility)
/// - OpenRouter (OpenAI-compatible Responses API)
/// - Microsoft MAI (Azure AI Foundry, OpenAI-compatible Chat Completions)
/// - Fireworks AI (open models, OpenAI-compatible Chat Completions)
/// - Meta Model API (Muse Spark, OpenAI-compatible Responses API)
/// - Anthropic Claude
/// - Google Gemini
/// - LlmSim (for testing)
pub fn create_driver_registry() -> DriverRegistry {
    let mut registry = DriverRegistry::new();
    everruns_openai::register_driver(&mut registry);
    everruns_openrouter::register_driver(&mut registry);
    everruns_mai::register_driver(&mut registry);
    everruns_fireworks::register_driver(&mut registry);
    everruns_meta::register_driver(&mut registry);
    everruns_anthropic::register_driver(&mut registry);
    everruns_gemini::register_driver(&mut registry);
    everruns_bedrock::register_driver(&mut registry);

    // LlmSim (from `everruns-test-support`, `sim` surface only — the worker
    // never links the demo fixture capabilities). The `LLMSIM_DEMO` env var
    // lets operators opt the in-process LlmSim driver into a pre-baked
    // scripted scenario without changing the default ("Hello! I'm a
    // simulated LLM response.") behavior.
    //
    // Recognized values:
    //   - `guarded` — drives the agent to attempt `rm -rf /` followed by a
    //     safe `ls`. With a matching `pre_tool_use` block hook the first
    //     call gets refused and the second succeeds — a live demo of the
    //     pre_tool_use block path.
    //   - `tasks` — starts a background bash run via `spawn_background` and
    //     lists it with `list_tasks`, exercising the session task registry
    //     end-to-end (knowledge/runtime-resources/session-tasks.md) without an LLM API key.
    //   - `monitor` — schedules a recurring monitor via `spawn_background`
    //     with a `schedule`, exercising the `monitor` task kind and its
    //     fire-history thread.
    //
    // The former `auditor` scenario was removed with EVE-875: it depended on
    // the fake_aws demo capability, which product registries no longer carry.
    //
    // Unknown values fall back to the default driver with a warning.
    match std::env::var("LLMSIM_DEMO").ok().as_deref() {
        Some("guarded") => {
            tracing::info!(
                "LLMSIM_DEMO=guarded: registering scripted LlmSim driver for the guarded-bash pre_tool_use demo"
            );
            everruns_test_support::llmsim_driver::register_driver_with_config(
                &mut registry,
                everruns_test_support::llmsim_driver::guarded_bash_demo_script(),
            );
        }
        Some("tasks") => {
            tracing::info!(
                "LLMSIM_DEMO=tasks: registering scripted LlmSim driver for the session tasks demo"
            );
            everruns_test_support::llmsim_driver::register_driver_with_config(
                &mut registry,
                everruns_test_support::llmsim_driver::session_tasks_demo_script(),
            );
        }
        Some("monitor") => {
            tracing::info!(
                "LLMSIM_DEMO=monitor: registering scripted LlmSim driver for the monitor demo"
            );
            everruns_test_support::llmsim_driver::register_driver_with_config(
                &mut registry,
                everruns_test_support::llmsim_driver::monitor_demo_script(),
            );
        }
        Some(other) => {
            tracing::warn!(
                value = %other,
                "LLMSIM_DEMO has an unrecognized value; falling back to default LlmSim driver"
            );
            everruns_test_support::llmsim_driver::register_driver(&mut registry);
        }
        None => {
            everruns_test_support::llmsim_driver::register_driver(&mut registry);
        }
    }

    registry
}

/// Create an LLM driver based on configuration
///
/// This factory supports all provider types: OpenAI, OpenAI Completions, Anthropic.
pub fn create_chat_driver(
    provider_type: &str,
    api_key: Option<&str>,
    base_url: Option<&str>,
) -> Result<BoxedChatDriver> {
    // Parsing is infallible: unknown ids become External providers.
    let ptype: DriverId = provider_type.parse().unwrap_or_else(|_| unreachable!());

    let mut config = ProviderConfig::new(ptype);
    if let Some(key) = api_key {
        config = config.with_api_key(key);
    }
    if let Some(url) = base_url {
        config = config.with_base_url(url);
    }

    let registry = create_driver_registry();
    registry.create_chat_driver(&config)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The product's default platform composition must register every official
    /// provider driver (EVE-874). Protocol drivers live in provider crates on
    /// the everruns-provider SPI; the worker is the composition point that
    /// assembles them into the default product registry, so a provider crate
    /// dropping out of the assembly fails here.
    #[test]
    fn default_registry_registers_all_official_providers() {
        let registry = create_driver_registry();
        let official = [
            DriverId::OpenAI,
            DriverId::OpenAICompletions,
            DriverId::OpenRouter,
            DriverId::Mai,
            DriverId::Fireworks,
            DriverId::Meta,
            DriverId::Anthropic,
            DriverId::Gemini,
            DriverId::Bedrock,
            // Seeded simulation driver (test-support `sim` surface).
            DriverId::LlmSim,
        ];
        for id in official {
            assert!(
                registry.has_driver(&id),
                "default product registry is missing official driver {id:?}"
            );
        }
    }
}

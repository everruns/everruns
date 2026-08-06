// LLM driver factory helpers
//
// Decision: Workers use gRPC adapters for database operations, not direct DB access.
// This module only contains LLM driver factory helpers.

use everruns_core::{BoxedChatDriver, DriverId, DriverRegistry, ProviderConfig, Result};

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

    // LlmSim. The `LLMSIM_DEMO` env var lets operators opt the in-process
    // LlmSim driver into a pre-baked scripted scenario without changing
    // the default ("Hello! I'm a simulated LLM response.") behavior.
    //
    // Recognized values:
    //   - `auditor` — drives the Cloud Cost & Security Auditor agent
    //     through a short EC2/S3 audit using fake_aws tools. Useful for
    //     exercising the user_hooks audit-log (`post_tool_use`) bundle
    //     end-to-end without an LLM API key.
    //   - `guarded` — drives the agent to attempt `rm -rf /` followed by a
    //     safe `ls`. With a matching `pre_tool_use` block hook the first
    //     call gets refused and the second succeeds — a live demo of the
    //     pre_tool_use block path.
    //   - `tasks` — starts a background bash run via `spawn_background` and
    //     lists it with `list_tasks`, exercising the session task registry
    //     end-to-end (specs/session-tasks.md) without an LLM API key.
    //   - `monitor` — schedules a recurring monitor via `spawn_background`
    //     with a `schedule`, exercising the `monitor` task kind and its
    //     fire-history thread.
    //
    // Unknown values fall back to the default driver with a warning.
    match std::env::var("LLMSIM_DEMO").ok().as_deref() {
        Some("auditor") => {
            tracing::info!(
                "LLMSIM_DEMO=auditor: registering scripted LlmSim driver for the Cloud Auditor demo"
            );
            everruns_core::llmsim_driver::register_driver_with_config(
                &mut registry,
                everruns_core::llmsim_driver::auditor_demo_script(),
            );
        }
        Some("guarded") => {
            tracing::info!(
                "LLMSIM_DEMO=guarded: registering scripted LlmSim driver for the guarded-bash pre_tool_use demo"
            );
            everruns_core::llmsim_driver::register_driver_with_config(
                &mut registry,
                everruns_core::llmsim_driver::guarded_bash_demo_script(),
            );
        }
        Some("tasks") => {
            tracing::info!(
                "LLMSIM_DEMO=tasks: registering scripted LlmSim driver for the session tasks demo"
            );
            everruns_core::llmsim_driver::register_driver_with_config(
                &mut registry,
                everruns_core::llmsim_driver::session_tasks_demo_script(),
            );
        }
        Some("monitor") => {
            tracing::info!(
                "LLMSIM_DEMO=monitor: registering scripted LlmSim driver for the monitor demo"
            );
            everruns_core::llmsim_driver::register_driver_with_config(
                &mut registry,
                everruns_core::llmsim_driver::monitor_demo_script(),
            );
        }
        Some(other) => {
            tracing::warn!(
                value = %other,
                "LLMSIM_DEMO has an unrecognized value; falling back to default LlmSim driver"
            );
            everruns_core::llmsim_driver::register_driver(&mut registry);
        }
        None => {
            everruns_core::llmsim_driver::register_driver(&mut registry);
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

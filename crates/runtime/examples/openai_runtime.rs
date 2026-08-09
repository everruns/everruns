//! Run an in-process runtime turn against a live OpenAI model.
//! This is 0.17 compatibility coverage. New applications use the `everruns`
//! Framework; see <https://docs.everruns.com/framework/runtime-compatibility/>.
//!
//! Unlike the `in_process_runtime` example (which uses the deterministic
//! `llmsim` driver), this one talks to a real provider. The key difference is
//! that a real driver must be **registered** in the `DriverRegistry` before the
//! matching `ResolvedModel.provider_type` can be resolved — setting
//! `default_model` alone is not enough.
//!
//! Usage:
//!
//! ```bash
//! export OPENAI_API_KEY=sk-...
//! cargo run -p everruns-runtime --example openai_runtime
//! # optionally override the model (defaults to gpt-5.5):
//! OPENAI_MODEL=gpt-5.4-mini cargo run -p everruns-runtime --example openai_runtime
//! ```

use everruns_core::driver_registry::DriverRegistry;
use everruns_core::{CapabilityRegistry, DriverId, PlatformDefinition, ResolvedModel};
use everruns_runtime::InProcessRuntimeBuilder;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // No key, no live call. Exit cleanly so the example is safe to run in CI.
    let Ok(api_key) = std::env::var("OPENAI_API_KEY") else {
        eprintln!("OPENAI_API_KEY is not set; skipping the live OpenAI turn.");
        eprintln!("Set it and re-run: export OPENAI_API_KEY=sk-...");
        return Ok(());
    };
    let model = std::env::var("OPENAI_MODEL").unwrap_or_else(|_| "gpt-5.5".to_string());

    // 1. Register the OpenAI driver so `DriverId::OpenAI` can be resolved.
    let mut drivers = DriverRegistry::new();
    everruns_openai::register_driver(&mut drivers);

    // 2. Build the platform from the driver registry (no extra capabilities for
    //    a plain chat turn; register your own here to give the agent tools).
    let platform = PlatformDefinition::new(CapabilityRegistry::new(), drivers);

    // 3. Point the runtime's default model at OpenAI.
    let runtime = InProcessRuntimeBuilder::new()
        .platform_definition(platform)
        .default_model(ResolvedModel {
            model,
            provider_type: DriverId::OpenAI,
            api_key: Some(api_key),
            base_url: None,
            provider_metadata: None,
        })
        .single_session(|s| {
            s.harness("assistant", "You are a concise, helpful assistant.")
                .agent("assistant-agent", "Answer in one short sentence.")
                .session_title("OpenAI Session")
        })
        .build()
        .await?;

    // 4. Run a turn and print the model's reply.
    let session_id = runtime
        .default_session_id()
        .expect("single_session sets the default session id");
    let result = runtime
        .run_text_turn(
            session_id,
            "Say hello from everruns-runtime in one sentence.",
        )
        .await?;

    println!("success:    {}", result.success);
    println!("iterations: {}", result.iterations);
    println!("response:   {}", result.response);
    if let Some(error) = result.error {
        eprintln!("error:      {error}");
    }

    Ok(())
}

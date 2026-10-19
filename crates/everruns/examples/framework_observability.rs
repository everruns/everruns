//! Export every Engine session through OpenTelemetry and optional Braintrust.
//!
//! Run with:
//!
//! ```text
//! OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4318 \
//! cargo run -p everruns --features openai,otel,braintrust --example framework_observability
//! ```

use std::time::Duration;

use everruns::observability::{Braintrust, OpenTelemetry, install_otlp_from_env};
use everruns::{Agent, Engine, OpenAI};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let telemetry = install_otlp_from_env();
    let provider = telemetry
        .tracer_provider()
        .ok_or("OTEL_EXPORTER_OTLP_ENDPOINT is required")?;
    let mut engine = Engine::builder().observe(OpenTelemetry::from_env().tracer_provider(provider));
    if let Some(braintrust) = Braintrust::from_env() {
        engine = engine.observe(braintrust);
    }
    let engine = engine.build();

    let agent = Agent::builder()
        .instructions("Answer concisely.")
        .provider(OpenAI::from_env()?)
        .model("gpt-6.1-sol")
        .build()?;
    let turn = engine.create(agent).run("Say hello.").await?;
    println!("{}", turn.response);

    let report = engine.shutdown(Duration::from_secs(10)).await;
    if report.timed_out {
        return Err("observability shutdown exceeded its deadline".into());
    }
    if report
        .stats
        .listeners
        .iter()
        .any(|listener| !listener.flushed)
    {
        return Err("observability exporter flush failed".into());
    }
    telemetry.shutdown().await?;
    Ok(())
}

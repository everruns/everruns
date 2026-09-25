//! Export every Engine session through OpenTelemetry and optional Braintrust.
//!
//! Run with:
//!
//! ```text
//! OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4318 \
//! cargo run -p everruns --features otel,braintrust --example framework_observability
//! ```

use std::time::Duration;

use everruns::observability::{Braintrust, OpenTelemetry, install_otlp_from_env};
use everruns::{Agent, Engine, Model};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _telemetry = install_otlp_from_env();
    let mut engine = Engine::builder().observe(OpenTelemetry::from_env());
    if let Some(braintrust) = Braintrust::from_env() {
        engine = engine.observe(braintrust);
    }
    let engine = engine.build();

    let agent = Agent::builder()
        .instructions("Answer concisely.")
        .model(Model::simulated("Hello from an observed Engine."))
        .build()?;
    let turn = engine.create(agent).run("Say hello.").await?;
    println!("{}", turn.response);

    let report = engine.shutdown(Duration::from_secs(10)).await;
    if report.timed_out {
        return Err("observability shutdown exceeded its deadline".into());
    }
    Ok(())
}

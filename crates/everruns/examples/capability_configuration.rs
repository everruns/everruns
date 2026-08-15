//! Configure every capability kind through one open Framework entrypoint.
//!
//! Run offline with:
//! `cargo run -p everruns --example capability_configuration`

use everruns::{
    Agent, CapabilityRef, CompactionConfig, InMemoryEngine, Model, ToolSearch, capability,
};

#[derive(capability::Deserialize, capability::JsonSchema)]
#[serde(crate = "everruns::capability::serde")]
#[schemars(crate = "everruns::capability::schemars")]
struct WeatherInput {
    city: String,
}

#[derive(capability::Serialize, capability::JsonSchema)]
#[serde(crate = "everruns::capability::serde")]
#[schemars(crate = "everruns::capability::schemars")]
struct WeatherOutput {
    summary: String,
}

struct Weather;

#[capability::async_trait]
impl capability::Handler for Weather {
    type Input = WeatherInput;
    type Output = WeatherOutput;
    type Error = capability::Error;

    fn name(&self) -> &str {
        "weather_forecast"
    }

    fn description(&self) -> &str {
        "Return the sample forecast for one city."
    }

    async fn execute(
        &self,
        input: Self::Input,
        _context: capability::Context,
    ) -> Result<Self::Output, Self::Error> {
        Ok(WeatherOutput {
            summary: format!("Clear skies in {}", input.city),
        })
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let weather_definition = capability::Definition::new(
        "weather",
        "Weather",
        "Application-defined sample weather tools.",
    )
    .tool(Weather);

    let agent = Agent::builder()
        .instructions("Use configured capabilities when relevant.")
        .model(Model::simulated("Configured."))
        .capability(CompactionConfig::new().budget_percent(0.85))
        .capability(ToolSearch::automatic())
        .capability(weather_definition)
        .capability(
            CapabilityRef::new("vendor.custom")
                .config(capability::serde_json::json!({ "mode": "database-driven" })),
        )
        .build()?;

    let turn = InMemoryEngine::new()
        .create(agent)
        .run("Confirm the agent is configured.")
        .await?;
    println!("{}", turn.response);
    Ok(())
}

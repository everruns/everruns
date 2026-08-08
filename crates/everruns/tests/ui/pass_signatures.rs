//! Supported signatures must expand and compile.
use everruns::{Agent, Model};

/// A required argument and an optional one, returning a Result.
#[everruns::tool]
async fn weather(city: String, units: Option<String>) -> Result<serde_json::Value, String> {
    Ok(serde_json::json!({ "city": city, "units": units }))
}

/// A renamed argument and an explicit name/description.
#[everruns::tool(name = "add", description = "Add two numbers.")]
async fn adder(#[tool(rename = "left")] a: i64, b: i64) -> i64 {
    a + b
}

/// A unit return and no arguments.
#[everruns::tool]
async fn ping() {}

fn main() {
    let _agent = Agent::builder()
        .instructions("Use the tools.")
        .model(Model::simulated("ok"))
        .tool(weather())
        .tool(adder())
        .tool(ping())
        .build()
        .expect("agent builds");
}

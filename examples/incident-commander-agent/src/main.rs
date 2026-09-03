//! A self-contained production-incident coordinator.
//!
//! Run with `cargo run -p everruns-incident-commander-agent`.

use everruns::{Agent, Engine, LlmSimConfig, Model, Turn};
use everruns_llmsim::{SimToolCall, SimTurn};
use serde_json::json;

const PRODUCTION_MODEL: &str = "muse-spark-1.3";

#[everruns::tool]
/// Record a bounded, non-sensitive incident status update.
async fn record_incident_update(update: String) -> Result<String, String> {
    Ok(format!("incident update recorded: {update}"))
}

async fn run() -> Result<Turn, Box<dyn std::error::Error>> {
    let agent = Agent::builder()
        .name("incident-commander-agent")
        .instructions("You are an incident commander. Log evidence, coordinate owners, and never make unsupported production changes.")
        .model(Model::simulated_with_config(LlmSimConfig::scripted(vec![
            SimTurn::ToolCalls(vec![SimToolCall {
                name: "record_incident_update".into(),
                arguments: json!({"update": "error rate elevated after deployment"}),
                id: Some("record_incident_update_1".into()),
            }]),
            SimTurn::Assistant("I opened the incident log, assessed impact, assigned owners, and will seek approval before any production-changing mitigation.".into()),
        ])))
        .tool(record_incident_update())
        .build()?;
    Ok(Engine::new()
        .create(agent)
        .send_and_wait("API errors increased after a deployment.")
        .await?)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("model profile: {PRODUCTION_MODEL}");
    let turn = run().await?;
    println!("tool calls: {}", turn.tool_calls);
    println!("{}", turn.response);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::run;

    #[tokio::test]
    async fn runs_without_credentials() {
        let turn = run().await.unwrap();
        assert_eq!(turn.tool_calls, 1);
        assert!(turn.response.contains("incident log"));
    }
}

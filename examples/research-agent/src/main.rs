//! A self-contained technical-research agent.
//!
//! Run with `cargo run -p everruns-research-agent`.

use everruns::{Agent, Engine, LlmSimConfig, Model, Turn};
use everruns_llmsim::{SimToolCall, SimTurn};
use serde_json::json;

const PRODUCTION_MODEL: &str = "z-ai/glm-5.2";

#[everruns::tool]
/// Record a primary source in the current research evidence log.
async fn record_source(url: String) -> Result<String, String> {
    Ok(format!("recorded source {url}"))
}

async fn run() -> Result<Turn, Box<dyn std::error::Error>> {
    let agent = Agent::builder()
        .name("research-agent")
        .instructions("You are a research agent. Plan first, use primary sources, retain evidence, and cite consequential claims.")
        .model(Model::simulated_with_config(LlmSimConfig::scripted(vec![
            SimTurn::ToolCalls(vec![SimToolCall {
                name: "record_source".into(),
                arguments: json!({"url": "https://docs.everruns.com/framework/session-history/"}),
                id: Some("record_source_1".into()),
            }]),
            SimTurn::Assistant("I recorded a primary source, would keep source notes, distinguish facts from inferences, and cite the final report.".into()),
        ])))
        .tool(record_source())
        .build()?;
    Ok(Engine::new()
        .create(agent)
        .send_and_wait("Research the tradeoffs of durable agent sessions.")
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
        assert!(turn.response.contains("primary source"));
    }
}

//! A self-contained Everruns Framework support agent.
//!
//! Run with `cargo run -p everruns-framework-support-agent`.

use everruns::{Agent, Engine, LlmSimConfig, Model, Turn};
use everruns_llmsim::{SimToolCall, SimTurn};
use serde_json::json;

const PRODUCTION_MODEL: &str = "claude-opus-5";

#[everruns::tool]
/// Find the authoritative Everruns documentation relevant to a support question.
async fn search_docs(topic: String) -> Result<String, String> {
    Ok(format!("documentation search queued for {topic}"))
}

async fn run() -> Result<Turn, Box<dyn std::error::Error>> {
    let agent = Agent::builder()
        .name("everruns-support-agent")
        .instructions("You support Everruns users. Separate evidence from hypotheses and give the smallest safe next step.")
        .model(Model::simulated_with_config(LlmSimConfig::scripted(vec![
            SimTurn::ToolCalls(vec![SimToolCall {
                name: "search_docs".into(),
                arguments: json!({"topic": "provider configuration"}),
                id: Some("search_docs_1".into()),
            }]),
            SimTurn::Assistant("Check the provider credential, selected model, and the returned error before changing the agent configuration.".into()),
        ])))
        .tool(search_docs())
        .build()?;
    Ok(Engine::new()
        .create(agent)
        .send_and_wait("My Framework session fails after I add a provider. What should I check?")
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
        assert!(turn.response.contains("provider credential"));
    }
}

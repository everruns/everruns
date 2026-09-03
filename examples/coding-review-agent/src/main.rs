//! A self-contained code-review agent.
//!
//! Run with `cargo run -p everruns-coding-review-agent`.

use everruns::{Agent, Engine, LlmSimConfig, Model, Turn};
use everruns_llmsim::{SimToolCall, SimTurn};
use serde_json::json;

const PRODUCTION_MODEL: &str = "claude-sonnet-5";

#[everruns::tool]
/// Inspect a path in the application's trusted code-review workspace.
async fn inspect_change(path: String) -> Result<String, String> {
    Ok(format!("reviewed trusted workspace path {path}"))
}

async fn run() -> Result<Turn, Box<dyn std::error::Error>> {
    let agent = Agent::builder()
        .name("coding-review-agent")
        .instructions("You are a code reviewer. Report only material, reproducible findings and name the validation performed.")
        .model(Model::simulated_with_config(LlmSimConfig::scripted(vec![
            SimTurn::ToolCalls(vec![SimToolCall {
                name: "inspect_change".into(),
                arguments: json!({"path": "src/lib.rs"}),
                id: Some("inspect_change_1".into()),
            }]),
            SimTurn::Assistant("Findings: no issue in this simulated review. Validation: inspected the changed path, callers, and focused tests.".into()),
        ])))
        .tool(inspect_change())
        .build()?;
    Ok(Engine::new()
        .create(agent)
        .send_and_wait("Review this change before merge.")
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
        assert!(turn.response.contains("Findings"));
    }
}

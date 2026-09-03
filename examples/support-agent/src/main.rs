//! A self-contained customer-support agent.
//!
//! Run with `cargo run -p everruns-support-agent`.

use everruns::{Agent, Engine, LlmSimConfig, Model, Turn};
use everruns_llmsim::{SimToolCall, SimTurn};
use serde_json::json;

const PRODUCTION_MODEL: &str = "gpt-5.6-terra";

#[everruns::tool]
/// Look up safe, public support state without exposing customer details.
async fn lookup_customer(customer_id: String) -> Result<String, String> {
    Ok(format!("{customer_id}: account verified"))
}

async fn run() -> Result<Turn, Box<dyn std::error::Error>> {
    let agent = Agent::builder()
        .name("support-agent")
        .instructions("You are a support agent. Verify facts, protect customer data, and state the next action.")
        .model(Model::simulated_with_config(LlmSimConfig::scripted(vec![
            SimTurn::ToolCalls(vec![SimToolCall {
                name: "lookup_customer".into(),
                arguments: json!({"customer_id": "cust_demo"}),
                id: Some("lookup_customer_1".into()),
            }]),
            SimTurn::Assistant("I verified the account and created an access-recovery ticket; I will not expose account details in chat.".into()),
        ])))
        .tool(lookup_customer())
        .build()?;
    Ok(Engine::new()
        .create(agent)
        .send_and_wait("A customer cannot sign in after resetting their password.")
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
        assert!(turn.response.contains("access-recovery"));
    }
}

//! Named agent delegation with distinct behavior/models, through the real Engine.
//! cargo run -p everruns --features openai,local --example named_members
use everruns::{Agent, Engine, Model, OpenAI, Provider};
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let provider = Provider::from(OpenAI::from_env()?);
    let reviewer = Agent::builder()
        .name("reviewer")
        .description("Reviews a proposed answer for missing details and mistakes.")
        .instructions(
            "Review the proposed technical explanation. Return concise, actionable corrections.",
        )
        .model(Model::new("gpt-6-luna", provider.clone()))
        .build()?;
    let lead=Agent::builder().name("lead").instructions("Draft a short technical explanation, delegate a critique to reviewer with spawn_agent in foreground mode, then use the critique to produce the final answer. The reviewer is a separate agent with its own model.").model(Model::new("gpt-6.2-sol",provider)).member(reviewer).max_iterations(8).build()?;
    let session = Engine::new().create(lead);
    let turn = session
        .run("Explain Rust ownership and borrowing with one small example.")
        .await?;
    if !turn.success {
        return Err(format!("Lead failed: {:?}", turn.error).into());
    }
    println!("{}", turn.response);
    Ok(())
}

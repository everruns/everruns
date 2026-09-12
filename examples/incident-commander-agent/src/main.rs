//! Run from the repository checkout; see README.md for credentials and scenarios.
use everruns_example_demo as demo;
mod tools;

use everruns::{Agent, Engine};

const MODEL: &str = "muse-spark-1.3";
const QUESTION: &str = "Investigate the checkout error-rate alert using the bundled incident evidence. Record a concise update with the likely cause, uncertainty, and safe next action.";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let api_key = std::env::var("MODEL_API_KEY").or_else(|_| std::env::var("META_API_KEY"))?;
    let input = std::env::args().skip(1).collect::<Vec<_>>().join(" ");
    let question = if input.is_empty() { QUESTION } else { &input };
    let agent = Agent::builder()
        .name("incident-commander-agent")
        .instructions(include_str!("instructions.md"))
        .provider(everruns_meta::provider("meta", api_key))
        .model(MODEL)
        .max_iterations(12)
        .tool(tools::inspect_evidence())
        .tool(tools::record_incident_update())
        .build()?;

    let engine = Engine::new();
    let session = engine.create(agent);
    println!("MODEL: {MODEL}");
    demo::run(&session, question).await?;
    Ok(())
}

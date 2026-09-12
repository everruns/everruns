//! Run from the repository checkout; see README.md for credentials and scenarios.
use everruns_example_demo as demo;
mod tools;

use everruns::{Agent, Engine};

const MODEL: &str = "claude-sonnet-5";
const QUESTION: &str = "Review sample_payment.rs against its refund contract. Run the bundled regression test and report only the reproduced defect.";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let api_key = std::env::var("ANTHROPIC_API_KEY")?;
    let input = std::env::args().skip(1).collect::<Vec<_>>().join(" ");
    let question = if input.is_empty() { QUESTION } else { &input };
    let agent = Agent::builder()
        .name("coding-review-agent")
        .instructions(include_str!("instructions.md"))
        .provider(everruns_anthropic::provider("anthropic", api_key))
        .model(MODEL)
        .max_iterations(12)
        .tool(tools::inspect_change())
        .tool(tools::run_regression())
        .build()?;

    let engine = Engine::new();
    let session = engine.create(agent);
    println!("MODEL: {MODEL}");
    demo::run(&session, question).await?;
    Ok(())
}

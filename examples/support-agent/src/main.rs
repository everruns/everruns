//! Run from the repository checkout; see README.md for credentials and scenarios.
use everruns_example_demo as demo;
mod tools;

use everruns::OpenAI;
use everruns::{Agent, Engine};

const MODEL: &str = "gpt-5.6-terra";
const QUESTION: &str = "Customer cust_mfa reset their password but still cannot sign in. Diagnose the next safe step; do not ask for secrets.";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let api_key = std::env::var("OPENAI_API_KEY")?;
    let input = std::env::args().skip(1).collect::<Vec<_>>().join(" ");
    let question = if input.is_empty() { QUESTION } else { &input };
    let agent = Agent::builder()
        .name("support-agent")
        .instructions(include_str!("instructions.md"))
        .provider(OpenAI::new(api_key))
        .model(MODEL)
        .max_iterations(12)
        .tool(tools::lookup_customer())
        .tool(tools::read_support_policy())
        .build()?;

    let engine = Engine::new();
    let session = engine.create(agent);
    println!("MODEL: {MODEL}");
    demo::run(&session, question).await?;
    Ok(())
}

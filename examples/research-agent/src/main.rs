//! Run from the repository checkout; see README.md for credentials and scenarios.
use everruns_example_demo as demo;

use everruns::{Agent, Engine};
use everruns_integrations_brave_search::BraveSearch;

const MODEL: &str = "z-ai/glm-5.2";
const QUESTION: &str = "Can durable execution prevent duplicate payments? Search and read two official primary sources. Answer in three short bullets: the guarantee, the failure window, and the mitigation. Cite the pages you read. No introduction; at most 100 words.";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let api_key = std::env::var("OPENROUTER_API_KEY")?;
    let input = std::env::args().skip(1).collect::<Vec<_>>().join(" ");
    let question = if input.is_empty() { QUESTION } else { &input };
    let agent = Agent::builder()
        .name("research-agent")
        .instructions(include_str!("instructions.md"))
        .provider(everruns_openrouter::provider("openrouter", api_key))
        .model(MODEL)
        .max_iterations(12)
        .capability(BraveSearch::from_env()?)
        .capability(everruns::WebFetch::new())
        .build()?;

    let engine = Engine::new();
    let session = engine.create(agent);
    println!("MODEL: {MODEL}");
    demo::run(&session, question).await?;
    Ok(())
}

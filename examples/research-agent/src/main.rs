//! A real web-research agent backed by OpenRouter and GLM.
//!
//! ```text
//! OPENROUTER_API_KEY=... cargo run -p everruns-research-agent
//! ```

use everruns::{Agent, Engine, Turn};
use everruns_integrations_brave_search::BraveSearch;

const MODEL: &str = "z-ai/glm-5.2";
const DEFAULT_QUESTION: &str = "Research durable agent sessions. Use web search before answering, cite the returned source URLs, and distinguish facts from inferences.";

fn build_agent(api_key: &str, search: BraveSearch) -> Result<Agent, everruns::BuildError> {
    Agent::builder()
        .name("research-agent")
        .instructions("You are a research agent. Use brave_web_search before answering factual questions. Cite the returned source URLs, distinguish facts from inferences, and say when the evidence is incomplete.")
        .provider(everruns_openrouter::provider("openrouter", api_key))
        .model(MODEL)
        .capability(search)
        .build()
}

async fn run(question: &str) -> Result<Turn, Box<dyn std::error::Error>> {
    let api_key = std::env::var("OPENROUTER_API_KEY")?;
    let agent = build_agent(&api_key, BraveSearch::from_env()?)?;
    let engine = Engine::new();
    let session = engine.create(agent);
    Ok(session.send_and_wait(question).await?)
}

fn question_from_args() -> String {
    let question = std::env::args().skip(1).collect::<Vec<_>>().join(" ");
    if question.is_empty() {
        DEFAULT_QUESTION.into()
    } else {
        question
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let question = question_from_args();
    let turn = run(&question).await?;
    println!("model: {MODEL}");
    println!("response: {}", turn.response);
    println!(
        "iterations: {}, tool calls: {}",
        turn.iterations, turn.tool_calls
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::build_agent;

    #[test]
    fn builds_without_contacting_openrouter() {
        assert!(build_agent("test-key", super::BraveSearch::new("test-search-key")).is_ok());
    }
}

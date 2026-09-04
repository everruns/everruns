//! A real web-research agent backed by OpenRouter and GLM.
//!
//! ```text
//! OPENROUTER_API_KEY=... cargo run -p everruns-research-agent
//! ```

use everruns::{Agent, Engine, Turn};
use serde::Deserialize;

const MODEL: &str = "z-ai/glm-5.2";
const DEFAULT_QUESTION: &str = "Research durable agent sessions. Use web search before answering, cite the returned source URLs, and distinguish facts from inferences.";

#[derive(Deserialize)]
struct SearchResponse {
    web: Option<SearchWeb>,
}

#[derive(Deserialize)]
struct SearchWeb {
    results: Vec<SearchResult>,
}

#[derive(Deserialize)]
struct SearchResult {
    title: String,
    url: String,
    description: String,
}

#[everruns::tool]
/// Search the public web and return up to five titled, citable results.
async fn search_web(query: String) -> Result<String, String> {
    let api_key = std::env::var("BRAVE_SEARCH_API_KEY")
        .map_err(|_| "BRAVE_SEARCH_API_KEY must be set to run web research".to_string())?;
    let url = reqwest::Url::parse_with_params(
        "https://api.search.brave.com/res/v1/web/search",
        [("q", query.as_str()), ("count", "5")],
    )
    .map_err(|error| format!("Could not build Brave Search URL: {error}"))?;
    let response = reqwest::Client::new()
        .get(url)
        .header("X-Subscription-Token", api_key)
        .send()
        .await
        .map_err(|error| format!("Brave Search request failed: {error}"))?
        .error_for_status()
        .map_err(|error| format!("Brave Search returned an error: {error}"))?
        .json::<SearchResponse>()
        .await
        .map_err(|error| format!("Brave Search response was invalid: {error}"))?;

    let results = response
        .web
        .map(|web| web.results)
        .unwrap_or_default()
        .into_iter()
        .take(5)
        .map(|result| {
            serde_json::json!({
                "title": result.title,
                "url": result.url,
                "description": result.description,
            })
        })
        .collect::<Vec<_>>();
    serde_json::to_string_pretty(&results)
        .map_err(|error| format!("Could not serialize search results: {error}"))
}

fn build_agent(api_key: &str) -> Result<Agent, everruns::BuildError> {
    Agent::builder()
        .name("research-agent")
        .instructions("You are a research agent. Use search_web before answering factual questions. Cite the returned source URLs, distinguish facts from inferences, and say when the evidence is incomplete.")
        .provider(everruns_openrouter::provider("openrouter", api_key))
        .model(MODEL)
        .tool(search_web())
        .build()
}

async fn run(question: &str) -> Result<Turn, Box<dyn std::error::Error>> {
    let api_key = std::env::var("OPENROUTER_API_KEY")?;
    let agent = build_agent(&api_key)?;
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
        assert!(build_agent("test-key").is_ok());
    }
}

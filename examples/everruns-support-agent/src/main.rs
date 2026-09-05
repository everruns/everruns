//! A real Everruns Framework support agent backed by Anthropic.
//!
//! ```text
//! ANTHROPIC_API_KEY=... cargo run -p everruns-framework-support-agent
//! ```

mod demo;

use everruns::{Agent, Engine, Turn};

const MODEL: &str = "claude-opus-5";
const DEFAULT_QUESTION: &str =
    "My Framework session fails after I add an Anthropic provider. What should I check first?";

#[everruns::tool]
/// Read authoritative Framework documentation for a support topic.
async fn search_docs(topic: String) -> Result<String, String> {
    let topic = topic.to_lowercase();
    let page = if topic.contains("custom") {
        "custom-providers"
    } else if topic.contains("provider") || topic.contains("model") {
        "models-and-providers"
    } else {
        "examples"
    };
    let url = format!(
        "https://raw.githubusercontent.com/everruns/everruns/main/docs/framework/{page}.md"
    );
    let body = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?
        .get(url)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .text()
        .await
        .map_err(|e| e.to_string())?;
    let content = body
        .strip_prefix("---\n")
        .and_then(|text| text.split_once("\n---\n"))
        .map_or(body.as_str(), |(_, content)| content.trim());
    Ok(format!(
        "Source: https://docs.everruns.com/framework/{page}/\n{}",
        content.chars().take(16000).collect::<String>()
    ))
}

fn build_agent(api_key: &str) -> Result<Agent, everruns::BuildError> {
    Agent::builder()
        .name("everruns-support-agent")
        .instructions("Keep the final answer within 150 words. You support Everruns Framework users. Use search_docs before answering. Separate evidence from hypotheses and give the smallest safe next step with relevant documentation links.")
        .provider(everruns_anthropic::provider("anthropic", api_key))
        .model(MODEL)
        .tool(search_docs())
        .build()
}

async fn run(question: &str) -> Result<Turn, Box<dyn std::error::Error>> {
    let api_key = std::env::var("ANTHROPIC_API_KEY")?;
    let agent = build_agent(&api_key)?;
    let engine = Engine::new();
    let session = engine.create(agent);
    demo::run(&session, question).await
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
    println!("Model: {MODEL}");
    run(&question).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::build_agent;

    #[test]
    fn builds_without_contacting_anthropic() {
        assert!(build_agent("test-key").is_ok());
    }
}

//! A real Everruns Framework support agent backed by Anthropic.
//!
//! ```text
//! ANTHROPIC_API_KEY=... cargo run -p everruns-framework-support-agent
//! ```

use everruns::{Agent, Engine, Turn};

const MODEL: &str = "claude-opus-5";
const DEFAULT_QUESTION: &str =
    "My Framework session fails after I add an Anthropic provider. What should I check first?";

#[everruns::tool]
/// Return authoritative Everruns documentation links for a support topic.
async fn search_docs(topic: String) -> Result<String, String> {
    let topic = topic.to_lowercase();
    if topic.contains("provider") || topic.contains("model") {
        Ok("Models and providers: https://docs.everruns.com/framework/models-and-providers/\nCustom providers: https://docs.everruns.com/framework/custom-providers/".into())
    } else {
        Ok("Framework overview: https://docs.everruns.com/framework/\nFramework examples: https://docs.everruns.com/framework/examples/".into())
    }
}

fn build_agent(api_key: &str) -> Result<Agent, everruns::BuildError> {
    Agent::builder()
        .name("everruns-support-agent")
        .instructions("You support Everruns Framework users. Use search_docs before answering. Separate evidence from hypotheses and give the smallest safe next step with relevant documentation links.")
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
    fn builds_without_contacting_anthropic() {
        assert!(build_agent("test-key").is_ok());
    }
}

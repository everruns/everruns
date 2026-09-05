//! A real code-review agent backed by Anthropic.
//!
//! ```text
//! ANTHROPIC_API_KEY=... cargo run -p everruns-coding-review-agent
//! ```

mod demo;

use everruns::{Agent, Engine, Turn};

const MODEL: &str = "claude-sonnet-5";
const DEFAULT_QUESTION: &str =
    "Review the included sample_payment.rs. Report only material, reproducible findings.";

#[everruns::tool]
/// Read the self-contained code change that this example asks the agent to review.
async fn inspect_change(path: String) -> Result<String, String> {
    if path != "sample_payment.rs" {
        return Err("This self-contained example exposes only sample_payment.rs.".into());
    }
    Ok(include_str!("../sample_payment.rs").into())
}

fn build_agent(api_key: &str) -> Result<Agent, everruns::BuildError> {
    Agent::builder()
        .name("coding-review-agent")
        .instructions("Keep the final answer within 150 words. You are a code reviewer. Use inspect_change before reviewing the sample. Report only material, reproducible findings. For each finding, explain impact, point to the relevant code, and name a focused validation.")
        .provider(everruns_anthropic::provider("anthropic", api_key))
        .model(MODEL)
        .tool(inspect_change())
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
    println!("\nCODE TO REVIEW\n{}", include_str!("../sample_payment.rs"));
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

//! A real customer-support agent backed by OpenAI.
//!
//! ```text
//! OPENAI_API_KEY=... cargo run -p everruns-support-agent
//! ```

mod demo;

use everruns::{Agent, Engine, OpenAI, Turn};

const MODEL: &str = "gpt-5.6-terra";
const DEFAULT_QUESTION: &str =
    "cust_demo cannot sign in after resetting their password. What should they do next?";

#[everruns::tool]
/// Look up the safe, public support state for a demo customer.
async fn lookup_customer(customer_id: String) -> Result<String, String> {
    match customer_id.as_str() {
        "cust_demo" => Ok("cust_demo: verified account; password reset completed; no active lockout; next safe action is to retry in a private browser window.".into()),
        _ => Err("Only the self-contained cust_demo record is available in this example.".into()),
    }
}

fn build_agent(api_key: &str) -> Result<Agent, everruns::BuildError> {
    Agent::builder()
        .name("support-agent")
        .instructions("Keep the final answer within 150 words. You are a customer-support agent. Use lookup_customer before answering account questions. Do not expose private data. Give a concise answer and a clear next action.")
        .provider(OpenAI::new(api_key))
        .model(MODEL)
        .tool(lookup_customer())
        .build()
}

async fn run(question: &str) -> Result<Turn, Box<dyn std::error::Error>> {
    let api_key = std::env::var("OPENAI_API_KEY")?;
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
    fn builds_without_contacting_openai() {
        assert!(build_agent("test-key").is_ok());
    }
}

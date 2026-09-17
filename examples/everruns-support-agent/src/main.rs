//! Run from the repository checkout; see README.md for credentials and scenarios.
use everruns_example_demo as demo;
mod agent;
mod tools;

use everruns::Engine;
use std::io::{self, Write};

const QUESTION: &str = "How do I resume a durable Framework session after restarting my process? Find and read relevant documentation before answering.";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let input = std::env::args().skip(1).collect::<Vec<_>>().join(" ");
    let question = match input.as_str() {
        "" => QUESTION.to_owned(),
        "--interactive" => read_question()?,
        _ => input,
    };

    // ANTHROPIC_API_KEY, declared by the Anthropic driver itself.
    let agent = agent::build(everruns_anthropic::from_env("anthropic")?)?;
    let engine = Engine::new();
    let session = engine.create(agent);

    println!("MODEL: {}", agent::MODEL);
    demo::run(&session, &question).await?;
    Ok(())
}

fn read_question() -> io::Result<String> {
    print!("Question: ");
    io::stdout().flush()?;

    let mut question = String::new();
    io::stdin().read_line(&mut question)?;
    validate_question(&question)
}

fn validate_question(question: &str) -> io::Result<String> {
    let question = question.trim();
    if question.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "question cannot be empty",
        ));
    }
    Ok(question.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interactive_question_is_trimmed() {
        assert_eq!(
            validate_question("  How do I resume a session?\n").unwrap(),
            "How do I resume a session?"
        );
    }

    #[test]
    fn interactive_question_cannot_be_empty() {
        assert_eq!(
            validate_question(" \n").unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
    }
}

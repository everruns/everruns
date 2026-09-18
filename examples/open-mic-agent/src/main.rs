#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Run from the repository checkout; see README.md for credentials and scenarios.
use everruns_example_demo as demo;
mod agent;
mod tools;

use everruns::Engine;
use std::io::{self, Write};

const QUESTION: &str =
    "Book submission mic_pun into tonight's line-up. Measure the bit before you decide.";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let input = std::env::args().skip(1).collect::<Vec<_>>().join(" ");
    let question = match input.as_str() {
        "" => QUESTION.to_owned(),
        "--interactive" => read_question()?,
        _ => input,
    };

    // ANTHROPIC_API_KEY for the agent, TYPESAFE_API_KEY for the classifier it
    // measures with; each is read by the driver that needs it.
    let agent = agent::build(
        everruns_anthropic::from_env("anthropic")?,
        everruns::Jev::from_env()?,
    )?;
    let engine = Engine::new();
    let session = engine.create(agent);

    println!("MODEL: {}", agent::MODEL);
    demo::run(&session, &question).await?;
    Ok(())
}

fn read_question() -> io::Result<String> {
    print!("Submission: ");
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
            validate_question("  Book mic_late.\n").unwrap(),
            "Book mic_late."
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

//! Run from the repository checkout; see README.md for credentials and scenarios.
use everruns_example_demo as demo;
mod agent;
mod tools;

use everruns::Engine;
use std::io::{self, Write};

const QUESTION: &str = "Customer cust_mfa reset their password but still cannot sign in. Diagnose the next safe step; do not ask for secrets.";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let api_key = std::env::var("OPENAI_API_KEY")?;
    let input = std::env::args().skip(1).collect::<Vec<_>>().join(" ");
    let question = match input.as_str() {
        "" => QUESTION.to_owned(),
        "--interactive" => read_question()?,
        _ => input,
    };

    let agent = agent::build(api_key)?;
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
            validate_question("  Help me sign in.\n").unwrap(),
            "Help me sign in."
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

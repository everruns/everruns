//! A real incident-commander agent backed by Meta Model API.
//!
//! ```text
//! MODEL_API_KEY=... cargo run -p everruns-incident-commander-agent
//! ```

mod demo;

use everruns::{Agent, Engine, Turn};

const MODEL: &str = "muse-spark-1.3";
const DEFAULT_QUESTION: &str = "API errors increased after a deployment. Start an incident update and propose safe next actions.";

#[everruns::tool]
/// Record a bounded, non-sensitive incident status update.
async fn record_incident_update(update: String) -> Result<String, String> {
    if update.len() > 500 {
        return Err("Keep incident updates under 500 characters.".into());
    }
    append_update(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("incident.log"),
        &update,
    )
}

fn append_update(path: &std::path::Path, update: &str) -> Result<String, String> {
    use std::io::Write;
    let mut log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    writeln!(log, "{}", update.replace(['\n', '\r'], " ")).map_err(|e| e.to_string())?;
    Ok(format!("Appended to incident.log: {update}"))
}

fn build_agent(api_key: &str) -> Result<Agent, everruns::BuildError> {
    Agent::builder()
        .name("incident-commander-agent")
        .instructions("Keep the final answer within 150 words. You are an incident commander. Record a concise incident update before answering. State known impact, unknowns, owners, and safe next actions. Never claim that a production change occurred unless the tool result says so.")
        .provider(everruns_meta::provider("meta", api_key))
        .model(MODEL)
        .tool(record_incident_update())
        .build()
}

async fn run(question: &str) -> Result<Turn, Box<dyn std::error::Error>> {
    let api_key = std::env::var("MODEL_API_KEY").or_else(|_| std::env::var("META_API_KEY"))?;
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
    fn incident_log_retains_multiple_updates() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("incident.log");
        super::append_update(&path, "Investigating\nerrors").unwrap();
        super::append_update(&path, "Rollback\rproposed").unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "Investigating errors\nRollback proposed\n"
        );
    }

    #[test]
    fn builds_without_contacting_meta() {
        assert!(build_agent("test-key").is_ok());
    }
}

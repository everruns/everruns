//! Persist, page, resume, and continue a Framework session entirely offline.

use std::path::{Path, PathBuf};

use everruns::{Agent, Engine, LocalConfig, Model};

fn agent(data_dir: &Path) -> Result<Agent, everruns::BuildError> {
    Agent::builder()
        .instructions("Keep the conversation concise.")
        .model(Model::simulated("Acknowledged."))
        .local(LocalConfig::new(data_dir))
        .build()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let data_dir = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/framework-session-history-example"));

    let session_id = {
        let engine = Engine::new();
        let first_agent = agent(&data_dir)?;
        let session = engine.create(first_agent);
        session.send_and_wait("My project is Atlas.").await?;
        session.send_and_wait("Remember that name.").await?;
        session.session_id()
    };

    // A newly built Agent restores behavior from trusted application
    // configuration; the engine attaches it to identity/history from the same
    // local profile. Application code never opens an event log or database.
    let second_agent = agent(&data_dir)?;
    let engine = Engine::new();
    engine.attach(session_id, second_agent).await?;
    let resumed = engine.resume(session_id).await?;
    let mut pages = resumed.history().limit(2)?.pages();
    while let Some(page) = pages.next_page().await? {
        for message in page.messages {
            println!("{}: {}", message.role, message.text());
        }
    }

    resumed.send_and_wait("Continue the session.").await?;
    println!("resumed session {session_id}");
    println!("local state: {}", data_dir.display());
    Ok(())
}

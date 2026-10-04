//! Run a Markdown agent or an asset folder without credentials.
use everruns::{AgentPackage, Engine, Model};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).unwrap_or_else(|| {
        format!(
            "{}/../../examples/agent-packages/triage",
            env!("CARGO_MANIFEST_DIR")
        )
    });
    let package = AgentPackage::load(path)?;
    let agent = package
        .builder()?
        .model(Model::simulated(
            "The package is loaded and ready to triage.",
        ))
        .build()?;
    let session = package.create(&Engine::new(), agent)?;
    let result = session.send_and_wait("Help me triage an issue.").await?;
    assert!(result.success);
    println!("{}", result.response);
    Ok(())
}

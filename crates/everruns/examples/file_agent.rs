//! Run a Markdown agent or an asset folder against a real model.
//!
//! ```text
//! cargo run -p everruns --features openai --example file_agent [-- PATH]
//! ```
use everruns::{AgentPackage, Engine, OpenAI};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).unwrap_or_else(|| {
        format!(
            "{}/../../examples/agents/triage",
            env!("CARGO_MANIFEST_DIR")
        )
    });
    let package = AgentPackage::load(path)?;
    let agent = package
        .builder()?
        .provider(OpenAI::from_env()?)
        .model("gpt-6.1-sol")
        .build()?;
    let session = package.create(&Engine::new(), agent)?;
    let result = session.send_and_wait("Help me triage an issue.").await?;
    assert!(result.success);
    println!("{}", result.response);
    Ok(())
}

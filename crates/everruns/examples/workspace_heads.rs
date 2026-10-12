//! Open an isolated local Git head and bind it to a durable Framework session.
//!
//! ```text
//! cargo run -p everruns --features local,openai --example workspace_heads -- /path/to/repo /path/to/state
//! ```

use std::path::PathBuf;
use std::sync::Arc;

use everruns::{Agent, Engine, LocalConfig, LocalGitWorkspace, OpenAI, Workspace, WorkspacePolicy};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let repository = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or(std::env::current_dir()?);
    let state = std::env::args_os()
        .nth(2)
        .map(PathBuf::from)
        .unwrap_or_else(|| repository.join(".everruns-local"));

    let backend = Arc::new(LocalGitWorkspace::new(state.join("workspace-provider"))?);
    let workspace = Workspace::open(backend.clone(), repository.to_string_lossy()).await?;
    let head = workspace.head("example").create().await?;
    let agent = Agent::builder()
        .instructions("Work only in the selected workspace head.")
        .provider(OpenAI::from_env()?)
        .model("gpt-6.1-sol")
        .local(LocalConfig::new(state.join("runtime")))
        .workspace_backend(backend)
        .workspace_policy(WorkspacePolicy::read_write())
        .build()?;
    let session = Engine::new()
        .create(agent.clone())
        .workspace(head.clone())
        .start()
        .await?;

    println!(
        "session={} workspace={} head={}",
        session.id(),
        workspace.id(),
        head.id()
    );
    Ok(())
}

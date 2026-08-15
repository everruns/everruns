//! Configure portable workspace access without importing runtime backends.

use everruns::{Agent, InMemoryEngine, Model, WorkspacePolicy};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let workspace = tempfile::tempdir()?;
    let policy = WorkspacePolicy::builder()
        .allow_read("input")
        .allow_write("generated")
        .deny_write("generated/locked")
        .build()?;

    assert!(policy.permits_read("input/brief.md"));
    assert!(policy.permits_write("generated/report.md"));
    assert!(!policy.permits_write("generated/locked/report.md"));
    assert!(!policy.permits_read(".env"));

    let agent = Agent::builder()
        .instructions("Use only files permitted by the workspace policy.")
        .model(Model::simulated("Workspace policy configured."))
        .workspace(workspace.path())
        .workspace_policy(policy)
        .readonly_file("input/brief.md", "Summarize the workspace policy.")
        .build()?;

    let turn = InMemoryEngine::new()
        .create(agent)
        .send_and_wait("Confirm the policy is active.")
        .await?;
    assert_eq!(turn.response, "Workspace policy configured.");
    assert_eq!(
        std::fs::read_to_string(workspace.path().join("input/brief.md"))?,
        "Summarize the workspace policy."
    );

    println!("{}", turn.response);
    Ok(())
}

//! Configure portable workspace access without importing runtime backends.
//!
//! ```text
//! cargo run -p everruns --features openai --example workspace_policy
//! ```

use everruns::{Agent, Engine, OpenAI, WorkspacePolicy};

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
        .instructions(
            "Use only files permitted by the workspace policy. Read input/brief.md and \
             follow it. Keep the answer to two sentences.",
        )
        .provider(OpenAI::from_env()?)
        .model("gpt-6.1-sol")
        .workspace(workspace.path())
        .workspace_policy(policy)
        .readonly_file("input/brief.md", "Summarize the workspace policy.")
        .build()?;

    let turn = Engine::new()
        .create(agent)
        .send_and_wait("Do what input/brief.md asks.")
        .await?;
    assert!(turn.success);
    assert!(!turn.response.trim().is_empty());
    assert_eq!(
        std::fs::read_to_string(workspace.path().join("input/brief.md"))?,
        "Summarize the workspace policy."
    );

    println!("{}", turn.response);
    println!("tool calls: {}", turn.tool_calls);
    Ok(())
}

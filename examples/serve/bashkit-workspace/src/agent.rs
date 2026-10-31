use serve::prelude::*;

/// Creates and inspects small reports in a session's Bashkit workspace.
#[agent]
#[expect(
    clippy::expect_used,
    reason = "the policy uses valid static path literals"
)]
fn assistant() -> Agent {
    Agent::builder()
        .model("openai/gpt-6-luna")
        .instructions(md!("instructions.md"))
        .customize(|builder| {
            // Workspaces default to read-only. Only this disposable subtree
            // needs writes; selecting the shell does not grant write access.
            builder.workspace_policy(
                everruns::WorkspacePolicy::builder()
                    .allow_read("/")
                    .allow_write("/workspace/demo")
                    .build()
                    .expect("valid demo workspace policy"),
            )
        })
        .build()
}

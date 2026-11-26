use serve::prelude::*;

/// Answers revenue questions from the warehouse.
#[agent(default)]
fn analyst() -> Agent {
    Agent::builder()
        .sandbox(SandboxKind::Bashkit)
        // Resolved by the host's model gateway (or `OPENAI_API_KEY` locally);
        // the app holds no provider keys.
        .model("openai/gpt-6.2-sol")
        .instructions(md!("instructions.md"))
        // Another agent of this app the analyst hands drafts to for review.
        .spawns("reviewer")
        .build()
}

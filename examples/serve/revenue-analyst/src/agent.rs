use serve::prelude::*;

/// Answers revenue questions from the warehouse.
#[agent(default)]
fn analyst() -> Agent {
    Agent::builder()
        // Resolved by the host's model gateway (or `OPENAI_API_KEY` locally);
        // the app holds no provider keys.
        .model("openai/gpt-6.1-sol")
        .instructions(md!("instructions.md"))
        // Another agent of this app the analyst hands drafts to for review.
        .member("reviewer")
        .build()
}

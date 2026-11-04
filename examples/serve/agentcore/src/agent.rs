use serve::prelude::*;

/// Answers order-status questions.
#[agent]
fn assistant() -> Agent {
    Agent::builder()
        // OPENAI_API_KEY, with optional OPENAI_BASE_URL for a compatible endpoint.
        .model("openai/gpt-6.1-sol")
        .instructions(md!("instructions.md"))
        .build()
}

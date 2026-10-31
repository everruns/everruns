use serve::prelude::*;

/// Rolls dice for board-game nights.
#[agent]
fn assistant() -> Agent {
    Agent::builder()
        // OPENAI_API_KEY, with optional OPENAI_BASE_URL for a compatible endpoint.
        // OPENROUTER_API_KEY is another supported provider route.
        .model("openai/gpt-6.1-sol")
        .instructions(md!("instructions.md"))
        .build()
}

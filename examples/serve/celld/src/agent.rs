use serve::prelude::*;

/// Researches a topic with a slow tool.
#[agent]
fn assistant() -> Agent {
    Agent::builder()
        // OPENAI_API_KEY, with optional OPENAI_BASE_URL for a compatible endpoint.
        .model("openai/gpt-6.2-sol")
        .instructions(md!("instructions.md"))
        .build()
}

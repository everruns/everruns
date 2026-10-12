use serve::prelude::*;

/// Researches a topic with a slow tool.
#[agent]
fn assistant() -> Agent {
    Agent::builder()
        // Resolved by the model gateway: SERVE_GATEWAY_URL on a host, or
        // `OPENAI_API_KEY` locally.
        .model("openai/gpt-6.1-sol")
        .instructions(md!("instructions.md"))
        .build()
}

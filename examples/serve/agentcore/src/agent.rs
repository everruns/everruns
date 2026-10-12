use serve::prelude::*;

/// Answers order-status questions.
#[agent]
fn assistant() -> Agent {
    Agent::builder()
        // Resolved by the model gateway: SERVE_GATEWAY_URL on AgentCore (for
        // example an AgentCore Gateway inference endpoint), or
        // `OPENAI_API_KEY` locally.
        .model("openai/gpt-6.1-sol")
        .instructions(md!("instructions.md"))
        .build()
}

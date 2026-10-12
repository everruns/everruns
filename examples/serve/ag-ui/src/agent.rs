use serve::prelude::*;

/// Deploys builds, with a person approving each deployment.
#[agent]
fn assistant() -> Agent {
    Agent::builder()
        // `OPENAI_API_KEY` routes `openai/…` to OpenAI; a host's
        // SERVE_GATEWAY_URL or OPENROUTER_API_KEY takes precedence.
        .model("openai/gpt-6.1-sol")
        .instructions(md!("instructions.md"))
        .build()
}

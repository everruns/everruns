use serve::prelude::*;

/// The front desk of a small hotel: room availability and opening hours.
#[agent]
fn concierge() -> Agent {
    Agent::builder()
        // Any chat model works: voice only changes how the caller is heard
        // and answered. `OPENAI_API_KEY` routes it to OpenAI.
        .model("openai/gpt-6.1-sol")
        .instructions(md!("instructions.md"))
        .build()
}

use serve::prelude::*;
use serve::sim;

/// The front desk of a small hotel: room availability and opening hours.
#[agent]
fn concierge() -> Agent {
    Agent::builder()
        // Any chat model works: voice only changes how the caller is heard
        // and answered. Without OPENAI_API_KEY, dev and evals use the
        // offline script below.
        .model("openai/gpt-5.6-terra")
        .instructions(md!("instructions.md"))
        .offline(sim::script([
            sim::call("rooms", json!({ "night": "Friday" })),
            sim::reply("We have two rooms free on Friday, a double and a suite."),
        ]))
        .build()
}

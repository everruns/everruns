use serve::prelude::*;
use serve::sim;

/// Rolls dice for board-game nights.
#[agent]
fn assistant() -> Agent {
    Agent::builder()
        // Resolved by the model gateway. Without one (no SERVE_GATEWAY_URL or
        // OPENROUTER_API_KEY), dev and evals use the offline script below.
        .model("anthropic/claude-sonnet-5")
        .instructions(md!("instructions.md"))
        .offline(sim::script([
            sim::call("roll_dice", json!({ "sides": 6 })),
            sim::reply("I rolled a six-sided die for you. (Offline demo reply: the roll is in the tool result.)"),
        ]))
        .build()
}

use serve::prelude::*;
use serve::sim;

/// Researches a topic with a slow tool.
#[agent]
fn assistant() -> Agent {
    Agent::builder()
        // Resolved by the model gateway (SERVE_GATEWAY_URL). Without one,
        // `--dev` uses the offline script below.
        .model("anthropic/claude-sonnet-5")
        .instructions(md!("instructions.md"))
        .offline(sim::script([
            sim::call("look_up", json!({ "topic": "celld" })),
            sim::reply("celld runs Durable Objects on your own machines. (Offline demo reply.)"),
        ]))
        .build()
}

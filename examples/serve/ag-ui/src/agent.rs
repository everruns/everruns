use serve::prelude::*;
use serve::sim;

/// Deploys builds, with a person approving each deployment.
#[agent]
fn assistant() -> Agent {
    Agent::builder()
        // Resolved by the model gateway. Without one (no SERVE_GATEWAY_URL or
        // OPENROUTER_API_KEY), dev and evals use the offline script below.
        .model("anthropic/claude-sonnet-5")
        .instructions(md!("instructions.md"))
        .offline(sim::script([
            sim::call("deploy", json!({ "environment": "staging" })),
            sim::reply(
                "Deployed to staging. (Offline demo reply: the deployment is in the tool result.)",
            ),
        ]))
        .build()
}

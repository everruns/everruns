use serve::prelude::*;
use serve::sim;

/// Answers order-status questions.
#[agent]
fn assistant() -> Agent {
    Agent::builder()
        // Resolved by the model gateway (SERVE_GATEWAY_URL, for example an
        // AgentCore Gateway inference endpoint). Without one, `--dev` and
        // evals use the offline script below.
        .model("anthropic/claude-sonnet-5")
        .instructions(md!("instructions.md"))
        .offline(sim::script([
            sim::call("order_status", json!({ "order_id": "A-1001" })),
            sim::reply(
                "Order A-1001 has shipped. (Offline demo reply: the status is in the tool result.)",
            ),
        ]))
        .build()
}

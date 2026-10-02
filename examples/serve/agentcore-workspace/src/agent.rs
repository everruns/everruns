use serve::prelude::*;
use serve::sim;

/// Works in a private, persistent workspace: shell, files, and a gated way
/// to share results.
#[agent]
fn assistant() -> Agent {
    Agent::builder()
        // Bedrock through the runtime's execution role when AWS_REGION is set
        // (as on AgentCore). Without it, `--dev` and evals follow the
        // offline script below.
        .model("bedrock/us.anthropic.claude-sonnet-4-6")
        .instructions(md!("instructions.md"))
        .offline(sim::script([
            sim::call(
                "bash",
                json!({ "command": "mkdir -p notes && echo '- check the build' >> notes/todo.md && cat notes/todo.md" }),
            ),
            sim::call(
                "share_report",
                json!({ "title": "Todo", "body": "- check the build" }),
            ),
            sim::reply("Added a todo to notes/todo.md and shared it. (Offline demo reply.)"),
        ]))
        .build()
}

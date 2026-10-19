use serve::prelude::*;

/// Works in a private, persistent workspace: shell, files, and a gated way
/// to share results.
#[agent]
fn assistant() -> Agent {
    Agent::builder()
        // Bedrock through the runtime's execution role when AWS_REGION is set
        // (as on AgentCore); locally, the AWS default credential chain.
        .model("bedrock/us.anthropic.claude-sonnet-4-6")
        .instructions(md!("instructions.md"))
        .build()
}

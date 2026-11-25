//! Spawn agents: one-off calls to other agents, through the real Engine.
//!
//! The lead drafts an answer, spawns two copies of itself in parallel to
//! check one fact each, then spawns `reviewer` (a separate agent on a lighter
//! model) and waits for its critique before answering. Every spawned agent
//! gets a brief, works in its own session, and returns a result; none of them
//! is followed afterwards. For followed work, see the `coordinator-agent`
//! example (Threads).
//!
//! ```text
//! OPENAI_API_KEY=... cargo run -p everruns --features openai,local --example spawn_agents
//! ```

use everruns::{Agent, Engine, Model, OpenAI, Provider, SpawnAgents};

const LEAD_INSTRUCTIONS: &str = "\
Answer the question in under 200 words. First spawn two copies of yourself \
(agent \"self\") at once, each checking one claim you plan to make, and wait \
for both. Then spawn reviewer with your draft and wait for its critique. Use \
the critique in your final answer.";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let provider = Provider::from(OpenAI::from_env()?);
    let reviewer = Agent::builder()
        .name("reviewer")
        .description("Reviews a draft answer for missing details and mistakes.")
        .instructions("Review the draft you are given. Return at most three concrete corrections.")
        .model(Model::new("gpt-6-luna", provider.clone()))
        .build()?;
    let lead = Agent::builder()
        .name("lead")
        .instructions(LEAD_INSTRUCTIONS)
        .model(Model::new("gpt-6.2-sol", provider))
        // The lead may spawn itself (the default) and the reviewer; at most
        // three spawned agents work at once.
        .capability(SpawnAgents::new().max_open(3))
        .spawns(reviewer)
        .max_iterations(12)
        // Show each one-off call as it starts.
        .on_tool_start(|call| async move {
            if call.tool_name == "spawn_agent" {
                let agent = call.arguments["agent"].as_str().unwrap_or("self");
                let wait = call.arguments["wait"].as_bool().unwrap_or(false);
                println!("spawn_agent -> {agent} (wait: {wait})");
            }
        })
        .build()?;
    let session = Engine::new().create(lead);
    let turn = session
        .run("Explain Rust ownership and borrowing with one small example.")
        .await?;
    if !turn.success {
        return Err(format!("lead failed: {:?}", turn.error).into());
    }
    println!("\n{}", turn.response);
    Ok(())
}

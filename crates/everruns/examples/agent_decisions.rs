//! Give an agent the decision service as a tool.
//!
//! The counterpart to `direct_decisions`: there your code asks the
//! questions, here the agent writes its own and acts on the numbers.
//!
//! Requires TYPESAFE_API_KEY and OPENAI_API_KEY:
//! ```text
//! cargo run -p everruns --features typesafe,openai --example agent_decisions
//! ```

use everruns::{Agent, Engine, Jev, OpenAI};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("OpenAI gpt-6.1-sol, classifying through TypeSafe.\n");

    // The same tool the hosted capability contributes, attached to an agent you
    // build yourself. The key is the application's own.
    let jev = Jev::from_env()?;
    let agent = Agent::builder()
        .name("reviewer")
        .instructions(
            "You review copy. When asked how something reads, measure it with \
             jev_decision and report the numbers rather than judging by eye.",
        )
        .provider(OpenAI::from_env()?)
        .model("gpt-6.1-sol")
        .capability(jev)
        .build()?;

    let session = Engine::new().create(agent);
    let turn = session
        .run("Rate this subject line for pushiness: 'Act now before it is too late'")
        .await?;

    println!("{}", turn.response);
    println!("\ntool calls: {}", turn.tool_calls);
    Ok(())
}

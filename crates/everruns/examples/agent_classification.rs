//! Give an agent the classifier as a tool.
//!
//! The counterpart to `direct_classification`: there your code asks the
//! questions, here the agent writes its own and acts on the numbers.
//!
//! Offline (no API key):
//! ```text
//! cargo run -p everruns --features jev --example agent_classification
//! ```
//! Live (requires TYPESAFE_API_KEY and OPENAI_API_KEY):
//! ```text
//! cargo run -p everruns --features jev,openai --example agent_classification -- --live
//! ```

use everruns::{Agent, Engine, Jev, Model};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let live = std::env::args().nth(1).is_some_and(|arg| arg == "--live");

    let model = if live {
        #[cfg(feature = "openai")]
        {
            println!("OpenAI gpt-5.6-terra, classifying through TypeSafe.\n");
            Model::new("gpt-5.6-terra", everruns::OpenAI::from_env()?)
        }
        #[cfg(not(feature = "openai"))]
        {
            return Err("Live mode needs --features jev,openai".into());
        }
    } else {
        println!("Offline simulator: the model does not really call the tool.\n");
        Model::simulated("The subject line reads as high-pressure marketing.")
    };

    // The same tool the hosted capability contributes, attached to an agent you
    // build yourself. The key is the application's own.
    let key = std::env::var("TYPESAFE_API_KEY").unwrap_or_else(|_| "offline-placeholder".into());
    let agent = Agent::builder()
        .name("reviewer")
        .instructions(
            "You review copy. When asked how something reads, measure it with \
             jev_evaluate and report the numbers rather than judging by eye.",
        )
        .model(model)
        .capability(Jev::new(key))
        .build()?;

    let session = Engine::new().create(agent);
    let turn = session
        .run("Rate this subject line for pushiness: 'Act now before it is too late'")
        .await?;

    println!("{}", turn.response);
    println!("\ntool calls: {}", turn.tool_calls);
    Ok(())
}

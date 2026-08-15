//! Send, steer, and optionally wait on one live Framework session.
//!
//! ```text
//! cargo run -p everruns --example live_session
//! ```

use std::time::Duration;

use everruns::{Agent, InMemoryEngine, LlmSimConfig, Model, SendDisposition};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let model = Model::simulated_with_config(
        LlmSimConfig::fixed("Trip planned.").with_response_delay(Duration::from_millis(100)),
    );
    let agent = Agent::builder()
        .instructions("Plan trips and incorporate every user message.")
        .model(model)
        .build()?;
    let session = InMemoryEngine::new().create(agent.clone());

    let initial = session.send("Plan my trip.").await?;
    let latest = session.send("Prefer trains.").await?;

    match latest.disposition {
        SendDisposition::Steered => {
            println!("latest message joined turn {}", latest.turn_id);
        }
        SendDisposition::Started => {
            println!("latest message started follow-up turn {}", latest.turn_id);
        }
        _ => {}
    }

    // This is correct whether the second message steered the first turn or
    // arrived after it completed and started a follow-up.
    let result = latest.wait().await?;
    println!("{}", result.response);

    // The first receipt remains independently waitable.
    let _ = initial.wait().await?;
    Ok(())
}

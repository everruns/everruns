//! External-consumer acceptance fixture: an out-of-workspace application that
//! depends only on the public `everruns` Framework crate and runs one offline
//! turn. Exercised by `scripts/test-external-consumer.sh`.

use everruns::{Agent, Model};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let agent = Agent::builder()
        .instructions("Return only the answer.")
        .model(Model::simulated("4"))
        .build()?;
    let turn = agent.session().run("What is 2 + 2?").await?;

    assert!(turn.success);
    assert_eq!(turn.response, "4");
    println!("{}", turn.response);
    Ok(())
}

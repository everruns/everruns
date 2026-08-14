use everruns::{Agent, Engine, InMemoryEngine, Model};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let engine = InMemoryEngine::new();
    let agent = Agent::builder()
        .instructions("Be concise.")
        .model(Model::simulated("Hello."))
        .build()?;

    let session = engine.create(agent);
    let session_id = session.session_id();
    println!("{}", session.send_and_wait("hello").await?.response);

    drop(session);
    let resumed = engine.resume(session_id).await?;
    println!(
        "{} messages",
        resumed.history().page().await?.messages.len()
    );
    Ok(())
}

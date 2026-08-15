//! Record and render a complete turn through the canonical Framework event API.
//!
//! Run offline with:
//!
//! ```text
//! cargo run -p everruns --example canonical_events
//! ```

use everruns::prelude::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let agent = Agent::builder()
        .instructions("Answer concisely.")
        .model(Model::simulated("Hello from Everruns."))
        .build()?;
    let session = InMemoryEngine::new().create(agent.clone());
    let mut events = session.events();

    let observer = tokio::spawn(async move {
        let mut rendered = String::new();
        let mut recorded = Vec::new();

        loop {
            let event = match events.recv().await {
                Ok(Some(event)) => event,
                Ok(None) => break,
                Err(error) => return Err(error),
            };
            recorded.push(event.as_json().clone());

            match event.kind {
                SessionEventKind::OutputStarted { .. } => print!("assistant: "),
                SessionEventKind::TextDelta { delta } => {
                    rendered.push_str(&delta);
                    print!("{delta}");
                }
                SessionEventKind::ToolStarted { tool_name, .. } => {
                    eprintln!("starting tool {tool_name}");
                }
                SessionEventKind::ToolCompleted {
                    tool_name, success, ..
                } => eprintln!("tool {tool_name} completed: {success}"),
                SessionEventKind::TurnCompleted => println!("\n[turn completed]"),
                SessionEventKind::TurnFailed { error } => eprintln!("turn failed: {error}"),
                _ => {}
            }
        }

        Ok((rendered, recorded))
    });

    let turn = session.run("Say hello.").await?;
    drop(session); // closes the stream after every buffered event is drained
    let (rendered, recorded) = observer.await??;

    assert_eq!(rendered, turn.response);
    assert!(
        recorded
            .iter()
            .any(|event| event["type"] == "output.message.completed")
    );
    println!("recorded {} canonical envelopes", recorded.len());
    Ok(())
}

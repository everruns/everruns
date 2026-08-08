//! Observe a session's events and cancel a turn — using only `everruns`.
//!
//! Run with:
//!
//! ```text
//! cargo run -p everruns --example observe_and_cancel
//! ```
//!
//! This example imports nothing but `everruns::prelude::*`: it renders streaming
//! output from a turn's [`SessionEvent`] feed, then shows a turn resolving to a
//! cancelled outcome through a [`CancellationToken`].

use everruns::prelude::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let agent = Agent::builder()
        .instructions("You are concise.")
        .model(Model::simulated("Hello from Everruns!"))
        .build()?;

    // --- Observe a turn's events -----------------------------------------
    let mut session = agent.session();
    let mut events = session.events();

    // Read the event stream on a background task while the turn runs.
    let observer = tokio::spawn(async move {
        let mut rendered = String::new();
        while let Some(event) = events.recv().await {
            match event.kind {
                SessionEventKind::TurnStarted => println!("[turn started]"),
                SessionEventKind::TextDelta { delta } => {
                    rendered.push_str(&delta);
                    print!("{delta}");
                }
                SessionEventKind::TurnCompleted => println!("\n[turn completed]"),
                _ => {}
            }
        }
        rendered
    });

    let turn = session.run("hi").await?;
    println!("final response: {}", turn.response);

    // Drop the session so the stream closes and the observer finishes.
    drop(session);
    let rendered = observer.await?;
    assert_eq!(rendered, turn.response);

    // --- Cancel a turn ---------------------------------------------------
    let mut session = agent.session();
    let token = CancellationToken::new();
    token.cancel(); // cancel up front for a deterministic example

    let cancelled = session
        .run_with("hi", RunOptions::new().cancel_token(token))
        .await?;
    assert_eq!(cancelled.stop_reason, TurnStopReason::Cancelled);
    assert!(!cancelled.success);
    println!("cancelled turn stop reason: {:?}", cancelled.stop_reason);

    Ok(())
}

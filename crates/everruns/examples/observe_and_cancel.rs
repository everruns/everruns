//! Observe a session's events and cancel a turn — using only `everruns`.
//!
//! Run with:
//!
//! ```text
//! cargo run -p everruns --features openai --example observe_and_cancel
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
        .provider(OpenAI::from_env()?)
        .model("gpt-5.6-terra")
        .build()?;

    // --- Observe a turn's events -----------------------------------------
    let session = InMemoryEngine::new().create(agent.clone());
    let mut events = session.events();

    let pending = session.send("hi").await?;
    let mut rendered = String::new();
    while let Some(event) = events.recv().await? {
        let terminal =
            event.turn_id.as_deref() == Some(&pending.turn_id) && event.kind.is_terminal();
        match event.kind {
            SessionEventKind::TurnStarted => println!("[turn started]"),
            SessionEventKind::TextDelta { delta } => {
                rendered.push_str(&delta);
                print!("{delta}");
            }
            SessionEventKind::TurnCompleted => println!("\n[turn completed]"),
            _ => {}
        }
        if terminal {
            break;
        }
    }
    let turn = pending.wait().await?;
    println!("final response: {}", turn.response);
    assert_eq!(rendered, turn.response);

    // --- Cancel a turn ---------------------------------------------------
    let session = InMemoryEngine::new().create(agent.clone());
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

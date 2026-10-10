//! An agent that talks through `send_message` instead of its assistant text.
//!
//! Run offline with:
//!
//! ```text
//! cargo run -p everruns --example explicit_communication
//! ```
//!
//! With `Communication::Explicit`, the agent's assistant text is private
//! working notes. It says something to the person only by calling
//! `send_message`, and every sent message arrives on the event stream as
//! `SessionEventKind::MessageSent`. This example scripts the model with
//! `llmsim`, so it runs without an API key, and prints what the agent said
//! separately from what it only noted.

use everruns::conversation::Communication;
use everruns::prelude::*;
use everruns::{LlmSimConfig, ToolCall};
use serde_json::json;

/// A model that writes a note, sends one message, then ends the turn with a
/// second note. `llmsim` returns one entry per model call.
fn scripted_model() -> Model {
    Model::simulated_with_config(
        LlmSimConfig::sequence(vec![
            "Note: they want the deploy status. Staging is green, production waits on review."
                .into(),
            "Note: status sent, nothing else to say this turn.".into(),
        ])
        .with_tool_call_sequence(vec![
            vec![ToolCall {
                id: "call_send_1".into(),
                name: "send_message".into(),
                arguments: json!({
                    "text": "Staging is green. Production is waiting on one review."
                }),
            }],
            vec![],
        ]),
    )
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let agent = Agent::builder()
        .instructions("You report deployment status.")
        .model(scripted_model())
        .communication(Communication::Explicit)
        .build()?;
    let session = Engine::new().create(agent);
    let mut events = session.events();

    let observer = tokio::spawn(async move {
        let mut sent = Vec::new();
        let mut notes = Vec::new();
        let mut current = String::new();
        loop {
            let event = match events.recv().await {
                Ok(Some(event)) => event,
                Ok(None) => break,
                Err(error) => return Err(error),
            };
            match event.kind {
                // For an explicit agent, assistant text is working notes.
                SessionEventKind::TextDelta { delta } => current.push_str(&delta),
                SessionEventKind::OutputCompleted { .. } => {
                    if !current.trim().is_empty() {
                        notes.push(std::mem::take(&mut current));
                    }
                }
                // What the agent said to the conversation.
                SessionEventKind::MessageSent { text, .. } => sent.push(text),
                _ => {}
            }
        }
        Ok((sent, notes))
    });

    let turn = session.run("How is the deploy going?").await?;
    drop(session); // closes the stream after every buffered event is drained
    let (sent, notes) = observer.await??;

    println!("sent messages:");
    for text in &sent {
        println!("  > {text}");
    }
    println!("working notes (never shown to the person):");
    for note in &notes {
        println!("  - {note}");
    }

    assert!(turn.success);
    assert_eq!(
        sent,
        ["Staging is green. Production is waiting on one review."]
    );
    assert!(notes.iter().all(|note| !sent.contains(note)));
    Ok(())
}

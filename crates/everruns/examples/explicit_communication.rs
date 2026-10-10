//! An agent that talks through `send_message` instead of its assistant text,
//! run against `gpt-5.6-terra`.
//!
//! ```text
//! cargo run -p everruns --features openai --example explicit_communication
//! ```
//!
//! With `Communication::Explicit`, the agent's assistant text is private
//! working notes. It says something to the person only by calling
//! `send_message` (or stays silent on purpose with `no_reply`), and every sent
//! message arrives on the event stream as `SessionEventKind::MessageSent`. The
//! example prints what the agent said separately from what it only noted.

use everruns::OpenAI;
use everruns::conversation::Communication;
use everruns::prelude::*;

/// Return the current deployment status of each environment.
#[everruns::tool]
async fn deploy_status() -> Result<String, String> {
    Ok("staging: green, deployed 12 minutes ago\n\
        production: blocked, waiting on 1 required review (PR #812)"
        .to_string())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let agent = Agent::builder()
        .name("deploy-reporter")
        .instructions(
            "You report deployment status. Check deploy_status before answering. \
             Keep what you send short.",
        )
        .provider(OpenAI::from_env()?)
        .model("gpt-5.6-terra")
        .tool(deploy_status())
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
        println!("  > {}", text.trim().replace('\n', "\n    "));
    }
    println!("working notes (never shown to the person):");
    for note in &notes {
        println!("  - {}", note.trim());
    }
    if notes.is_empty() {
        println!("  (none this turn)");
    }

    assert!(turn.success);
    assert!(
        !sent.is_empty(),
        "a status question should get a sent reply"
    );
    Ok(())
}

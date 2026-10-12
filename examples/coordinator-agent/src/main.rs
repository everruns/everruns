#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! A coordinator hands a launch to two threads and reports back.
//!
//! ```text
//! OPENAI_API_KEY=... cargo run -p everruns-coordinator-agent
//! ```

use std::time::Duration;

use everruns::coordination;
use everruns::{Engine, Session};
use everruns_coordinator_agent::{
    LAUNCH_REQUEST, RESOLVE_REQUEST, all_ready, coordinator, openai_model, render_board, replies,
    wait_for_new_reply, wait_for_threads,
};

/// How long to wait for the threads, and for the coordinator's update.
const WAIT: Duration = Duration::from_secs(300);

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let model = openai_model()?;
    let workspace =
        std::env::temp_dir().join(format!("everruns-coordinator-{}", std::process::id()));
    std::fs::create_dir_all(&workspace)?;
    let engine = Engine::new();
    let session = engine.create(coordinator(model, &workspace)?);

    say(&session, LAUNCH_REQUEST).await?;
    let seen = replies(&session).await?.len();
    let threads = wait_for_threads(&session, WAIT, all_ready).await?;
    println!("\nBoard\n{}\n", render_board(&threads));

    // Each finished thread woke the coordinator with an automatic update.
    if let Some(reply) = wait_for_new_reply(&session, seen, WAIT).await? {
        println!("coordinator> {reply}   (automatic update)\n");
    }

    say(&session, RESOLVE_REQUEST).await?;
    let threads = coordination::threads(&session).await?;
    println!("\nBoard\n{}", render_board(&threads));
    println!("\nDrafts are in {}", workspace.display());
    Ok(())
}

async fn say(session: &Session, text: &str) -> Result<(), Box<dyn std::error::Error>> {
    println!("person> {text}");
    let turn = session.run(text).await?;
    println!("coordinator> {}", turn.response);
    Ok(())
}

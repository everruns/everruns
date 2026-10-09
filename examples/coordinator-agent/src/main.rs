#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! A coordinator hands a launch to two threads and reports back.
//!
//! ```text
//! # Offline, deterministic, no key required
//! cargo run -p everruns-coordinator-agent -- --offline
//!
//! # Against OpenAI
//! OPENAI_API_KEY=... cargo run -p everruns-coordinator-agent
//! ```

use std::time::Duration;

use everruns::coordination;
use everruns::providers::openai::OpenAI;
use everruns::{Engine, Model, Session};
use everruns_coordinator_agent::{
    LAUNCH_REQUEST, RESOLVE_REQUEST, all_ready, coordinator, render_board, replies,
    simulated_model, wait_for_new_reply, wait_for_threads,
};

const OPENAI_MODEL: &str = "gpt-5.6-terra";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let offline = std::env::args().any(|arg| arg == "--offline");
    let model = if offline {
        simulated_model()
    } else {
        Model::new(OPENAI_MODEL, OpenAI::from_env()?)
    };
    let workspace =
        std::env::temp_dir().join(format!("everruns-coordinator-{}", std::process::id()));
    std::fs::create_dir_all(&workspace)?;
    let engine = Engine::new();
    let session = engine.create(coordinator(model, &workspace)?);
    let wait = if offline { 10 } else { 300 };

    say(&session, LAUNCH_REQUEST).await?;
    let seen = replies(&session).await?.len();
    let threads = wait_for_threads(&session, Duration::from_secs(wait), all_ready).await?;
    println!("\nBoard\n{}\n", render_board(&threads));

    // Each finished thread woke the coordinator with an automatic update.
    if let Some(reply) = wait_for_new_reply(&session, seen, Duration::from_secs(wait)).await? {
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

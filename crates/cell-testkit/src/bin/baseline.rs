//! The framework alone: the same agents and turns as a load run, on one
//! process under Everruns' own in-memory authority (`MemoryHosting`) and no
//! Raft, to tell the runtime's cost from the framework's.
//!
//! ```text
//! cell-baseline --agent worker3 --sessions 400 --turns 2
//! ```

use std::sync::Arc;
use std::time::Instant;

use clap::Parser;
use everruns::SessionId;
use everruns_cell::Host;
use futures::stream::{self, StreamExt};
use serde_json::json;

#[derive(Parser)]
struct Cli {
    #[arg(long, default_value = "worker3")]
    agent: String,
    #[arg(long, default_value = "400")]
    sessions: usize,
    #[arg(long, default_value = "2")]
    turns: usize,
}

fn cpu_secs() -> f64 {
    let stat = std::fs::read_to_string("/proc/self/stat").unwrap_or_default();
    let fields: Vec<&str> = stat
        .rsplit_once(')')
        .map(|(_, rest)| rest.split_whitespace().collect())
        .unwrap_or_default();
    let ticks = |i: usize| {
        fields
            .get(i)
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(0.0)
    };
    // utime and stime are fields 14 and 15; `rest` starts at field 3.
    (ticks(11) + ticks(12)) / 100.0
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let host = Arc::new(everruns_cell_testkit::host());
    let definition = everruns_cell_testkit::definition(&cli.agent)
        .ok_or_else(|| anyhow::anyhow!("no test agent {}", cli.agent))?;
    let build = everruns::hosting::AgentBuild::new(cli.agent.clone(), "baseline");
    let agent = host
        .agent(&definition)
        .map_err(|error| anyhow::anyhow!("cannot build the test agent: {error}"))?;
    let engine = everruns::Engine::builder()
        .hosting(everruns::hosting::MemoryHosting::default())
        .register(build.clone(), agent)
        .build();
    let cpu = cpu_secs();
    let started = Instant::now();
    let turns: anyhow::Result<usize> = stream::iter(0..cli.sessions)
        .map(|_| {
            let engine = engine.clone();
            let build = build.clone();
            async move {
                let session = engine
                    .create_registered(SessionId::new(), &build)
                    .await
                    .map_err(|error| anyhow::anyhow!("create: {error}"))?;
                let mut done = 0;
                for turn in 0..cli.turns {
                    let result = session
                        .send_and_wait(format!("turn {turn}"))
                        .await
                        .map_err(|error| anyhow::anyhow!("turn: {error}"))?;
                    done += usize::from(result.success);
                }
                anyhow::Ok(done)
            }
        })
        .buffer_unordered(cli.sessions)
        .fold(Ok(0), |sum, n| async move { Ok(sum? + n?) })
        .await;
    let turns = turns?;
    let wall = started.elapsed().as_secs_f64();
    let cpu = cpu_secs() - cpu;
    println!(
        "{}",
        json!({
            "turns": turns,
            "wall_secs": wall,
            "turns_per_sec": turns as f64 / wall,
            "cpu_secs": cpu,
            "cpu_ms_per_turn": cpu * 1000.0 / turns as f64,
        })
    );
    Ok(())
}

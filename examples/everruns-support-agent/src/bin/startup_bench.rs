//! Startup latency benchmark for the Framework support agent.
//!
//! Measures the time to a session that is ready to accept a message: the tokio
//! runtime, the Agent, the Engine, the Session, and `Session::start` (binds the
//! default environment and initializes host backends). It stops at ready. No
//! model call is made and process teardown is excluded (the process exits
//! immediately, without running destructors).
//!
//! ```text
//! # one cold process, the number that matters
//! cargo run --release -p everruns-framework-support-agent --bin startup_bench
//! # steady-state cost of the same path, 500 iterations in one process
//! cargo run --release -p everruns-framework-support-agent --bin startup_bench -- --warm
//! ```
//!
//! `startup_launcher.c` next to this file prints a CLOCK_MONOTONIC timestamp
//! and then `execv`s this binary, which is how exec and dynamic-linking time
//! before `main` is attributed.

use std::time::{Duration, Instant};

use everruns::{Agent, Engine, Session};

const MODEL: &str = "claude-opus-5";
const ITERATIONS: usize = 500;

#[everruns::tool]
/// Read authoritative Framework documentation for a support topic.
async fn search_docs(topic: String) -> Result<String, String> {
    Ok(topic)
}

fn build_agent(api_key: &str) -> Result<Agent, everruns::BuildError> {
    Agent::builder()
        .name("everruns-support-agent")
        .instructions("Keep the final answer within 150 words. You support Everruns Framework users. Use search_docs before answering. Separate evidence from hypotheses and give the smallest safe next step with relevant documentation links.")
        .provider(everruns_anthropic::provider("anthropic", api_key))
        .model(MODEL)
        .tool(search_docs())
        .build()
}

/// CLOCK_MONOTONIC in seconds, comparable with the launcher's timestamp.
fn monotonic() -> f64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `ts` is a valid, writable timespec for the duration of the call.
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    ts.tv_sec as f64 + ts.tv_nsec as f64 / 1e9
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1e3
}

async fn ready() -> Result<Session, Box<dyn std::error::Error>> {
    let agent = build_agent("bench-key")?;
    let engine = Engine::new();
    let session = engine.create(agent);
    session.start().await?;
    Ok(session)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let main_entry = monotonic();
    let started = Instant::now();

    // `--current-thread` measures the single-threaded runtime; the default
    // matches `#[tokio::main]`, which starts one worker thread per core.
    let current_thread = std::env::args().any(|arg| arg == "--current-thread");
    let step = Instant::now();
    let runtime = if current_thread {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?
    } else {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?
    };
    let runtime_built = step.elapsed();

    if std::env::args().any(|arg| arg == "--warm") {
        return runtime.block_on(warm());
    }

    let step = Instant::now();
    let agent = build_agent("bench-key")?;
    let agent_built = step.elapsed();

    let step = Instant::now();
    let engine = Engine::new();
    let session = engine.create(agent);
    let session_created = step.elapsed();

    let step = Instant::now();
    runtime.block_on(session.start())?;
    let session_started = step.elapsed();

    let in_process = started.elapsed();

    println!("main_entry_monotonic {main_entry:.6}");
    println!("tokio_runtime_ms     {:.3}", ms(runtime_built));
    println!("agent_build_ms       {:.3}", ms(agent_built));
    println!("session_create_ms    {:.3}", ms(session_created));
    println!("session_start_ms     {:.3}", ms(session_started));
    println!("main_to_ready_ms     {:.3}", ms(in_process));
    println!("ready_monotonic      {:.6}", monotonic());

    // Stop at ready: no model call, and no teardown in the measured window.
    std::process::exit(0);
}

async fn warm() -> Result<(), Box<dyn std::error::Error>> {
    let mut samples = Vec::with_capacity(ITERATIONS);
    for _ in 0..ITERATIONS {
        let step = Instant::now();
        let session = ready().await?;
        samples.push(step.elapsed());
        drop(session);
    }
    samples.sort_unstable();
    let mean = samples.iter().sum::<Duration>().as_secs_f64() * 1e3 / samples.len() as f64;
    let at = |quantile: f64| ms(samples[((samples.len() - 1) as f64 * quantile) as usize]);
    println!(
        "warm ready (excludes tokio runtime, {ITERATIONS} iterations): min {:.3}ms p50 {:.3}ms mean {mean:.3}ms p99 {:.3}ms",
        ms(samples[0]),
        at(0.50),
        at(0.99),
    );
    Ok(())
}

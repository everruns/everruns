//! Startup latency benchmark for the Framework support agent.
//!
//! Measures the local, provider-free cost of getting the agent ready to accept
//! a message: build the Agent, construct the Engine, create the Session, and
//! start it (binds the default environment and initializes host backends).
//! No network calls are made; the API key is a placeholder.
//!
//! ```text
//! cargo run --release -p everruns-framework-support-agent --bin startup_bench
//! ```

use std::time::{Duration, Instant};

use everruns::{Agent, Engine};

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

fn report(label: &str, mut samples: Vec<Duration>) {
    samples.sort_unstable();
    let micros = |d: Duration| d.as_secs_f64() * 1_000.0;
    let p = |q: f64| micros(samples[((samples.len() - 1) as f64 * q) as usize]);
    let mean = samples.iter().sum::<Duration>().as_secs_f64() * 1_000.0 / samples.len() as f64;
    println!(
        "{label:<28} min {:>8.3}ms  p50 {:>8.3}ms  mean {:>8.3}ms  p99 {:>8.3}ms  max {:>8.3}ms",
        micros(samples[0]),
        p(0.50),
        mean,
        p(0.99),
        micros(samples[samples.len() - 1]),
    );
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let process_start = Instant::now();

    // Cold path: the very first time through, before any caches are warm.
    let cold = Instant::now();
    let agent = build_agent("bench-key")?;
    let cold_build = cold.elapsed();
    let cold = Instant::now();
    let engine = Engine::new();
    let session = engine.create(agent);
    let cold_create = cold.elapsed();
    let cold = Instant::now();
    session.start().await?;
    let cold_start = cold.elapsed();
    let cold_total = process_start.elapsed();

    if std::env::var_os("STARTUP_BENCH_COLD_ONLY").is_some() {
        println!("cold total {:.3}ms", cold_total.as_secs_f64() * 1e3);
        return Ok(());
    }

    let mut build = Vec::with_capacity(ITERATIONS);
    let mut engine_new = Vec::with_capacity(ITERATIONS);
    let mut create = Vec::with_capacity(ITERATIONS);
    let mut start = Vec::with_capacity(ITERATIONS);
    let mut total = Vec::with_capacity(ITERATIONS);

    for _ in 0..ITERATIONS {
        let all = Instant::now();
        let step = Instant::now();
        let agent = build_agent("bench-key")?;
        build.push(step.elapsed());
        let step = Instant::now();
        let engine = Engine::new();
        engine_new.push(step.elapsed());
        let step = Instant::now();
        let session = engine.create(agent);
        create.push(step.elapsed());
        let step = Instant::now();
        session.start().await?;
        start.push(step.elapsed());
        total.push(all.elapsed());
    }

    println!("iterations: {ITERATIONS}\n");
    println!("cold (first pass in a fresh process)");
    println!(
        "  Agent::build           {:>8.3}ms",
        cold_build.as_secs_f64() * 1e3
    );
    println!(
        "  Engine::new + create   {:>8.3}ms",
        cold_create.as_secs_f64() * 1e3
    );
    println!(
        "  Session::start         {:>8.3}ms",
        cold_start.as_secs_f64() * 1e3
    );
    println!(
        "  total                  {:>8.3}ms",
        cold_total.as_secs_f64() * 1e3
    );
    println!();
    report("Agent::build", build);
    report("Engine::new", engine_new);
    report("Engine::create", create);
    report("Session::start", start);
    report("ready-to-send (total)", total);
    Ok(())
}

//! Load and invariant checks against a running cell fleet.
//!
//! ```text
//! cell-drive load  --gateways http://127.0.0.1:7101 --agent worker --sessions 1000 --turns 3
//! cell-drive check --gateways http://127.0.0.1:7101 --sessions-file s.txt --turns 3 --ledger l.txt
//! ```

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use clap::{Parser, Subcommand};
use futures::stream::{self, StreamExt};
use serde_json::{Value, json};

#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    command: Mode,
}

#[derive(Subcommand)]
enum Mode {
    /// Create sessions and run turns in each; report throughput and latency.
    Load {
        /// Comma-separated gateway base URLs; requests rotate across them.
        #[arg(long)]
        gateways: String,
        /// The agent every session runs: a test definition by name (see
        /// `everruns_cell_testkit::definition`), or `@path` to a JSON definition.
        #[arg(long, default_value = "worker")]
        agent: String,
        #[arg(long, default_value = "100")]
        sessions: usize,
        #[arg(long, default_value = "1")]
        turns: u64,
        /// Sessions driven at once.
        #[arg(long, default_value = "200")]
        concurrency: usize,
        /// Write created session ids here.
        #[arg(long)]
        sessions_file: Option<PathBuf>,
        /// Keep retrying failed requests (for chaos runs).
        #[arg(long)]
        persistent: bool,
        /// Everruns llmsim scenarios to play (needs `--agent sim`): a
        /// comma-separated list, session `i` playing entry `i mod n`, or
        /// `mix` for a production-like blend.
        #[arg(long, default_value = "")]
        scenarios: String,
        /// Open loop: start new sessions at this mean rate per second
        /// (Poisson arrivals) instead of all at once. `--sessions` then caps
        /// how many start (0: no cap), and `--concurrency` is ignored.
        #[arg(long, default_value = "0")]
        rate: f64,
        /// With `--rate`: keep starting sessions for this many seconds.
        #[arg(long, default_value = "60")]
        duration: f64,
        /// Mean pause between a session's turns, in seconds (exponential,
        /// like a person reading and typing). 0: send the next message as
        /// soon as the turn ends.
        #[arg(long, default_value = "0")]
        think: f64,
    },
    /// Print a test agent's definition (what a client sends to create a
    /// session running it).
    Agent { name: String },
    /// Verify every session's log against the durability invariants.
    Check {
        #[arg(long)]
        gateways: String,
        #[arg(long)]
        sessions_file: PathBuf,
        #[arg(long)]
        turns: u64,
        #[arg(long)]
        ledger: Option<PathBuf>,
    },
}

struct Client {
    http: reqwest::Client,
    gateways: Vec<String>,
    next: AtomicUsize,
    persistent: bool,
}

impl Client {
    fn base(&self) -> &str {
        &self.gateways[self.next.fetch_add(1, Ordering::Relaxed) % self.gateways.len()]
    }

    async fn send(&self, method: &str, path: &str, body: Option<&Value>) -> anyhow::Result<Value> {
        let deadline = Instant::now() + Duration::from_secs(if self.persistent { 600 } else { 60 });
        loop {
            let url = format!("{}{path}", self.base());
            let request = match (method, body) {
                ("POST", Some(body)) => self.http.post(url).json(body),
                _ => self.http.get(url),
            };
            match request.timeout(Duration::from_secs(60)).send().await {
                Ok(response) if response.status().is_success() => {
                    return Ok(response.json().await?);
                }
                Ok(response) if !self.persistent || Instant::now() > deadline => {
                    let status = response.status();
                    anyhow::bail!(
                        "{method} {path}: {status} {}",
                        response.text().await.unwrap_or_default()
                    )
                }
                Err(error) if !self.persistent || Instant::now() > deadline => {
                    return Err(error.into());
                }
                _ => tokio::time::sleep(Duration::from_millis(200)).await,
            }
        }
    }
}

/// The message for `turn`: a scenario marker makes the sim agent play it.
fn text(scenario: &str, turn: u64) -> String {
    if scenario.is_empty() {
        format!("turn {turn}")
    } else {
        format!("[[llmsim:{scenario}]] turn {turn}")
    }
}

/// One session's admit and turn latencies, and its scenario.
type SessionRun = (Vec<u128>, Vec<u128>, String);

fn percentile(sorted: &[u128], q: f64) -> u128 {
    if sorted.is_empty() {
        return 0;
    }
    sorted[((sorted.len() as f64 - 1.0) * q) as usize]
}

fn summary(mut samples: Vec<u128>) -> Value {
    samples.sort_unstable();
    json!({
        "count": samples.len(),
        "p50_ms": percentile(&samples, 0.5) as f64 / 1000.0,
        "p90_ms": percentile(&samples, 0.9) as f64 / 1000.0,
        "p99_ms": percentile(&samples, 0.99) as f64 / 1000.0,
        "max_ms": samples.last().copied().unwrap_or(0) as f64 / 1000.0,
    })
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        Mode::Load {
            gateways,
            agent,
            sessions,
            turns,
            concurrency,
            sessions_file,
            persistent,
            scenarios,
            rate,
            duration,
            think,
        } => {
            let shape = Shape {
                sessions,
                turns,
                concurrency,
                rate,
                duration,
                think,
            };
            load(
                gateways,
                agent,
                shape,
                sessions_file,
                persistent,
                scenario_list(&scenarios),
            )
            .await
        }
        Mode::Check {
            gateways,
            sessions_file,
            turns,
            ledger,
        } => check(gateways, sessions_file, turns, ledger).await,
        Mode::Agent { name } => {
            let definition = everruns_cell_testkit::definition(&name)
                .ok_or_else(|| anyhow::anyhow!("no test agent {name}"))?;
            println!("{}", serde_json::to_string_pretty(&definition)?);
            Ok(())
        }
    }
}

#[expect(
    clippy::expect_used,
    reason = "a client with no TLS config only fails to build when the TLS backend cannot start, and the driver is useless without one"
)]
fn client(gateways: &str, persistent: bool) -> Arc<Client> {
    Arc::new(Client {
        http: reqwest::Client::builder()
            .no_proxy() // fleet traffic is direct, whatever the system proxy says
            .pool_max_idle_per_host(1024)
            .tcp_nodelay(true)
            .build()
            .expect("client"),
        gateways: gateways.split(',').map(|s| s.trim().to_string()).collect(),
        next: AtomicUsize::new(0),
        persistent,
    })
}

/// A production-like blend of the Everruns llmsim scenarios, weighted by
/// repetition: mostly short chat and tool-using research turns, some
/// reasoning, long answers, slow first tokens, a rate limit and long tools.
const MIX: &[&str] = &[
    "chat-short",
    "chat-short",
    "chat-short",
    "chat-short",
    "sol-agent",
    "sol-agent",
    "luna-agent",
    "research-3-tools",
    "file-research",
    "parallel-tools",
    "thinking",
    "long-answer",
    "slow-first-token",
    "flaky-429",
    "long-tool",
];

fn scenario_list(arg: &str) -> Vec<String> {
    match arg.trim() {
        "" => Vec::new(),
        "mix" => MIX.iter().map(|s| s.to_string()).collect(),
        list => list.split(',').map(|s| s.trim().to_string()).collect(),
    }
}

/// How a load run creates sessions and paces their turns.
struct Shape {
    sessions: usize,
    turns: u64,
    concurrency: usize,
    rate: f64,
    duration: f64,
    think: f64,
}

/// An exponential sample with this mean, capped at five means (a person
/// who wandered off for an hour would only stretch the run).
fn exponential(mean: f64) -> Duration {
    if mean <= 0.0 {
        return Duration::ZERO;
    }
    let u: f64 = rand::random::<f64>().max(1e-12);
    Duration::from_secs_f64((-u.ln() * mean).min(5.0 * mean))
}

async fn create(client: &Client, agent: &Value) -> anyhow::Result<String> {
    let session = client
        .send("POST", "/v1/sessions", Some(&json!({ "agent": agent })))
        .await?;
    Ok(session["id"].as_str().unwrap_or_default().to_string())
}

/// Run `turns` turns in session `id`, pausing `think` (mean) between them.
async fn drive_session(
    client: Arc<Client>,
    id: String,
    turns: u64,
    scenario: String,
    think: f64,
    progress: Arc<Progress>,
) -> anyhow::Result<SessionRun> {
    let mut admits = Vec::new();
    let mut turn_latency = Vec::new();
    for turn in 1..=turns {
        if turn > 1 {
            tokio::time::sleep(exponential(think)).await;
        }
        let at = Instant::now();
        let message_id = format!("message_{}", uuid_hex());
        client
            .send(
                "POST",
                &format!("/v1/sessions/{id}/messages"),
                Some(&json!({ "text": text(&scenario, turn), "id": message_id })),
            )
            .await?;
        admits.push(at.elapsed().as_micros());
        progress.running.fetch_add(1, Ordering::Relaxed);
        let waited = async {
            loop {
                let view = client
                    .send("GET", &format!("/v1/sessions/{id}/wait?turns={turn}"), None)
                    .await?;
                if view["turns_completed"].as_u64().unwrap_or(0) >= turn {
                    return anyhow::Ok(());
                }
            }
        }
        .await;
        progress.running.fetch_sub(1, Ordering::Relaxed);
        waited?;
        progress.turns.fetch_add(1, Ordering::Relaxed);
        turn_latency.push(at.elapsed().as_micros());
    }
    anyhow::Ok((admits, turn_latency, scenario))
}

#[derive(Default)]
struct Progress {
    sessions: AtomicUsize,
    running: AtomicUsize,
    turns: AtomicUsize,
}

/// Print progress every 10 s to stderr, for long runs.
fn report_progress(progress: Arc<Progress>, started: Instant) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(10));
        tick.tick().await;
        let mut last = 0;
        loop {
            tick.tick().await;
            let turns = progress.turns.load(Ordering::Relaxed);
            eprintln!(
                "{:>6.0}s  sessions {:>6}  turns running {:>5}  turns done {:>7}  ({:.1}/s)",
                started.elapsed().as_secs_f64(),
                progress.sessions.load(Ordering::Relaxed),
                progress.running.load(Ordering::Relaxed),
                turns,
                (turns - last) as f64 / 10.0,
            );
            last = turns;
        }
    })
}

async fn load(
    gateways: String,
    agent: String,
    shape: Shape,
    sessions_file: Option<PathBuf>,
    persistent: bool,
    scenarios: Vec<String>,
) -> anyhow::Result<()> {
    let client = client(&gateways, persistent);
    // The client owns the agent: every session carries its definition.
    let agent: Value = match agent.strip_prefix('@') {
        Some(path) => serde_json::from_str(&std::fs::read_to_string(path)?)?,
        None => everruns_cell_testkit::definition(&agent)
            .ok_or_else(|| anyhow::anyhow!("no test agent {agent}"))?,
    };
    let scenario_for = |i: usize| {
        if scenarios.is_empty() {
            String::new()
        } else {
            scenarios[i % scenarios.len()].clone()
        }
    };
    let progress = Arc::new(Progress::default());
    let Shape {
        sessions,
        turns,
        concurrency,
        rate,
        duration,
        think,
    } = shape;

    let started;
    let create_secs;
    let ids: Vec<String>;
    let results: Vec<anyhow::Result<SessionRun>>;
    if rate > 0.0 {
        // Open loop: sessions arrive on their own clock, whether or not the
        // fleet keeps up, the way users do.
        started = Instant::now();
        let ticker = report_progress(progress.clone(), started);
        let mut tasks = tokio::task::JoinSet::new();
        let mut created = Vec::new();
        let mut i = 0;
        while started.elapsed().as_secs_f64() < duration && (sessions == 0 || i < sessions) {
            tokio::time::sleep(exponential(1.0 / rate)).await;
            let (client, agent, progress) = (client.clone(), agent.clone(), progress.clone());
            let scenario = scenario_for(i);
            let (tx, rx) = tokio::sync::oneshot::channel();
            created.push(rx);
            tasks.spawn(async move {
                let id = create(&client, &agent).await;
                let _ = tx.send(id.as_ref().ok().cloned());
                progress.sessions.fetch_add(1, Ordering::Relaxed);
                drive_session(client, id?, turns, scenario, think, progress).await
            });
            i += 1;
        }
        create_secs = started.elapsed().as_secs_f64();
        results = tasks.join_all().await;
        ticker.abort();
        let mut out = Vec::new();
        for rx in created {
            if let Ok(Some(id)) = rx.await {
                out.push(id);
            }
        }
        ids = out;
        if let Some(path) = &sessions_file {
            std::fs::write(path, ids.join("\n"))?;
        }
    } else {
        let created = Instant::now();
        ids = stream::iter(0..sessions)
            .map(|_| {
                let (client, agent) = (client.clone(), agent.clone());
                async move { create(&client, &agent).await }
            })
            .buffer_unordered(concurrency)
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<_, _>>()?;
        create_secs = created.elapsed().as_secs_f64();
        if let Some(path) = &sessions_file {
            std::fs::write(path, ids.join("\n"))?;
        }
        progress.sessions.store(ids.len(), Ordering::Relaxed);
        started = Instant::now();
        let ticker = report_progress(progress.clone(), started);
        results = stream::iter(ids.clone().into_iter().enumerate())
            .map(|(i, id)| {
                drive_session(
                    client.clone(),
                    id,
                    turns,
                    scenario_for(i),
                    think,
                    progress.clone(),
                )
            })
            .buffer_unordered(concurrency)
            .collect()
            .await;
        ticker.abort();
    }
    let wall = started.elapsed().as_secs_f64();
    let mut admits = Vec::new();
    let mut latencies = Vec::new();
    let mut errors = Vec::new();
    let mut by_scenario: HashMap<String, Vec<u128>> = HashMap::new();
    // Turn latency by how far into its session the turn is (tenths), to see
    // whether turns slow down as sessions grow.
    let mut by_tenth: Vec<Vec<u128>> = vec![Vec::new(); 10];
    for result in results {
        match result {
            Ok((a, l, scenario)) => {
                for (i, latency) in l.iter().enumerate() {
                    by_tenth[(i * 10 / l.len().max(1)).min(9)].push(*latency);
                }
                admits.extend(a);
                if !scenario.is_empty() {
                    by_scenario.entry(scenario).or_default().extend(&l);
                }
                latencies.extend(l);
            }
            Err(error) => errors.push(error.to_string()),
        }
    }
    let cells = client
        .send("GET", "/v1/cells", None)
        .await
        .unwrap_or(Value::Null);
    let report = json!({
        "sessions": ids.len(),
        "turns_per_session": turns,
        "rate": rate,
        "think_secs": think,
        "turns_completed": latencies.len(),
        "errors": errors.len(),
        "first_errors": errors.iter().take(5).collect::<Vec<_>>(),
        "create_secs": create_secs,
        "wall_secs": wall,
        "turns_per_sec": latencies.len() as f64 / wall,
        "admit": summary(admits),
        "turn": summary(latencies),
        "turn_p50_ms_by_tenth": by_tenth
            .into_iter()
            .map(|mut samples| {
                samples.sort_unstable();
                percentile(&samples, 0.5) as f64 / 1000.0
            })
            .collect::<Vec<_>>(),
        "scenarios": by_scenario
            .into_iter()
            .map(|(name, samples)| (name, summary(samples)))
            .collect::<serde_json::Map<_, _>>(),
        "cells": cells,
    });
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

/// Every `session_<32 hex>` id in `text`.
fn session_ids(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find("session_") {
        let candidate = &rest[at..];
        let hex = &candidate["session_".len()..];
        if hex.len() >= 32 && hex[..32].bytes().all(|b| b.is_ascii_hexdigit()) {
            out.push(candidate[..40].to_string());
        }
        rest = &rest[at + "session_".len()..];
    }
    out
}

fn uuid_hex() -> String {
    format!("{:032x}", rand::random::<u128>())
}

async fn check(
    gateways: String,
    sessions_file: PathBuf,
    turns: u64,
    ledger: Option<PathBuf>,
) -> anyhow::Result<()> {
    let client = client(&gateways, true);
    let ids: Vec<String> = std::fs::read_to_string(&sessions_file)?
        .lines()
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect();
    let mut executions: HashMap<String, usize> = HashMap::new();
    if let Some(ledger) = &ledger {
        for line in std::fs::read_to_string(ledger).unwrap_or_default().lines() {
            *executions.entry(line.to_string()).or_default() += 1;
        }
    }
    let mut failures = Vec::new();
    let mut totals = json!({ "sessions": ids.len(), "events": 0, "tool_started": 0, "tool_executions": 0, "interrupted": 0, "resumed_turns": 0 });
    for id in &ids {
        let view = client
            .send(
                "GET",
                &format!("/v1/sessions/{id}/wait?turns={turns}&timeout_ms=60000"),
                None,
            )
            .await?;
        let events = client
            .send("GET", &format!("/v1/sessions/{id}/events"), None)
            .await?;
        let events = events["data"].as_array().cloned().unwrap_or_default();
        let mut problems = Vec::new();
        if view["turns_completed"].as_u64() != Some(turns) {
            problems.push(format!(
                "turns_completed {} != {turns}",
                view["turns_completed"]
            ));
        }
        let mut seen = HashSet::new();
        for (i, event) in events.iter().enumerate() {
            if event["sequence"].as_i64() != Some(i as i64 + 1) {
                problems.push(format!("sequence gap at {i}: {}", event["sequence"]));
                break;
            }
            if !seen.insert(event["id"].as_str().unwrap_or_default().to_string()) {
                problems.push(format!("duplicate event id {}", event["id"]));
            }
        }
        let count = |t: &str| events.iter().filter(|e| e["type"] == t).count();
        let inputs = count("input.message");
        let completed = count("turn.completed");
        let started = count("turn.started");
        let tool_started = count("tool.started");
        if inputs as u64 != turns {
            problems.push(format!("{inputs} input.message events, expected {turns}"));
        }
        if completed as u64 != turns {
            problems.push(format!(
                "{completed} turn.completed events, expected {turns}"
            ));
        }
        if started != completed {
            problems.push(format!(
                "{started} turn.started vs {completed} turn.completed"
            ));
        }
        let executed = executions.get(id).copied().unwrap_or(0);
        if ledger.is_some() && executed > tool_started {
            problems.push(format!(
                "tool ran {executed} times but only {tool_started} tool.started"
            ));
        }
        let interrupted = events
            .iter()
            .filter(|e| e["type"] == "tool.completed" && e.to_string().contains("interrupted"))
            .count();
        totals["events"] = json!(totals["events"].as_u64().unwrap_or(0) + events.len() as u64);
        totals["tool_started"] =
            json!(totals["tool_started"].as_u64().unwrap_or(0) + tool_started as u64);
        totals["tool_executions"] =
            json!(totals["tool_executions"].as_u64().unwrap_or(0) + executed as u64);
        totals["interrupted"] =
            json!(totals["interrupted"].as_u64().unwrap_or(0) + interrupted as u64);
        // Members a lead handed work to: each must have run exactly one turn
        // for its one input, however often the lead's call was retried.
        let members: HashSet<String> = events
            .iter()
            .filter(|e| e["type"] == "tool.completed")
            .flat_map(|e| session_ids(&e.to_string()))
            .filter(|member| member != id)
            .collect();
        for member in &members {
            let events = client
                .send("GET", &format!("/v1/sessions/{member}/events"), None)
                .await?;
            let events = events["data"].as_array().cloned().unwrap_or_default();
            let count = |t: &str| events.iter().filter(|e| e["type"] == t).count();
            let tool_started = count("tool.started");
            if count("input.message") != 1 || count("turn.completed") != 1 {
                problems.push(format!(
                    "member {member}: {} inputs, {} turns",
                    count("input.message"),
                    count("turn.completed")
                ));
            }
            let executed = executions.get(member).copied().unwrap_or(0);
            if ledger.is_some() && executed > tool_started {
                problems.push(format!(
                    "member {member}: tool ran {executed} times but only {tool_started} tool.started"
                ));
            }
            totals["tool_executions"] =
                json!(totals["tool_executions"].as_u64().unwrap_or(0) + executed as u64);
        }
        totals["members"] = json!(totals["members"].as_u64().unwrap_or(0) + members.len() as u64);
        if !problems.is_empty() {
            failures.push(json!({ "session": id, "problems": problems }));
        }
    }
    let report = json!({ "ok": failures.is_empty(), "totals": totals, "failures": failures.iter().take(20).collect::<Vec<_>>(), "failed_sessions": failures.len() });
    println!("{}", serde_json::to_string_pretty(&report)?);
    if !failures.is_empty() {
        std::process::exit(1);
    }
    Ok(())
}

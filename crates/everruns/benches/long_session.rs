//! Long-session benchmark: does a turn cost the same at turn 5 000 as at 5?
//!
//! Data plane v2 (`knowledge/execution/data-plane-v2/`) hosts sessions on
//! Engine and gates plan step 1 on this: throughput and p99 per turn late in
//! long sessions within 10% of the first turns, and few waited authority
//! operations per turn. `S` sessions run `T` llmsim turns each, all at once,
//! on [`MemoryHosting`]; the model answers instantly, so the numbers are
//! Engine's own per-turn cost.
//!
//! The report is split into tenths of the run. Each row has turns per
//! second, p50/p99 per turn and the authority writes and reads per turn
//! (see [`everruns::hosting::AuthorityCounts`]: on a replicated authority
//! each is a round trip to the leader, and each write also waits for a
//! quorum).
//!
//! Decisions:
//! - Every session runs concurrently, like a cell's sessions do, so tenths
//!   line up with session length.
//! - `cargo test` runs this as a smoke (`test = true`); `cargo bench` runs
//!   the full scale.
//!
//! Usage:
//!   cargo bench -p everruns --bench long_session
//!   cargo bench -p everruns --bench long_session -- --sessions 40 --turns 5000
//!   cargo bench -p everruns --bench long_session -- --summary out.jsonl

use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use everruns::hosting::{AgentBuild, AuthorityCounts, MemoryHosting};
use everruns::{Agent, CompactionConfig, Engine, LlmSimConfig, Model, SessionId};
use serde_json::json;

/// A turn may take this long before the run counts as broken.
const TURN_TIMEOUT: Duration = Duration::from_secs(60);
const TENTHS: usize = 10;

struct Options {
    smoke: bool,
    sessions: usize,
    turns: usize,
    moniker: String,
    summary: Option<PathBuf>,
}

impl Options {
    fn from_args() -> Self {
        let (mut smoke, mut bench) = (false, false);
        let (mut sessions, mut turns, mut moniker, mut summary) = (None, None, None, None);
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--smoke" => smoke = true,
                "--bench" => bench = true,
                "--sessions" => sessions = args.next().and_then(|v| v.parse().ok()),
                "--turns" => turns = args.next().and_then(|v| v.parse().ok()),
                "--moniker" => moniker = args.next(),
                "--summary" => summary = args.next().map(PathBuf::from),
                _ => {}
            }
        }
        let smoke = smoke || !bench;
        Self {
            smoke,
            sessions: sessions.unwrap_or(if smoke { 2 } else { 40 }),
            turns: turns.unwrap_or(if smoke { 20 } else { 1_000 }),
            moniker: moniker.unwrap_or_else(|| "local".into()),
            summary,
        }
    }
}

/// Where the run was when the `n`th tenth of its turns had finished.
#[derive(Clone, Copy)]
struct Mark {
    at: Instant,
    counts: AuthorityCounts,
}

struct Progress {
    done: usize,
    marks: Vec<Mark>,
    /// Per-turn latency, keyed by the tenth of the run the turn finished in.
    latencies: Vec<Vec<Duration>>,
}

fn percentile_ms(latencies: &[Duration], pct: f64) -> f64 {
    if latencies.is_empty() {
        return 0.0;
    }
    let mut sorted = latencies.to_vec();
    sorted.sort();
    let index = ((sorted.len() as f64 * pct).ceil() as usize).clamp(1, sorted.len()) - 1;
    sorted[index].as_secs_f64() * 1_000.0
}

fn main() {
    let opts = Options::from_args();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let total = opts.sessions * opts.turns;
    let profile = MemoryHosting::default();
    let build = AgentBuild::new("long-session", "bench-v1");
    let agent = Agent::builder()
        .instructions("You are concise.")
        // Answers of ~150 tokens in an 8k window: proactive compaction runs
        // every few dozen turns, as it would for a real model, so the run
        // measures a long session after compaction rather than an
        // ever-growing prompt.
        .model(Model::simulated_with_config(
            LlmSimConfig::lorem(150).with_context_window(8_000),
        ))
        .capability(CompactionConfig::new())
        .build()
        .expect("valid agent");
    let engine = Engine::builder()
        .hosting(profile.clone())
        .register(build.clone(), agent)
        .build();

    let progress = Arc::new(Mutex::new(Progress {
        done: 0,
        marks: Vec::with_capacity(TENTHS + 1),
        latencies: vec![Vec::new(); TENTHS],
    }));
    let started = runtime.block_on(async {
        let mut sessions = Vec::with_capacity(opts.sessions);
        for _ in 0..opts.sessions {
            sessions.push(
                engine
                    .create_registered(SessionId::new(), &build)
                    .await
                    .expect("create session"),
            );
        }
        let started = Instant::now();
        progress.lock().unwrap().marks.push(Mark {
            at: started,
            counts: profile.authority_counts(),
        });
        let tasks: Vec<_> = sessions
            .into_iter()
            .map(|session| {
                let progress = progress.clone();
                let profile = profile.clone();
                let turns = opts.turns;
                tokio::spawn(async move {
                    for turn in 0..turns {
                        let sent = Instant::now();
                        let result =
                            tokio::time::timeout(TURN_TIMEOUT, session.send_and_wait("hi"))
                                .await
                                .unwrap_or_else(|_| panic!("turn {turn} timed out"))
                                .unwrap_or_else(|error| panic!("turn {turn} failed: {error}"));
                        assert!(result.success, "turn {turn}: {result:?}");
                        let latency = sent.elapsed();
                        let mut progress = progress.lock().unwrap();
                        let tenth = (progress.done * TENTHS / total).min(TENTHS - 1);
                        progress.latencies[tenth].push(latency);
                        progress.done += 1;
                        if progress.done * TENTHS / total >= progress.marks.len() {
                            progress.marks.push(Mark {
                                at: Instant::now(),
                                counts: profile.authority_counts(),
                            });
                        }
                    }
                })
            })
            .collect();
        for task in tasks {
            task.await.expect("session task");
        }
        started
    });
    let elapsed = started.elapsed();

    let progress = progress.lock().unwrap();
    println!(
        "long_session ({}): {} sessions x {} turns, llmsim 150-token answers, 8k window, zero model latency",
        if opts.smoke { "smoke" } else { "full" },
        opts.sessions,
        opts.turns
    );
    println!(
        "{:<6} {:>10} {:>9} {:>9} {:>12} {:>11}",
        "tenth", "turns/s", "p50 ms", "p99 ms", "writes/turn", "reads/turn"
    );
    let mut tenths = Vec::new();
    for (index, window) in progress.marks.windows(2).enumerate() {
        let turns = progress.latencies[index].len().max(1) as f64;
        let seconds = window[1].at.duration_since(window[0].at).as_secs_f64();
        let writes = (window[1].counts.writes - window[0].counts.writes) as f64 / turns;
        let reads = (window[1].counts.reads - window[0].counts.reads) as f64 / turns;
        let (p50, p99) = (
            percentile_ms(&progress.latencies[index], 0.50),
            percentile_ms(&progress.latencies[index], 0.99),
        );
        let rate = turns / seconds.max(f64::EPSILON);
        println!(
            "{:<6} {:>10.1} {:>9.2} {:>9.2} {:>12.1} {:>11.1}",
            index + 1,
            rate,
            p50,
            p99,
            writes,
            reads
        );
        tenths.push(json!({
            "tenth": index + 1,
            "turns_per_sec": (rate * 10.0).round() / 10.0,
            "p50_ms": (p50 * 100.0).round() / 100.0,
            "p99_ms": (p99 * 100.0).round() / 100.0,
            "writes_per_turn": (writes * 10.0).round() / 10.0,
            "reads_per_turn": (reads * 10.0).round() / 10.0,
        }));
    }
    let counts = profile.authority_counts();
    println!(
        "total: {total} turns in {:.1} s ({:.1} turns/s), {:.1} writes and {:.1} reads per turn",
        elapsed.as_secs_f64(),
        total as f64 / elapsed.as_secs_f64(),
        counts.writes as f64 / total as f64,
        counts.reads as f64 / total as f64
    );
    assert_eq!(progress.done, total, "every turn finished");

    if let Some(path) = opts.summary {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .unwrap_or_else(|error| panic!("open {}: {error}", path.display()));
        let line = json!({
            "bench": "long_session",
            "moniker": opts.moniker,
            "smoke": opts.smoke,
            "sessions": opts.sessions,
            "turns_per_session": opts.turns,
            "turns_per_sec": (total as f64 / elapsed.as_secs_f64() * 10.0).round() / 10.0,
            "writes_per_turn": (counts.writes as f64 / total as f64 * 10.0).round() / 10.0,
            "reads_per_turn": (counts.reads as f64 / total as f64 * 10.0).round() / 10.0,
            "tenths": tenths,
        });
        writeln!(file, "{line}").expect("write summary");
    }
}

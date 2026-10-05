//! Turn backend benchmark: what a turn costs on each `TurnBackend`.
//!
//! Runs the same llmsim-scripted turns through the facade on the in-process
//! backend (the default) and on the durable memory backend
//! (`durable::Backend::memory`) at a few worker counts, with `C` slots sending
//! turns concurrently (each works through a few short sessions), and reports
//! per-turn latency (p50/p99 of `send_and_wait`) and turns per second. The
//! model answers instantly, so the numbers are the backend's own overhead:
//! persistence, queueing, checkpoints, wakeups and the ticket's wait for the
//! workflow to end.
//!
//! Decisions:
//! - Lives in the facade, not in `everruns-durable-engine` as first planned:
//!   it needs `Engine`, `InProcessRuntime` and both backends, and the facade
//!   already depends on the durable engine. A dev-dependency back onto the
//!   facade would build a second copy of the durable engine whose types do
//!   not meet the facade's.
//! - The durable PostgreSQL backend is not a framework backend yet (see
//!   `knowledge/framework/execution-backends.md`), so it is not measured.
//! - `cargo test` runs this binary as a smoke (it sets `test = true`); only
//!   `cargo bench` (which passes `--bench`) runs the full scale, so the facade
//!   CI job checks the bench still works without an extra cargo invocation.
//!
//! Scenarios: `text` (one reason, the answer) and `tool` (reason, a function
//! tool call, reason), each at concurrency 1, 16 and 64.
//!
//! Usage:
//!   cargo bench -p everruns --features durable --bench turn_backends
//!   cargo bench -p everruns --features durable --bench turn_backends -- --summary out.jsonl
//!   cargo bench -p everruns --features durable --bench turn_backends -- --smoke
//!
//! `--summary <file>` appends one JSON line per scenario in the shape of
//! `crates/durable/benches/baseline.jsonl` (`tasks` are turns, `e2e_*` the
//! per-turn latency), so `scripts/lib/durable-bench-compare.sh` can compare a
//! run with `crates/everruns/benches/turn_backends_baseline.jsonl`.

use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use everruns::{Agent, Engine, FunctionTool, LlmSimConfig, Model, ToolCall, durable};
use serde_json::json;

/// A turn may take this long before the run counts as broken.
const TURN_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug)]
enum Backend {
    InProcess,
    DurableMemory { workers: usize },
}

impl Backend {
    fn name(self) -> String {
        match self {
            Self::InProcess => "in_process".into(),
            Self::DurableMemory { workers } => format!("durable_memory_w{workers}"),
        }
    }

    fn engine(self) -> Engine {
        match self {
            Self::InProcess => Engine::new(),
            Self::DurableMemory { workers } => Engine::builder()
                .backend(durable::Backend::memory().workers(workers))
                .build(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Scenario {
    Text,
    Tool,
}

impl Scenario {
    fn name(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Tool => "tool",
        }
    }

    /// An agent for one session that runs `turns` turns of this scenario.
    fn agent(self, turns: usize) -> Agent {
        let builder = Agent::builder().instructions("You are concise.");
        match self {
            Self::Text => builder.model(Model::simulated("Sure.")),
            Self::Tool => {
                // llmsim cycles the sequence per reason: a call, then the
                // answer, once per turn. Unique ids keep each turn's call apart.
                let sequence = (0..turns)
                    .flat_map(|turn| {
                        [
                            vec![ToolCall {
                                id: format!("call_ping_{turn}"),
                                name: "ping".into(),
                                arguments: json!({}),
                            }],
                            vec![],
                        ]
                    })
                    .collect();
                let tool = FunctionTool::new(
                    "ping",
                    "Respond to a ping.",
                    json!({ "type": "object", "properties": {} }),
                    |_args: serde_json::Value| async move { Ok::<_, String>(json!({ "ok": true })) },
                );
                builder
                    .model(Model::simulated_with_config(
                        LlmSimConfig::fixed("pinged").with_tool_call_sequence(sequence),
                    ))
                    .tool(tool)
            }
        }
        .build()
        .expect("valid agent")
    }

    fn expected_tool_calls(self) -> usize {
        match self {
            Self::Text => 0,
            Self::Tool => 1,
        }
    }
}

struct Options {
    smoke: bool,
    moniker: String,
    summary: Option<PathBuf>,
}

impl Options {
    fn from_args() -> Self {
        let mut smoke = false;
        let mut bench = false;
        let mut moniker = None;
        let mut summary = None;
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--smoke" => smoke = true,
                "--bench" => bench = true,
                "--moniker" => moniker = args.next(),
                "--summary" => summary = args.next().map(PathBuf::from),
                _ => {}
            }
        }
        Self {
            // `cargo bench` passes `--bench`; anything else (`cargo test`,
            // a bare run) is the smoke unless asked otherwise.
            smoke: smoke || !bench,
            moniker: moniker.unwrap_or_else(|| "local".into()),
            summary,
        }
    }
}

/// One concurrency level: `concurrency` slots run at once, each sending the
/// turns of `sessions_per_slot` sessions one after another.
///
/// Sessions stay short (`turns_per_session`) on purpose: a turn's cost grows
/// with the history it carries on either backend (an in-process tool turn
/// went from ~6 ms to ~85 ms p50 over a 100-turn session), and that growth
/// would drown the backend overhead this bench is after.
#[derive(Clone, Copy)]
struct Load {
    concurrency: usize,
    sessions_per_slot: usize,
    turns_per_session: usize,
}

impl Load {
    const fn new(concurrency: usize, sessions_per_slot: usize, turns_per_session: usize) -> Self {
        Self {
            concurrency,
            sessions_per_slot,
            turns_per_session,
        }
    }

    fn turns(self) -> usize {
        self.concurrency * self.sessions_per_slot * self.turns_per_session
    }
}

struct Measured {
    turns: usize,
    elapsed: Duration,
    latencies: Vec<Duration>,
}

impl Measured {
    fn turns_per_sec(&self) -> f64 {
        self.turns as f64 / self.elapsed.as_secs_f64()
    }

    fn percentile_ms(&self, pct: f64) -> f64 {
        let mut sorted = self.latencies.clone();
        sorted.sort();
        let index = ((sorted.len() as f64 * pct).ceil() as usize).clamp(1, sorted.len()) - 1;
        sorted[index].as_secs_f64() * 1_000.0
    }
}

/// Run `load` on a fresh engine and time every `send_and_wait`.
async fn run(backend: Backend, scenario: Scenario, load: Load) -> Measured {
    let engine = backend.engine();
    // Sessions exist before the clock starts; creating one is not a turn.
    let slots: Vec<Vec<_>> = (0..load.concurrency)
        .map(|_| {
            (0..load.sessions_per_slot)
                .map(|_| engine.create(scenario.agent(load.turns_per_session)))
                .collect()
        })
        .collect();
    let started = Instant::now();
    let tasks: Vec<_> = slots
        .into_iter()
        .map(|sessions| {
            tokio::spawn(async move {
                let mut latencies =
                    Vec::with_capacity(load.sessions_per_slot * load.turns_per_session);
                for session in sessions {
                    for turn in 0..load.turns_per_session {
                        let sent = Instant::now();
                        let result = tokio::time::timeout(
                            TURN_TIMEOUT,
                            session.send_and_wait("ping please"),
                        )
                        .await
                        .unwrap_or_else(|_| panic!("{backend:?} {scenario:?}: turn timed out"))
                        .unwrap_or_else(|error| {
                            panic!("{backend:?} {scenario:?}: turn {turn} failed: {error}")
                        });
                        latencies.push(sent.elapsed());
                        assert!(result.success, "{backend:?} {scenario:?}: {result:?}");
                        assert_eq!(
                            result.tool_calls,
                            scenario.expected_tool_calls(),
                            "{backend:?} {scenario:?}: {result:?}"
                        );
                    }
                }
                latencies
            })
        })
        .collect();
    let mut latencies = Vec::with_capacity(load.turns());
    for task in tasks {
        latencies.extend(task.await.expect("session task"));
    }
    let elapsed = started.elapsed();
    drop(engine);
    Measured {
        turns: latencies.len(),
        elapsed,
        latencies,
    }
}

fn main() {
    let opts = Options::from_args();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");

    let (backends, loads) = if opts.smoke {
        (
            vec![Backend::InProcess, Backend::DurableMemory { workers: 4 }],
            vec![Load::new(1, 1, 3), Load::new(4, 1, 2)],
        )
    } else {
        // Enough turns per row for a stable p99, in under half a minute.
        (
            vec![
                Backend::InProcess,
                Backend::DurableMemory { workers: 4 },
                Backend::DurableMemory { workers: 16 },
            ],
            vec![
                Load::new(1, 40, 5),
                Load::new(16, 4, 5),
                Load::new(64, 2, 5),
            ],
        )
    };

    println!(
        "turn_backends ({}): llmsim, zero model latency",
        if opts.smoke { "smoke" } else { "full" }
    );
    println!(
        "{:<8} {:<20} {:>5} {:>6} {:>10} {:>10} {:>10}",
        "scenario", "backend", "conc", "turns", "turns/s", "p50 ms", "p99 ms"
    );
    let mut lines = Vec::new();
    for scenario in [Scenario::Text, Scenario::Tool] {
        for &backend in &backends {
            // Warm the backend's code paths and allocator once per pairing.
            runtime.block_on(run(backend, scenario, Load::new(1, 1, 2)));
            for &load in &loads {
                let concurrency = load.concurrency;
                let measured = runtime.block_on(run(backend, scenario, load));
                let (p50, p99) = (measured.percentile_ms(0.50), measured.percentile_ms(0.99));
                println!(
                    "{:<8} {:<20} {:>5} {:>6} {:>10.1} {:>10.2} {:>10.2}",
                    scenario.name(),
                    backend.name(),
                    concurrency,
                    measured.turns,
                    measured.turns_per_sec(),
                    p50,
                    p99
                );
                lines.push(json!({
                    "bench": "turn_backends",
                    "scenario": format!("{}_{}_c{concurrency}", scenario.name(), backend.name()),
                    "moniker": opts.moniker,
                    "smoke": opts.smoke,
                    "tasks": measured.turns,
                    "tasks_per_sec": (measured.turns_per_sec() * 10.0).round() / 10.0,
                    // A turn has no separate start: `send_and_wait` is
                    // submit to settle, so `s2s_*` repeats `e2e_*` for the
                    // compare script, which reports `s2s_p99_ms`.
                    "s2s_p50_ms": (p50 * 100.0).round() / 100.0,
                    "s2s_p99_ms": (p99 * 100.0).round() / 100.0,
                    "e2e_p50_ms": (p50 * 100.0).round() / 100.0,
                    "e2e_p99_ms": (p99 * 100.0).round() / 100.0,
                }));
            }
        }
    }

    if let Some(path) = opts.summary {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .unwrap_or_else(|error| panic!("open {}: {error}", path.display()));
        for line in lines {
            writeln!(file, "{line}").expect("write summary");
        }
    }
}

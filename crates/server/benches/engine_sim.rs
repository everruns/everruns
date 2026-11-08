//! Engine simulator: many agents through the real server and several gRPC
//! workers in one process, with simulated model and tool waits.
//!
//! The question it answers is how many turns workers hold at once, and what
//! holding them costs, without a deploy. It starts the production server in
//! process (`ServerAppBuilder`, PostgreSQL from `DATABASE_URL`) and
//! `--workers` standalone workers (`WorkerAppBuilder`, each over the server's
//! gRPC port like the `everruns-worker` binary). Each session then sends
//! `--messages` messages whose llmsim scenario marker scripts the turn:
//! `luna-agent` plays four model calls timed like GPT-6 Luna with file tool
//! rounds between them, `long-tool` two 25 s shell commands. Almost all of a
//! turn is waiting, as in production.
//!
//! Reported:
//! - `held`: running turns, from live leases sampled every second (peak in
//!   total and on the busiest worker). Turn admission decides it
//!   (`crates/worker/src/actor_runtime/admission.rs`).
//! - `pickup`: `input.message` to `turn.started`; `turn`: to `turn.completed`,
//!   by scenario, from the events' own timestamps.
//! - `db stmts/turn`: statements PostgreSQL executed per turn
//!   (`pg_stat_statements`, as in `turn_latency`; the bench's own queries are
//!   left out: each carries an `engine_sim` comment).
//! - `rss/held turn`: how much the process's resident memory grew over its
//!   idle baseline, per turn held at the peak. Server and workers share the
//!   process, so this is an upper bound on a worker's memory per turn, the
//!   figure turn admission's ceiling assumes (`ASSUMED_TURN_MEMORY`).
//! - `lag`: how late a 50 ms timer on the shared runtime fires, p50 and max.
//!
//! Decisions:
//! - Nothing reads SSE: completion is read from the events table by one
//!   query per 200 ms for every session, so the driver costs the server
//!   nothing per session and thousands of sessions fit on one machine.
//! - Server, workers and driver share one process and runtime. The
//!   simulation is about the engine's own limits (admission, leases, the
//!   database), not the network; the local stress bench
//!   (`scripts/stress/run.py`) runs separate processes when that matters.
//! - A bench target (`harness = false`), never run by `cargo test`, because
//!   it needs a database and process-wide environment. The weekly
//!   `durable-bench.yml` runs it; `--smoke` keeps it working in a minute.
//!
//! Usage:
//!   DATABASE_URL=postgres://... cargo bench -p everruns-server --bench engine_sim
//!   ... -- --sessions 1000 --messages 2 --mix luna-agent=3,long-tool=1
//!   ... -- --ceiling 20          # WORKER_MAX_RUNNING_TURNS per worker (0: from memory)
//!   ... -- --workers 3 --speed 0.5 --ramp-secs 20 --pool-max 50
//!   ... -- --top 40            # the statements run most per turn
//!   ... -- --smoke               # a few sessions, sped up, about a minute
//!   ... -- --summary out.jsonl --moniker <name>

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Write as _;
use std::net::TcpListener;
use std::path::PathBuf;
use std::str::FromStr as _;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use everruns_contracts::typed_id::SessionId;
use everruns_server::app_builder::ServerAppBuilder;
use everruns_server::server::ServerConfig;
use everruns_worker::actor_runtime::admission::{
    AdmissionConfig, DEFAULT_MAX_RUNTIME_LAG, auto_ceiling, process_resident_memory,
};
use everruns_worker::{TaskWorkerConfig, WorkerAppBuilder};
use serde_json::{Value, json};
use tokio::sync::{Mutex, Notify};
use uuid::Uuid;

const WORKER_TOKEN: &str = "server-engine-sim-bench";
/// `key_id:base64` of 32 fixed bytes ("everruns-engine-simulator-bench!").
const BENCH_ENCRYPTION_KEY: &str = "kek-sim-v1:ZXZlcnJ1bnMtZW5naW5lLXNpbXVsYXRvci1iZW5jaCE=";
/// What `luna-agent`'s file tools read; the same notes `scripts/load-test.py`
/// seeds.
const RESEARCH_NOTES_LINE: &str = "The runtime stores session events durably, streams replies, and distributes turns across workers.\n";

struct Options {
    smoke: bool,
    sessions: usize,
    messages: usize,
    mix: Vec<(String, f64)>,
    speed: f64,
    ramp: Duration,
    think: Duration,
    workers: usize,
    ceiling: usize,
    turn_timeout: Duration,
    pool_max: Option<u32>,
    top: usize,
    moniker: String,
    summary: Option<PathBuf>,
}

impl Options {
    fn from_args() -> Self {
        let mut opts = Self {
            smoke: false,
            sessions: 400,
            messages: 2,
            mix: parse_mix("luna-agent=3,long-tool=1"),
            speed: 1.0,
            ramp: Duration::from_secs(20),
            think: Duration::from_secs(1),
            workers: 3,
            ceiling: 0,
            turn_timeout: Duration::from_secs(300),
            pool_max: None,
            top: 0,
            moniker: "local".into(),
            summary: None,
        };
        let mut args = std::env::args().skip(1);
        let mut smoke_overrides = true;
        while let Some(arg) = args.next() {
            let mut value = || args.next().unwrap_or_else(|| panic!("{arg} needs a value"));
            match arg.as_str() {
                "--smoke" => opts.smoke = true,
                "--sessions" => {
                    opts.sessions = value().parse().expect("sessions");
                    smoke_overrides = false;
                }
                "--messages" => opts.messages = value().parse().expect("messages"),
                "--mix" => opts.mix = parse_mix(&value()),
                "--speed" => opts.speed = value().parse().expect("speed"),
                "--ramp-secs" => {
                    opts.ramp = Duration::from_secs_f64(value().parse().expect("ramp"))
                }
                "--think-secs" => {
                    opts.think = Duration::from_secs_f64(value().parse().expect("think"))
                }
                "--workers" => opts.workers = value().parse().expect("workers"),
                "--ceiling" => opts.ceiling = value().parse().expect("ceiling"),
                "--turn-timeout-secs" => {
                    opts.turn_timeout = Duration::from_secs(value().parse().expect("timeout"))
                }
                "--pool-max" => opts.pool_max = Some(value().parse().expect("pool max")),
                "--top" => opts.top = value().parse().expect("top"),
                "--moniker" => opts.moniker = value(),
                "--summary" => opts.summary = Some(PathBuf::from(value())),
                // `cargo bench` passes `--bench`.
                _ => {}
            }
        }
        if opts.smoke && smoke_overrides {
            // The shell commands of `long-tool` do not scale with `speed`.
            opts.sessions = 6;
            opts.messages = 1;
            opts.mix = parse_mix("luna-agent=1");
            opts.speed = 0.05;
            opts.ramp = Duration::from_millis(500);
            opts.think = Duration::ZERO;
            opts.turn_timeout = Duration::from_secs(60);
        }
        opts
    }
}

fn parse_mix(text: &str) -> Vec<(String, f64)> {
    text.split(',')
        .filter(|part| !part.trim().is_empty())
        .map(|part| match part.split_once('=') {
            Some((name, weight)) => (name.trim().to_owned(), weight.parse().expect("weight")),
            None => (part.trim().to_owned(), 1.0),
        })
        .collect()
}

fn main() {
    let opts = Options::from_args();
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        eprintln!("engine_sim needs DATABASE_URL (a PostgreSQL database it may migrate)");
        std::process::exit(2);
    };
    if std::env::var_os("RUST_LOG").is_some() {
        tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .with_writer(std::io::stderr)
            .init();
    }
    let http_port = free_port();
    let grpc_port = free_port();
    configure_environment(&database_url, &opts);

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let (line, ok) = runtime.block_on(run(&opts, &database_url, http_port, grpc_port));

    if let Some(path) = &opts.summary {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap_or_else(|error| panic!("open {}: {error}", path.display()));
        writeln!(file, "{line}").expect("write summary");
    }
    // The server and workers run until killed; this process is done.
    std::process::exit(if ok { 0 } else { 1 });
}

fn configure_environment(database_url: &str, opts: &Options) {
    // SAFETY: called from `main` before the Tokio runtime or any other thread
    // exists, so nothing can observe the environment while it changes.
    unsafe {
        std::env::set_var("DATABASE_URL", database_url);
        std::env::set_var("DEPLOYMENT_GRADE", "dev");
        std::env::set_var("AUTH_MODE", "none");
        std::env::set_var("WORKER_GRPC_AUTH_TOKEN", WORKER_TOKEN);
        // gRPC workers store encrypted checkpoints through the server; a fixed
        // throwaway key, the bench database holds only llmsim data.
        std::env::set_var("SECRETS_ENCRYPTION_KEY", BENCH_ENCRYPTION_KEY);
        // One client on 127.0.0.1 creates every session: the per-IP and
        // per-org limits would measure the limiter, not the engine.
        std::env::set_var("RATE_LIMIT_API_REQUESTS_PER_MINUTE", "0");
        std::env::set_var("RATE_LIMIT_ORG_SESSION_CREATE_PER_MINUTE", "1000000");
        std::env::set_var("RATE_LIMIT_ORG_TOOL_CALLS_PER_MINUTE", "1000000");
        if let Some(pool_max) = opts.pool_max {
            std::env::set_var("DATABASE_POOL_MAX", pool_max.to_string());
        }
    }
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .expect("bind a local port")
        .port()
}

async fn run(opts: &Options, database_url: &str, http_port: u16, grpc_port: u16) -> (Value, bool) {
    let base = format!("http://127.0.0.1:{http_port}");
    let config = ServerConfig {
        dev_mode: false,
        no_migrations: false,
        api_prefix: String::new(),
        cors_origins: vec![],
        addr: format!("127.0.0.1:{http_port}"),
        grpc_addr: format!("127.0.0.1:{grpc_port}"),
    };
    let server = tokio::spawn(ServerAppBuilder::new(config).run());
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .pool_max_idle_per_host(256)
        .build()
        .expect("http client");
    wait_for_health(&client, &base, &server).await;

    let ceiling = if opts.ceiling == 0 {
        auto_ceiling(None)
    } else {
        opts.ceiling
    };
    let holders: Vec<String> = (0..opts.workers)
        .map(|i| format!("engine-sim-worker-{i}-{}", Uuid::now_v7().simple()))
        .collect();
    let workers: Vec<_> = holders
        .iter()
        .map(|holder| {
            let config = TaskWorkerConfig {
                worker_id: holder.clone(),
                grpc_address: format!("127.0.0.1:{grpc_port}"),
                turn_admission: AdmissionConfig {
                    ceiling,
                    memory_limit: None,
                    max_runtime_lag: DEFAULT_MAX_RUNTIME_LAG,
                },
                ..TaskWorkerConfig::default()
            };
            tokio::spawn(WorkerAppBuilder::new(config).run())
        })
        .collect();

    let db = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(database_url)
        .await
        .expect("bench database connection");
    let stats = DbStats::connect(&db).await;
    let agent = create_agent(&client, &base).await;

    // One session first: the first on a fresh database creates the user's
    // principals and warms every cache, and hundreds racing to do it would
    // measure that instead.
    let warmup = create_session(&client, &base, &agent).await;
    let completions = Completions::new(db.clone());
    let poller = tokio::spawn(completions.clone().poll());
    completions.track(&[warmup.uuid]).await;
    let warm_text = message_text(&opts.mix[0].0, 0, 0, opts.speed);
    send(&client, &base, &warmup, &warm_text).await;
    completions
        .wait(warmup.uuid, 1, Duration::from_secs(120))
        .await
        .expect("the warm-up turn completes");

    println!(
        "engine_sim ({}): server + {} gRPC workers in process, ceiling {} per worker, {} sessions x {} messages, mix {:?}, speed {}",
        if opts.smoke { "smoke" } else { "full" },
        opts.workers,
        ceiling,
        opts.sessions,
        opts.messages,
        opts.mix,
        opts.speed,
    );

    // Create and seed the sessions before the clock starts.
    let mut sessions = Vec::with_capacity(opts.sessions);
    let creating = Arc::new(tokio::sync::Semaphore::new(32));
    let mut created = tokio::task::JoinSet::new();
    for _ in 0..opts.sessions {
        let (client, base, agent, creating) = (
            client.clone(),
            base.clone(),
            agent.clone(),
            creating.clone(),
        );
        created.spawn(async move {
            let _permit = creating.acquire().await.expect("semaphore");
            create_session(&client, &base, &agent).await
        });
    }
    while let Some(session) = created.join_next().await {
        sessions.push(session.expect("create session"));
    }
    let ids: Vec<Uuid> = sessions.iter().map(|session| session.uuid).collect();
    completions.track(&ids).await;

    let baseline_rss = process_resident_memory();
    let lag = Arc::new(Mutex::new(Vec::<f64>::new()));
    let lag_probe = tokio::spawn(probe_lag(lag.clone()));
    let held = Arc::new(Mutex::new(HeldPeak::default()));
    let sampler = tokio::spawn(sample_held(db.clone(), held.clone(), holders.clone()));
    if let Some(stats) = &stats {
        stats.reset().await;
    }

    let started = Instant::now();
    let stuck = Arc::new(AtomicUsize::new(0));
    let failed = Arc::new(AtomicUsize::new(0));
    let names: Vec<String> = opts.mix.iter().map(|(name, _)| name.clone()).collect();
    let weights: Vec<f64> = opts.mix.iter().map(|(_, weight)| *weight).collect();
    let mut drivers = tokio::task::JoinSet::new();
    for (index, session) in sessions.into_iter().enumerate() {
        let (client, base, completions) = (client.clone(), base.clone(), completions.clone());
        let (stuck, failed) = (stuck.clone(), failed.clone());
        let (names, weights) = (names.clone(), weights.clone());
        let delay = opts
            .ramp
            .mul_f64(index as f64 / opts.sessions.max(1) as f64);
        let (messages, speed, think, timeout) =
            (opts.messages, opts.speed, opts.think, opts.turn_timeout);
        drivers.spawn(async move {
            tokio::time::sleep(delay).await;
            for message in 0..messages {
                let scenario = pick(&names, &weights, index * 31 + message);
                let text = message_text(scenario, index, message, speed);
                send(&client, &base, &session, &text).await;
                match completions
                    .wait(session.uuid, message as u32 + 1, timeout)
                    .await
                {
                    Ok(()) => {}
                    Err(Outcome::Failed) => {
                        failed.fetch_add(1, Ordering::SeqCst);
                        return;
                    }
                    Err(Outcome::Stuck) => {
                        stuck.fetch_add(1, Ordering::SeqCst);
                        return;
                    }
                }
                tokio::time::sleep(think).await;
            }
        });
    }
    while drivers.join_next().await.is_some() {}
    let elapsed = started.elapsed();
    sampler.abort();
    lag_probe.abort();
    poller.abort();

    let turns = turn_timings(&db, &ids).await;
    let db_per_turn = match &stats {
        Some(stats) => Some(stats.per_turn(turns.len()).await),
        None => None,
    };
    if let (Some(stats), true) = (&stats, opts.top > 0) {
        for line in stats.top(turns.len(), opts.top).await {
            println!("    {line}");
        }
    }
    let held = held.lock().await.clone();
    let mut lag = lag.lock().await.clone();
    let (stuck, failed) = (stuck.load(Ordering::SeqCst), failed.load(Ordering::SeqCst));

    let rss_per_turn = match (baseline_rss, held.rss_at_peak) {
        (Some(base), Some(peak)) if held.total > 0 => {
            Some(peak.saturating_sub(base) as f64 / held.total as f64 / (1024.0 * 1024.0))
        }
        _ => None,
    };
    let mut pickup: Vec<f64> = turns.iter().map(|turn| turn.pickup_ms).collect();
    println!(
        "turns {} in {:.1}s ({:.1}/s), failed {failed}, stuck {stuck}",
        turns.len(),
        elapsed.as_secs_f64(),
        turns.len() as f64 / elapsed.as_secs_f64()
    );
    println!(
        "held turns: peak {} total, {} on the busiest worker (ceiling {ceiling})",
        held.total, held.busiest
    );
    println!(
        "pickup ms: p50 {:.0}, p95 {:.0}, p99 {:.0}, max {:.0}",
        percentile(&mut pickup, 0.50),
        percentile(&mut pickup, 0.95),
        percentile(&mut pickup, 0.99),
        percentile(&mut pickup, 1.0)
    );
    let mut by_scenario: HashMap<&str, Vec<f64>> = HashMap::new();
    for turn in &turns {
        by_scenario
            .entry(turn.scenario.as_str())
            .or_default()
            .push(turn.turn_ms);
    }
    let mut scenarios = serde_json::Map::new();
    for (scenario, durations) in &mut by_scenario {
        println!(
            "turn ms [{scenario}]: n {}, p50 {:.0}, p99 {:.0}",
            durations.len(),
            percentile(durations, 0.50),
            percentile(durations, 0.99)
        );
        scenarios.insert(
            (*scenario).to_owned(),
            json!({
                "turns": durations.len(),
                "turn_p50_ms": round2(percentile(durations, 0.50)),
                "turn_p99_ms": round2(percentile(durations, 0.99)),
            }),
        );
    }
    println!(
        "db stmts/turn: {}",
        db_per_turn.map_or_else(|| "n/a".to_owned(), |v| format!("{v:.1}"))
    );
    println!(
        "rss: baseline {} MiB, at peak {} MiB, {} MiB per held turn",
        mib(baseline_rss),
        mib(held.rss_at_peak),
        rss_per_turn.map_or_else(|| "n/a".to_owned(), |v| format!("{v:.2}"))
    );
    println!(
        "runtime lag ms: p50 {:.1}, p99 {:.1}, max {:.1}",
        percentile(&mut lag, 0.50),
        percentile(&mut lag, 0.99),
        percentile(&mut lag, 1.0)
    );

    for worker in workers {
        worker.abort();
    }
    server.abort();
    let line = json!({
        "bench": "server_engine_sim",
        "moniker": opts.moniker,
        "smoke": opts.smoke,
        "workers": opts.workers,
        "ceiling": ceiling,
        "sessions": opts.sessions,
        "messages": opts.messages,
        "speed": opts.speed,
        "tasks": turns.len(),
        "tasks_per_sec": round2(turns.len() as f64 / elapsed.as_secs_f64()),
        "failed": failed,
        "stuck": stuck,
        "held_peak": held.total,
        "held_peak_worker": held.busiest,
        "s2s_p50_ms": round2(percentile(&mut pickup, 0.50)),
        "s2s_p99_ms": round2(percentile(&mut pickup, 0.99)),
        "scenarios": scenarios,
        "db_statements_per_turn": db_per_turn.map(round2),
        "rss_mib_per_held_turn": rss_per_turn.map(round2),
        "lag_p99_ms": round2(percentile(&mut lag, 0.99)),
    });
    let ok = failed == 0 && stuck == 0 && turns.len() == opts.sessions * opts.messages;
    (line, ok)
}

fn mib(bytes: Option<u64>) -> String {
    bytes.map_or_else(|| "n/a".to_owned(), |b| (b / (1024 * 1024)).to_string())
}

/// The scenario for one message, from the weighted mix, deterministically.
fn pick<'a>(names: &'a [String], weights: &[f64], seed: usize) -> &'a str {
    let total: f64 = weights.iter().sum();
    // A fixed spread over [0, 1) (golden-ratio steps), so runs are comparable.
    let mut point = ((seed as f64 * 0.618_033_988_75).fract()) * total;
    for (name, weight) in names.iter().zip(weights) {
        if point < *weight {
            return name;
        }
        point -= weight;
    }
    names.last().expect("a scenario mix")
}

fn message_text(scenario: &str, session: usize, message: usize, speed: f64) -> String {
    let speed = if speed == 1.0 {
        String::new()
    } else {
        format!(" speed={speed}")
    };
    format!("Simulated message {message}. [[llmsim:{scenario} seed={session}{speed}]]")
}

/// Peak running turns over the run.
#[derive(Clone, Default)]
struct HeldPeak {
    total: usize,
    busiest: usize,
    rss_at_peak: Option<u64>,
}

/// Every second: live leases per holder, which are the turns running now.
async fn sample_held(db: sqlx::PgPool, peak: Arc<Mutex<HeldPeak>>, holders: Vec<String>) {
    let query = "/* engine_sim */ SELECT lease_holder, COUNT(*)::int8 FROM session_actors
         WHERE lease_holder = ANY($1) AND lease_expires_at > NOW() GROUP BY lease_holder";
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let Ok(rows) = sqlx::query_as::<_, (String, i64)>(query)
            .bind(&holders)
            .fetch_all(&db)
            .await
        else {
            continue;
        };
        let total: i64 = rows.iter().map(|(_, held)| held).sum();
        let busiest = rows.iter().map(|(_, held)| *held).max().unwrap_or(0);
        let mut peak = peak.lock().await;
        peak.busiest = peak.busiest.max(busiest as usize);
        if total as usize > peak.total {
            peak.total = total as usize;
            peak.rss_at_peak = process_resident_memory();
        }
    }
}

/// How late a 50 ms timer fires on the shared runtime, in ms.
async fn probe_lag(samples: Arc<Mutex<Vec<f64>>>) {
    const TICK: Duration = Duration::from_millis(50);
    loop {
        let start = Instant::now();
        tokio::time::sleep(TICK).await;
        let late = start.elapsed().saturating_sub(TICK);
        samples.lock().await.push(late.as_secs_f64() * 1_000.0);
    }
}

#[derive(Debug)]
enum Outcome {
    Failed,
    Stuck,
}

/// Turn ends per session, read from the events table by one query for all of
/// them every 200 ms.
#[derive(Clone)]
struct Completions {
    db: sqlx::PgPool,
    sessions: Arc<Mutex<Vec<Uuid>>>,
    ended: Arc<Mutex<HashMap<Uuid, (u32, u32)>>>,
    changed: Arc<Notify>,
}

impl Completions {
    fn new(db: sqlx::PgPool) -> Self {
        Self {
            db,
            sessions: Arc::default(),
            ended: Arc::default(),
            changed: Arc::new(Notify::new()),
        }
    }

    async fn track(&self, ids: &[Uuid]) {
        self.sessions.lock().await.extend_from_slice(ids);
    }

    async fn poll(self) {
        let query = "/* engine_sim */ SELECT session_id,
                COUNT(*) FILTER (WHERE event_type = 'turn.completed')::int8,
                COUNT(*) FILTER (WHERE event_type = 'turn.failed')::int8
             FROM events WHERE session_id = ANY($1)
               AND event_type IN ('turn.completed', 'turn.failed')
             GROUP BY session_id";
        loop {
            let ids = self.sessions.lock().await.clone();
            if !ids.is_empty()
                && let Ok(rows) = sqlx::query_as::<_, (Uuid, i64, i64)>(query)
                    .bind(&ids)
                    .fetch_all(&self.db)
                    .await
            {
                let mut ended = self.ended.lock().await;
                for (id, completed, failed) in rows {
                    ended.insert(id, (completed as u32, failed as u32));
                }
                drop(ended);
                self.changed.notify_waiters();
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    /// Until `session` completed `turns` turns.
    async fn wait(&self, session: Uuid, turns: u32, timeout: Duration) -> Result<(), Outcome> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let changed = self.changed.notified();
            let (completed, failed) = self
                .ended
                .lock()
                .await
                .get(&session)
                .copied()
                .unwrap_or_default();
            if failed > 0 {
                return Err(Outcome::Failed);
            }
            if completed >= turns {
                return Ok(());
            }
            if tokio::time::timeout_at(deadline, changed).await.is_err() {
                return Err(Outcome::Stuck);
            }
        }
    }
}

struct TurnTiming {
    scenario: String,
    pickup_ms: f64,
    turn_ms: f64,
}

/// Each input's pickup (`turn.started`) and end (`turn.completed`).
async fn turn_timings(db: &sqlx::PgPool, ids: &[Uuid]) -> Vec<TurnTiming> {
    let query = "/* engine_sim */ SELECT i.data::text,
            EXTRACT(EPOCH FROM (s.ts - i.ts))::float8 * 1000,
            EXTRACT(EPOCH FROM (c.ts - i.ts))::float8 * 1000
         FROM events i
         JOIN LATERAL (SELECT ts FROM events s WHERE s.session_id = i.session_id
             AND s.event_type = 'turn.started' AND s.sequence > i.sequence
             ORDER BY s.sequence LIMIT 1) s ON true
         JOIN LATERAL (SELECT ts FROM events c WHERE c.session_id = i.session_id
             AND c.event_type = 'turn.completed' AND c.sequence > i.sequence
             ORDER BY c.sequence LIMIT 1) c ON true
         WHERE i.session_id = ANY($1) AND i.event_type = 'input.message'";
    sqlx::query_as::<_, (String, f64, f64)>(query)
        .bind(ids)
        .fetch_all(db)
        .await
        .expect("read turn timings")
        .into_iter()
        .map(|(data, pickup_ms, turn_ms)| TurnTiming {
            scenario: data
                .split("[[llmsim:")
                .nth(1)
                .and_then(|rest| rest.split([' ', ']']).next())
                .unwrap_or("unknown")
                .to_owned(),
            pickup_ms,
            turn_ms,
        })
        .collect()
}

/// Statements per turn from `pg_stat_statements`, as in `turn_latency`.
struct DbStats {
    pool: sqlx::PgPool,
}

impl DbStats {
    async fn connect(pool: &sqlx::PgPool) -> Option<Self> {
        let ready = sqlx::query("CREATE EXTENSION IF NOT EXISTS pg_stat_statements")
            .execute(pool)
            .await
            .is_ok()
            && sqlx::query("SELECT 1 FROM pg_stat_statements LIMIT 1")
                .execute(pool)
                .await
                .is_ok();
        if !ready {
            println!("note: pg_stat_statements is not loaded, so DB work per turn is not reported");
            return None;
        }
        Some(Self { pool: pool.clone() })
    }

    async fn reset(&self) {
        sqlx::query("SELECT pg_stat_statements_reset()")
            .execute(&self.pool)
            .await
            .expect("reset pg_stat_statements");
    }

    /// The `top` statements by calls per turn, as `calls/turn  query`.
    async fn top(&self, turns: usize, top: usize) -> Vec<String> {
        let rows: Vec<(f64, f64, String)> = sqlx::query_as(
            "SELECT calls::float8, total_exec_time, query FROM pg_stat_statements
             WHERE dbid = (SELECT oid FROM pg_database WHERE datname = current_database())
               AND query NOT ILIKE '%pg_stat_statements%'
               AND query NOT LIKE '%engine_sim%'
             ORDER BY calls DESC LIMIT $1",
        )
        .bind(top as i64)
        .fetch_all(&self.pool)
        .await
        .expect("read top statements");
        let turns = turns.max(1) as f64;
        rows.into_iter()
            .map(|(calls, total_ms, query)| {
                let query: String = query.split_whitespace().collect::<Vec<_>>().join(" ");
                let query: String = query.chars().take(140).collect();
                format!(
                    "{:>7.1} calls/turn {:>7.2} ms/turn  {query}",
                    calls / turns,
                    total_ms / turns
                )
            })
            .collect()
    }

    async fn per_turn(&self, turns: usize) -> f64 {
        let (calls,): (f64,) = sqlx::query_as(
            "SELECT COALESCE(SUM(calls), 0)::float8 FROM pg_stat_statements
             WHERE dbid = (SELECT oid FROM pg_database WHERE datname = current_database())
               AND query NOT ILIKE '%pg_stat_statements%'
               AND query NOT LIKE '%engine_sim%'",
        )
        .fetch_one(&self.pool)
        .await
        .expect("read pg_stat_statements");
        calls / turns.max(1) as f64
    }
}

async fn wait_for_health(
    client: &reqwest::Client,
    base: &str,
    server: &tokio::task::JoinHandle<anyhow::Result<()>>,
) {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        assert!(
            !server.is_finished(),
            "the server exited during startup (see its logs with RUST_LOG)"
        );
        if let Ok(response) = client.get(format!("{base}/health")).send().await
            && response.status().is_success()
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the server did not become healthy"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn post(client: &reqwest::Client, url: String, body: Value) -> Value {
    let response = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .unwrap_or_else(|error| panic!("POST {url}: {error}"));
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    assert!(status.is_success(), "POST {url}: {status}: {text}");
    serde_json::from_str(&text).unwrap_or_else(|error| panic!("POST {url}: {error}: {text}"))
}

/// An agent on an llmsim model; the scenario markers time every call.
async fn create_agent(client: &reqwest::Client, base: &str) -> String {
    let provider = post(
        client,
        format!("{base}/v1/providers"),
        json!({"name": format!("Engine simulator {}", Uuid::now_v7()), "provider_type": "llmsim"}),
    )
    .await;
    let provider_id = provider["id"].as_str().expect("provider id");
    let model = post(
        client,
        format!("{base}/v1/providers/{provider_id}/models"),
        json!({
            "model_id": format!("llmsim-sim-{}", Uuid::now_v7().simple()),
            "display_name": "Engine simulator",
            "enabled": true
        }),
    )
    .await;
    let agent = post(
        client,
        format!("{base}/v1/agents"),
        json!({
            "name": format!("engine-sim-{}", Uuid::now_v7().simple()),
            "system_prompt": "Respond concisely.",
            "default_model_id": model["id"],
        }),
    )
    .await;
    agent["public_id"]
        .as_str()
        .or_else(|| agent["id"].as_str())
        .expect("agent id")
        .to_owned()
}

struct Session {
    id: String,
    uuid: Uuid,
}

/// A session on the generic harness (file and shell tools), with the notes
/// `luna-agent` reads.
async fn create_session(client: &reqwest::Client, base: &str, agent: &str) -> Session {
    let session = post(
        client,
        format!("{base}/v1/sessions"),
        json!({"harness_name": "generic", "agent_id": agent, "title": "Engine simulator"}),
    )
    .await;
    let id = session["id"].as_str().expect("session id").to_owned();
    let uuid = SessionId::from_str(&id).expect("a session id").uuid();
    post(
        client,
        format!("{base}/v1/sessions/{id}/fs/load-test/research.md"),
        json!({
            "content": format!("# Research notes\n\n{}", RESEARCH_NOTES_LINE.repeat(40)),
            "encoding": "text"
        }),
    )
    .await;
    Session { id, uuid }
}

async fn send(client: &reqwest::Client, base: &str, session: &Session, text: &str) {
    post(
        client,
        format!("{base}/v1/sessions/{}/messages", session.id),
        json!({"message": {"content": [{"type": "text", "text": text}]}}),
    )
    .await;
}

fn percentile(values: &mut [f64], pct: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(f64::total_cmp);
    let index = ((values.len() as f64 * pct).ceil() as usize).clamp(1, values.len()) - 1;
    values[index]
}

fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

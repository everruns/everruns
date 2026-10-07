//! Server turn latency: one chat turn through the real server and a worker
//! over gRPC, end to end.
//!
//! The bench starts the production server in process (`ServerAppBuilder`,
//! HTTP and gRPC on local ports, PostgreSQL from `DATABASE_URL`) and a
//! standalone worker in process (`WorkerAppBuilder`, connected to that gRPC
//! port the way the `everruns-worker` binary is). It then creates an llmsim
//! provider with an instant model, an agent and sessions over HTTP, and sends
//! messages: each turn is `POST /v1/sessions/{id}/messages`, the server
//! enqueues the turn on the durable queue, the worker claims its steps over
//! gRPC and runs them, and the turn ends with `turn.completed`.
//!
//! Reported per turn, as p50/p95/p99:
//! - `e2e`: client wall time from sending the POST to receiving
//!   `turn.completed` on the session's SSE stream.
//! - `server`: `input.message` to `turn.completed`, from the events' own
//!   timestamps: the server's view, free of the client's polling.
//! - `pickup`: `input.message` to `turn.started`: enqueue, wakeup and claim.
//! - `db stmts/turn` and `db ms/turn`: statements PostgreSQL executed and
//!   their execution time per turn, from `pg_stat_statements` reset at the
//!   start of each scenario. Wall times swing with the machine; statement
//!   counts barely move between runs, so they are the regression signal for
//!   the platform's database work. Background loops (sweeps, heartbeats) run
//!   during the scenario and are counted too: they are part of what a turn
//!   costs the database. Needs `shared_preload_libraries=pg_stat_statements`;
//!   without it the columns read `n/a`. With `pg_stat_statements.track=all`
//!   (as CI runs it) statements inside trigger functions count too, so
//!   compare runs only under the same setting.
//!
//! Decisions:
//! - The model answers instantly (llmsim without `-latency` in the model id),
//!   so the numbers are the platform's own overhead: HTTP, persistence, the
//!   queue, the gRPC hop and event writes. Real model latency dwarfs them.
//! - Sessions are kept to five turns, as in the facade's `turn_backends`
//!   bench: a turn's cost grows with the history it carries.
//! - A bench target (`harness = false`, not run by `cargo test`), because it
//!   needs a database and process-wide environment, and builds the whole
//!   server in release. The weekly `durable-bench.yml` runs it; clippy's
//!   `--all-targets` keeps it compiling on every PR.
//! - The server runs its migrations on startup, so a fresh database is
//!   enough. The environment is set before Tokio starts, as in
//!   `background_sweep_wiring_test`, because the server reads it at build.
//!
//! Usage:
//!   DATABASE_URL=postgres://... cargo bench -p everruns-server --bench turn_latency
//!   ... -- --smoke                 # five turns, a second or two
//!   ... -- --summary out.jsonl     # append one JSON line per scenario
//!   ... -- --moniker <name>        # label for the summary lines
//!   ... -- --top <n>               # print the n statements costing most DB time per turn
//!
//! `--summary` lines follow `crates/durable/benches/baseline.jsonl`
//! (`tasks` are turns, `s2s_*` the pickup, `e2e_*` the client latency), so
//! `scripts/lib/durable-bench-compare.sh` can compare two runs.

use std::fs::OpenOptions;
use std::io::Write as _;
use std::net::TcpListener;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use everruns_server::app_builder::ServerAppBuilder;
use everruns_server::server::ServerConfig;
use everruns_worker::{TaskWorkerConfig, WorkerAppBuilder};
use futures::StreamExt as _;
use serde_json::{Value, json};

/// A turn may take this long before the run counts as broken.
const TURN_TIMEOUT: Duration = Duration::from_secs(60);
const TURNS_PER_SESSION: usize = 5;
const WORKER_TOKEN: &str = "server-turn-latency-bench";

struct Options {
    smoke: bool,
    moniker: String,
    summary: Option<PathBuf>,
    top: usize,
}

impl Options {
    fn from_args() -> Self {
        let mut smoke = false;
        let mut moniker = None;
        let mut summary = None;
        let mut top = 0;
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--smoke" => smoke = true,
                "--moniker" => moniker = args.next(),
                "--summary" => summary = args.next().map(PathBuf::from),
                "--top" => top = args.next().and_then(|n| n.parse().ok()).unwrap_or(10),
                // `cargo bench` passes `--bench`.
                _ => {}
            }
        }
        Self {
            smoke,
            moniker: moniker.unwrap_or_else(|| "local".into()),
            summary,
            top,
        }
    }
}

/// `concurrency` slots run at once, each sending the turns of
/// `sessions_per_slot` sessions one after another.
#[derive(Clone, Copy)]
struct Load {
    concurrency: usize,
    sessions_per_slot: usize,
}

struct Turn {
    e2e: Duration,
    server_ms: f64,
    pickup_ms: f64,
}

fn main() {
    let opts = Options::from_args();
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        eprintln!("turn_latency needs DATABASE_URL (a PostgreSQL database it may migrate)");
        std::process::exit(2);
    };
    let http_port = free_port();
    let grpc_port = free_port();
    configure_environment(&database_url);

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let lines = runtime.block_on(run(&opts, &database_url, http_port, grpc_port));

    if let Some(path) = &opts.summary {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap_or_else(|error| panic!("open {}: {error}", path.display()));
        for line in lines {
            writeln!(file, "{line}").expect("write summary");
        }
    }
    // The server and worker run until killed; this process is done.
    std::process::exit(0);
}

fn configure_environment(database_url: &str) {
    // SAFETY: called from `main` before the Tokio runtime or any other thread
    // exists, so nothing can observe the environment while it changes.
    unsafe {
        std::env::set_var("DATABASE_URL", database_url);
        std::env::set_var("DEPLOYMENT_GRADE", "dev");
        std::env::set_var("AUTH_MODE", "none");
        std::env::set_var("WORKER_GRPC_AUTH_TOKEN", WORKER_TOKEN);
        // Every client here shares 127.0.0.1, so the per-IP API limit (1200
        // requests a minute) trips on event polling once sessions run in
        // parallel. The bench measures turns, not the limiter.
        std::env::set_var("RATE_LIMIT_API_REQUESTS_PER_MINUTE", "0");
    }
}

/// A local port nothing listens on right now.
fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .expect("bind a local port")
        .port()
}

async fn run(opts: &Options, database_url: &str, http_port: u16, grpc_port: u16) -> Vec<Value> {
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
        .timeout(Duration::from_secs(30))
        .build()
        .expect("http client");
    wait_for_health(&client, &base, &server).await;

    let worker_config = TaskWorkerConfig {
        grpc_address: format!("127.0.0.1:{grpc_port}"),
        ..TaskWorkerConfig::default()
    };
    let worker = tokio::spawn(WorkerAppBuilder::new(worker_config).run());

    let agent = create_agent(&client, &base).await;
    // The first turn pays one-off costs (worker registration, caches).
    let mut warmup = create_session(&client, &base, &agent).await;
    send_turn(&client, &base, &mut warmup).await;
    let stats = DbStats::connect(database_url).await;

    let loads = if opts.smoke {
        vec![Load {
            concurrency: 1,
            sessions_per_slot: 1,
        }]
    } else {
        vec![
            Load {
                concurrency: 1,
                sessions_per_slot: 20,
            },
            Load {
                concurrency: 8,
                sessions_per_slot: 4,
            },
        ]
    };
    println!(
        "turn_latency ({}): real server + gRPC worker, llmsim, zero model latency",
        if opts.smoke { "smoke" } else { "full" }
    );
    println!(
        "{:>5} {:>6} {:>8} {:>9} {:>9} {:>9} {:>11} {:>11} {:>11} {:>14} {:>11}",
        "conc",
        "turns",
        "turns/s",
        "e2e p50",
        "e2e p95",
        "e2e p99",
        "server p50",
        "server p95",
        "pickup p50",
        "db stmts/turn",
        "db ms/turn"
    );
    let mut lines = Vec::new();
    for load in loads {
        let mut slots = Vec::new();
        for _ in 0..load.concurrency {
            let mut sessions = Vec::new();
            for _ in 0..load.sessions_per_slot {
                sessions.push(create_session(&client, &base, &agent).await);
            }
            slots.push(sessions);
        }
        if let Some(stats) = &stats {
            stats.reset().await;
        }
        let started = Instant::now();
        let tasks: Vec<_> = slots
            .into_iter()
            .map(|sessions| {
                let client = client.clone();
                let base = base.clone();
                tokio::spawn(async move {
                    let mut turns = Vec::new();
                    for mut session in sessions {
                        for _ in 0..TURNS_PER_SESSION {
                            turns.push(send_turn(&client, &base, &mut session).await);
                        }
                    }
                    turns
                })
            })
            .collect();
        let mut turns = Vec::new();
        for task in tasks {
            turns.extend(task.await.expect("session slot"));
        }
        let elapsed = started.elapsed();
        let db = match &stats {
            Some(stats) => Some(stats.per_turn(turns.len(), opts.top).await),
            None => None,
        };
        let db_cell =
            |value: Option<f64>| value.map_or_else(|| "n/a".to_owned(), |v| format!("{v:.1}"));

        let mut e2e: Vec<f64> = turns
            .iter()
            .map(|t| t.e2e.as_secs_f64() * 1_000.0)
            .collect();
        let mut server: Vec<f64> = turns.iter().map(|t| t.server_ms).collect();
        let mut pickup: Vec<f64> = turns.iter().map(|t| t.pickup_ms).collect();
        let turns_per_sec = turns.len() as f64 / elapsed.as_secs_f64();
        let pct = |values: &mut Vec<f64>, p: f64| round2(percentile(values, p));
        println!(
            "{:>5} {:>6} {:>8.1} {:>9.2} {:>9.2} {:>9.2} {:>11.2} {:>11.2} {:>11.2} {:>14} {:>11}",
            load.concurrency,
            turns.len(),
            turns_per_sec,
            pct(&mut e2e, 0.50),
            pct(&mut e2e, 0.95),
            pct(&mut e2e, 0.99),
            pct(&mut server, 0.50),
            pct(&mut server, 0.95),
            pct(&mut pickup, 0.50),
            db_cell(db.as_ref().map(|db| db.statements)),
            db_cell(db.as_ref().map(|db| db.exec_ms)),
        );
        if let Some(db) = &db {
            for line in &db.top {
                println!("      {line}");
            }
        }
        lines.push(json!({
            "bench": "server_turn_latency",
            "scenario": format!("text_c{}", load.concurrency),
            "moniker": opts.moniker,
            "smoke": opts.smoke,
            "tasks": turns.len(),
            "tasks_per_sec": (turns_per_sec * 10.0).round() / 10.0,
            "s2s_p50_ms": pct(&mut pickup, 0.50),
            "s2s_p99_ms": pct(&mut pickup, 0.99),
            "e2e_p50_ms": pct(&mut e2e, 0.50),
            "e2e_p95_ms": pct(&mut e2e, 0.95),
            "e2e_p99_ms": pct(&mut e2e, 0.99),
            "server_p50_ms": pct(&mut server, 0.50),
            "server_p95_ms": pct(&mut server, 0.95),
            "server_p99_ms": pct(&mut server, 0.99),
            "db_statements_per_turn": db.as_ref().map(|db| round2(db.statements)),
            "db_ms_per_turn": db.as_ref().map(|db| round2(db.exec_ms)),
        }));
    }

    worker.abort();
    server.abort();
    lines
}

/// Database work per turn, read from `pg_stat_statements`.
struct DbStats {
    pool: sqlx::PgPool,
}

struct DbPerTurn {
    statements: f64,
    exec_ms: f64,
    /// `--top` lines: the statements costing the most time per turn.
    top: Vec<String>,
}

impl DbStats {
    /// `None`, with a note, when the extension is not loaded.
    async fn connect(database_url: &str) -> Option<Self> {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect(database_url)
            .await
            .expect("bench database connection");
        let ready = sqlx::query("CREATE EXTENSION IF NOT EXISTS pg_stat_statements")
            .execute(&pool)
            .await
            .is_ok()
            && sqlx::query("SELECT 1 FROM pg_stat_statements LIMIT 1")
                .execute(&pool)
                .await
                .is_ok();
        if !ready {
            println!(
                "note: pg_stat_statements is not loaded (shared_preload_libraries), so DB work per turn is not reported"
            );
            return None;
        }
        Some(Self { pool })
    }

    async fn reset(&self) {
        sqlx::query("SELECT pg_stat_statements_reset()")
            .execute(&self.pool)
            .await
            .expect("reset pg_stat_statements");
    }

    async fn per_turn(&self, turns: usize, top: usize) -> DbPerTurn {
        let turns = turns.max(1) as f64;
        // Only this database: the server and worker share it, other
        // databases on the instance are not the bench's.
        let (calls, exec_ms): (f64, f64) = sqlx::query_as(
            "SELECT COALESCE(SUM(calls), 0)::float8, COALESCE(SUM(total_exec_time), 0)::float8
             FROM pg_stat_statements
             WHERE dbid = (SELECT oid FROM pg_database WHERE datname = current_database())
               AND query NOT ILIKE '%pg_stat_statements%'",
        )
        .fetch_one(&self.pool)
        .await
        .expect("read pg_stat_statements");
        let mut lines = Vec::new();
        if top > 0 {
            let rows: Vec<(f64, f64, String)> = sqlx::query_as(
                "SELECT calls::float8, total_exec_time, query
                 FROM pg_stat_statements
                 WHERE dbid = (SELECT oid FROM pg_database WHERE datname = current_database())
                   AND query NOT ILIKE '%pg_stat_statements%'
                 ORDER BY total_exec_time DESC
                 LIMIT $1",
            )
            .bind(top as i64)
            .fetch_all(&self.pool)
            .await
            .expect("read top statements");
            for (calls, total_ms, query) in rows {
                let query: String = query.split_whitespace().collect::<Vec<_>>().join(" ");
                let query: String = query.chars().take(110).collect();
                lines.push(format!(
                    "{:>6.2} ms {:>5.1} calls/turn  {query}",
                    total_ms / turns,
                    calls / turns
                ));
            }
        }
        DbPerTurn {
            statements: calls / turns,
            exec_ms: exec_ms / turns,
            top: lines,
        }
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

/// An agent on an llmsim model that answers at once.
async fn create_agent(client: &reqwest::Client, base: &str) -> String {
    let provider = post(
        client,
        format!("{base}/v1/providers"),
        json!({"name": "Turn latency bench", "provider_type": "llmsim"}),
    )
    .await;
    let provider_id = provider["id"].as_str().expect("provider id");
    // No `-latency` in the id: llmsim streams instantly.
    let model = post(
        client,
        format!("{base}/v1/providers/{provider_id}/models"),
        json!({
            "model_id": format!("llmsim-bench-{}", uuid::Uuid::now_v7().simple()),
            "display_name": "Turn latency bench",
            "enabled": true
        }),
    )
    .await;
    let agent = post(
        client,
        format!("{base}/v1/agents"),
        json!({
            "name": format!("turn-latency-bench-{}", uuid::Uuid::now_v7().simple()),
            "system_prompt": "Reply briefly.",
            "default_model_id": model["id"]
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
    /// The session's SSE stream, opened once, as the UI does. Polling the
    /// events endpoint instead put about ten extra HTTP requests per turn on
    /// the server (each with its auth and session reads), which swamped the
    /// turn's own database work in the per-turn statement counts.
    stream: SseReader,
}

/// A minimal reader of the session SSE stream: one JSON event per frame.
struct SseReader {
    body: futures::stream::BoxStream<'static, reqwest::Result<Vec<u8>>>,
    buffer: String,
}

impl SseReader {
    /// Opens the stream and waits for the server's `connected` frame: the
    /// server subscribes to the session before sending it, so nothing written
    /// after this returns is missed.
    async fn open(base: &str, session_id: &str) -> Self {
        // Its own client: the request client's 30 s timeout would cut the
        // stream.
        let response = reqwest::Client::new()
            .get(format!("{base}/v1/sessions/{session_id}/sse"))
            .header("accept", "text/event-stream")
            .send()
            .await
            .unwrap_or_else(|error| panic!("SSE for {session_id}: {error}"));
        assert!(
            response.status().is_success(),
            "SSE for {session_id}: {}",
            response.status()
        );
        let mut reader = Self {
            body: response
                .bytes_stream()
                .map(|chunk| chunk.map(|bytes| bytes.to_vec()))
                .boxed(),
            buffer: String::new(),
        };
        let deadline = Duration::from_secs(10);
        match tokio::time::timeout(deadline, reader.next_frame()).await {
            Ok(Some((kind, _))) if kind == "connected" => reader,
            other => panic!("SSE for {session_id} did not connect: {other:?}"),
        }
    }

    /// The next frame's event name and JSON data, `None` when the stream ends.
    async fn next_frame(&mut self) -> Option<(String, Value)> {
        loop {
            if let Some(end) = self.buffer.find("\n\n") {
                let frame: String = self.buffer.drain(..end + 2).collect();
                let mut kind = String::new();
                let mut data = String::new();
                for line in frame.lines() {
                    if let Some(value) = line.strip_prefix("event:") {
                        kind = value.trim().to_owned();
                    } else if let Some(value) = line.strip_prefix("data:") {
                        if !data.is_empty() {
                            data.push('\n');
                        }
                        data.push_str(value.trim_start());
                    }
                }
                if data.is_empty() {
                    continue;
                }
                let data = serde_json::from_str(&data).unwrap_or(Value::Null);
                return Some((kind, data));
            }
            let chunk = self.body.next().await?.ok()?;
            self.buffer
                .push_str(&String::from_utf8_lossy(&chunk).replace("\r\n", "\n"));
        }
    }
}

async fn create_session(client: &reqwest::Client, base: &str, agent: &str) -> Session {
    let session = post(
        client,
        format!("{base}/v1/sessions"),
        json!({"harness_name": "base", "agent_id": agent, "title": "Turn latency bench"}),
    )
    .await;
    let id = session["id"].as_str().expect("session id").to_owned();
    let stream = SseReader::open(base, &id).await;
    Session { id, stream }
}

/// Send one message and wait for its turn to complete on the SSE stream.
async fn send_turn(client: &reqwest::Client, base: &str, session: &mut Session) -> Turn {
    let sent = Instant::now();
    post(
        client,
        format!("{base}/v1/sessions/{}/messages", session.id),
        json!({"message": {"content": [{"type": "text", "text": "Hello"}]}}),
    )
    .await;
    let mut seen: Vec<Value> = Vec::new();
    let (turn_id, input_message_id) = loop {
        let remaining = TURN_TIMEOUT
            .checked_sub(sent.elapsed())
            .unwrap_or_else(|| panic!("turn did not finish: {seen:?}"));
        let event = match tokio::time::timeout(remaining, session.stream.next_frame()).await {
            Ok(Some((_, event))) => event,
            other => panic!("SSE for {}: {other:?} after {seen:?}", session.id),
        };
        if event["type"] == "turn.failed" {
            panic!("turn failed: {event}");
        }
        let completed = event["type"] == "turn.completed";
        seen.push(event);
        if completed {
            let last = seen.last().expect("just pushed");
            let turn_id = last["context"]["turn_id"]
                .as_str()
                .or_else(|| last["data"]["turn_id"].as_str())
                .expect("turn.completed names its turn")
                .to_owned();
            let input_message_id = last["context"]["input_message_id"]
                .as_str()
                .map(str::to_owned);
            break (turn_id, input_message_id);
        }
    };
    let e2e = sent.elapsed();

    // A previous turn's trailing events (session.idled) can arrive after this
    // POST; only this turn's events count.
    let input_message_id = input_message_id.as_deref();
    let at = |kind: &str| {
        seen.iter()
            .filter(|event| event["type"] == kind)
            .filter(|event| {
                // `input.message` is written before its turn exists, so it
                // carries no turn context: match it by the turn's message.
                event["context"]["turn_id"].as_str() == Some(turn_id.as_str())
                    || (input_message_id.is_some()
                        && event["data"]["message"]["id"].as_str() == input_message_id)
            })
            .filter_map(|event| event["ts"].as_str())
            .filter_map(|ts| DateTime::parse_from_rfc3339(ts).ok())
            .map(|ts| ts.with_timezone(&Utc))
            .min()
            .unwrap_or_else(|| panic!("turn has no {kind}: {seen:?}"))
    };
    let input = at("input.message");
    let ms = |to: DateTime<Utc>| (to - input).num_microseconds().unwrap_or(0) as f64 / 1_000.0;
    Turn {
        e2e,
        server_ms: ms(at("turn.completed")),
        pickup_ms: ms(at("turn.started")),
    }
}

fn percentile(values: &mut [f64], pct: f64) -> f64 {
    values.sort_by(f64::total_cmp);
    let index = ((values.len() as f64 * pct).ceil() as usize).clamp(1, values.len()) - 1;
    values[index]
}

fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

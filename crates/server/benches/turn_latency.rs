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
//! - `e2e`: client wall time from sending the POST to seeing `turn.completed`
//!   in the session's events (polled every 5 ms, so it carries up to that
//!   much polling slack).
//! - `server`: `input.message` to `turn.completed`, from the events' own
//!   timestamps: the server's view, free of the client's polling.
//! - `pickup`: `input.message` to `turn.started`: enqueue, wakeup and claim.
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
use serde_json::{Value, json};

/// A turn may take this long before the run counts as broken.
const TURN_TIMEOUT: Duration = Duration::from_secs(60);
const POLL: Duration = Duration::from_millis(5);
const TURNS_PER_SESSION: usize = 5;
const WORKER_TOKEN: &str = "server-turn-latency-bench";

struct Options {
    smoke: bool,
    moniker: String,
    summary: Option<PathBuf>,
}

impl Options {
    fn from_args() -> Self {
        let mut smoke = false;
        let mut moniker = None;
        let mut summary = None;
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--smoke" => smoke = true,
                "--moniker" => moniker = args.next(),
                "--summary" => summary = args.next().map(PathBuf::from),
                // `cargo bench` passes `--bench`.
                _ => {}
            }
        }
        Self {
            smoke,
            moniker: moniker.unwrap_or_else(|| "local".into()),
            summary,
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
    let lines = runtime.block_on(run(&opts, http_port, grpc_port));

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
    }
}

/// A local port nothing listens on right now.
fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .expect("bind a local port")
        .port()
}

async fn run(opts: &Options, http_port: u16, grpc_port: u16) -> Vec<Value> {
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
        "{:>5} {:>6} {:>8} {:>9} {:>9} {:>9} {:>11} {:>11} {:>11}",
        "conc",
        "turns",
        "turns/s",
        "e2e p50",
        "e2e p95",
        "e2e p99",
        "server p50",
        "server p95",
        "pickup p50"
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

        let mut e2e: Vec<f64> = turns
            .iter()
            .map(|t| t.e2e.as_secs_f64() * 1_000.0)
            .collect();
        let mut server: Vec<f64> = turns.iter().map(|t| t.server_ms).collect();
        let mut pickup: Vec<f64> = turns.iter().map(|t| t.pickup_ms).collect();
        let turns_per_sec = turns.len() as f64 / elapsed.as_secs_f64();
        let pct = |values: &mut Vec<f64>, p: f64| round2(percentile(values, p));
        println!(
            "{:>5} {:>6} {:>8.1} {:>9.2} {:>9.2} {:>9.2} {:>11.2} {:>11.2} {:>11.2}",
            load.concurrency,
            turns.len(),
            turns_per_sec,
            pct(&mut e2e, 0.50),
            pct(&mut e2e, 0.95),
            pct(&mut e2e, 0.99),
            pct(&mut server, 0.50),
            pct(&mut server, 0.95),
            pct(&mut pickup, 0.50),
        );
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
        }));
    }

    worker.abort();
    server.abort();
    lines
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
    /// The highest event sequence seen, so each turn reads only its own
    /// events. A sequence cursor, not `since_id`: lifecycle events are stored
    /// in the background (write-behind), so ids do not arrive in id order.
    last_sequence: Option<i64>,
}

async fn create_session(client: &reqwest::Client, base: &str, agent: &str) -> Session {
    let session = post(
        client,
        format!("{base}/v1/sessions"),
        json!({"harness_name": "base", "agent_id": agent, "title": "Turn latency bench"}),
    )
    .await;
    let mut session = Session {
        id: session["id"].as_str().expect("session id").to_owned(),
        last_sequence: None,
    };
    // Skip the session's creation events.
    events_after(client, base, &mut session).await;
    session
}

async fn get_events(client: &reqwest::Client, url: &str) -> Vec<Value> {
    let body: Value = client
        .get(url)
        .send()
        .await
        .unwrap_or_else(|error| panic!("GET {url}: {error}"))
        .json()
        .await
        .unwrap_or_else(|error| panic!("GET {url}: {error}"));
    body["data"].as_array().cloned().unwrap_or_default()
}

/// The session's events after the last one seen, advancing the cursor.
async fn events_after(client: &reqwest::Client, base: &str, session: &mut Session) -> Vec<Value> {
    let mut url = format!("{base}/v1/sessions/{}/events", session.id);
    match session.last_sequence {
        Some(sequence) => url.push_str(&format!("?after_sequence={sequence}")),
        None => url.push_str("?limit=1000"),
    }
    let events = get_events(client, &url).await;
    if let Some(sequence) = events.iter().filter_map(|e| e["sequence"].as_i64()).max() {
        session.last_sequence = Some(sequence.max(session.last_sequence.unwrap_or(0)));
    }
    events
}

/// Send one message and wait for its turn to complete.
async fn send_turn(client: &reqwest::Client, base: &str, session: &mut Session) -> Turn {
    let sent = Instant::now();
    post(
        client,
        format!("{base}/v1/sessions/{}/messages", session.id),
        json!({"message": {"content": [{"type": "text", "text": "Hello"}]}}),
    )
    .await;
    let mut seen = Vec::new();
    let completed = loop {
        let events = events_after(client, base, session).await;
        if let Some(failed) = events.iter().find(|event| event["type"] == "turn.failed") {
            panic!("turn failed: {failed}");
        }
        let completed = events
            .iter()
            .find(|event| event["type"] == "turn.completed")
            .cloned();
        seen.extend(events);
        if let Some(completed) = completed {
            break completed;
        }
        assert!(
            sent.elapsed() < TURN_TIMEOUT,
            "turn did not finish: {seen:?}"
        );
        tokio::time::sleep(POLL).await;
    };
    let e2e = sent.elapsed();

    // Read the whole turn once it ended, so an event stored after the cursor
    // moved past its sequence still counts.
    let turn_id = completed["data"]["turn_id"]
        .as_str()
        .expect("turn.completed names its turn");
    let mut turn = get_events(
        client,
        &format!(
            "{base}/v1/sessions/{}/events?turn_id={turn_id}&limit=1000",
            session.id
        ),
    )
    .await;
    turn.extend(seen);
    let at = |kind: &str| {
        turn.iter()
            .filter(|event| event["type"] == kind)
            .filter_map(|event| event["ts"].as_str())
            .filter_map(|ts| DateTime::parse_from_rfc3339(ts).ok())
            .map(|ts| ts.with_timezone(&Utc))
            .min()
            .unwrap_or_else(|| panic!("turn has no {kind}: {turn:?}"))
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

//! Step 8's latency gate (design.md): with 200 sessions streaming at once, a
//! delta reaches its SSE client within 100 ms (p99) of the executor making
//! it, and the edge router adds under 5 ms (p99) to a data plane call.
//!
//! The `stamp` model writes the time it made each delta into the delta, so
//! the client measures make-to-arrival without any clock but its own. The
//! numbers are printed for the step's record; the gate's are from a release
//! build (`cargo test --profile release-ci -p everruns-cell-testkit --test
//! latency -- --nocapture`).

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use everruns_cell::dp::{ConfigAck, ConfigUpdate};
use everruns_cell::edge::Edge;
use everruns_cell::gateway::Gateway;
use everruns_cell::node_control::ControlPlane;
use everruns_cell::org_token::OrgTokens;
use everruns_cell_testkit::fake_cp::FakeCp;
use serde_json::{Value, json};
use support::Fleet;

const SESSIONS: usize = 200;

// Each gate measures its own loaded fleet, not contention with the other
// benchmark's fleet. Keep the same budgets under both default libtest
// parallelism (main CI) and --test-threads=1 (affected-crate PR CI).
static LATENCY_GATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let at = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[at.min(sorted.len() - 1)]
}

/// What one stream saw: every delta's latency, and the turn's first.
struct Seen {
    deltas: Vec<f64>,
    first: Option<f64>,
}

/// Read `url`'s stream until the turn completes; latency of each delta in ms.
async fn watch(http: reqwest::Client, url: String, ready: tokio::sync::mpsc::Sender<()>) -> Seen {
    let mut response = http.get(&url).send().await.unwrap();
    assert!(response.status().is_success(), "{url}");
    let mut buf = String::new();
    let mut seen = Seen {
        deltas: Vec::new(),
        first: None,
    };
    let mut announced = false;
    let deadline = Instant::now() + Duration::from_secs(120);
    while Instant::now() < deadline {
        let Ok(Ok(Some(chunk))) =
            tokio::time::timeout(Duration::from_secs(60), response.chunk()).await
        else {
            break;
        };
        let now = chrono::Utc::now().timestamp_micros();
        buf.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(at) = buf.find("\n\n") {
            let frame: String = buf.drain(..at + 2).collect();
            let event = frame
                .lines()
                .find_map(|l| l.strip_prefix("event: "))
                .unwrap_or_default();
            if event == "connected" && !announced {
                announced = true;
                let _ = ready.send(()).await;
            }
            if event == "turn.completed" {
                return seen;
            }
            if event != "output.message.delta" {
                continue;
            }
            let data: Value = frame
                .lines()
                .find_map(|l| l.strip_prefix("data: "))
                .and_then(|d| serde_json::from_str(d).ok())
                .unwrap_or(Value::Null);
            let delta = data["data"]["delta"]
                .as_str()
                .or_else(|| data["delta"].as_str())
                .unwrap_or_default();
            for stamp in delta.split(';').filter_map(|s| s.strip_prefix('@')) {
                if let Ok(made) = stamp.parse::<i64>() {
                    let ms = (now - made) as f64 / 1000.0;
                    seen.first.get_or_insert(ms);
                    seen.deltas.push(ms);
                }
            }
        }
    }
    seen
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn deltas_reach_200_streaming_clients_within_100ms_p99() {
    let _gate = LATENCY_GATE.lock().await;
    let mut fleet = Fleet::start().await;
    fleet.executor_with(1, SESSIONS / 2);
    fleet.executor_with(2, SESSIONS / 2);
    let mut sessions = Vec::with_capacity(SESSIONS);
    for _ in 0..SESSIONS {
        sessions.push(fleet.create("stamp").await);
    }
    let (ready, mut connected) = tokio::sync::mpsc::channel(SESSIONS);
    let mut watchers = Vec::with_capacity(SESSIONS);
    for session in &sessions {
        let url = format!("{}/v1/sessions/{session}/sse", fleet.base());
        watchers.push(tokio::spawn(watch(fleet.http.clone(), url, ready.clone())));
    }
    drop(ready);
    tokio::time::timeout(Duration::from_secs(30), async {
        for _ in 0..SESSIONS {
            connected
                .recv()
                .await
                .expect("a stream failed before connecting");
        }
    })
    .await
    .expect("all streams connect before the latency measurement");
    let sent = Instant::now();
    let says: Vec<_> = sessions
        .iter()
        .map(|session| {
            let http = fleet.http.clone();
            let url = format!("{}/v1/sessions/{session}/messages", fleet.base());
            tokio::spawn(async move {
                let reply = http
                    .post(url)
                    .json(&json!({ "text": "go" }))
                    .send()
                    .await
                    .unwrap();
                assert!(reply.status().is_success(), "{}", reply.status());
            })
        })
        .collect();
    for say in says {
        say.await.unwrap();
    }
    let mut deltas = Vec::new();
    let mut firsts = Vec::new();
    for watcher in watchers {
        let seen = watcher.await.unwrap();
        deltas.extend(seen.deltas);
        firsts.extend(seen.first);
    }
    deltas.sort_by(f64::total_cmp);
    firsts.sort_by(f64::total_cmp);
    let (p50, p99, max) = (
        percentile(&deltas, 0.5),
        percentile(&deltas, 0.99),
        percentile(&deltas, 1.0),
    );
    let first_p99 = percentile(&firsts, 0.99);
    eprintln!(
        "streaming: {} sessions, {} deltas in {:?}; delta latency p50 {p50:.1} ms, p99 {p99:.1} ms, max {max:.1} ms; first delta p50 {:.1} ms, p99 {first_p99:.1} ms",
        SESSIONS,
        deltas.len(),
        sent.elapsed(),
        percentile(&firsts, 0.5),
    );
    assert_eq!(firsts.len(), SESSIONS, "every stream saw its deltas");
    assert!(
        deltas.len() >= SESSIONS * everruns_cell_testkit::STAMPS * 9 / 10,
        "few deltas: {}",
        deltas.len()
    );
    // The budget is a release number. Unoptimized, with three nodes, two
    // executors and 200 clients sharing one process on a small runner, the
    // check keeps a looser bound that still catches a stream that batches
    // or stalls.
    let budget = if cfg!(debug_assertions) { 500.0 } else { 100.0 };
    assert!(first_p99 < budget, "first delta p99 {first_p99:.1} ms");
    assert!(p99 < budget, "delta p99 {p99:.1} ms");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_edge_adds_under_5ms_p99() {
    let _gate = LATENCY_GATE.lock().await;
    const ORG: &str = "org-a";
    const LINK: &str = "link-secret";
    const CALLS: usize = 400;
    let cp = FakeCp::start().await.unwrap();
    cp.set_link_token(LINK);
    cp.set_balance(ORG, 10_000_000);
    let mut link = ControlPlane::new(cp.url.clone());
    link.token = Some(LINK.into());
    link.jwks_url = Some(cp.jwks_url());
    let mut fleet = Fleet::start_with_cp(Some(link)).await;
    fleet.executor(1);
    let mut package = everruns_cell_testkit::definition("echo").unwrap();
    package["name"] = json!("helper");
    let update = ConfigUpdate {
        epoch: 1,
        version: 1,
        base_version: 0,
        changes: vec![
            json!({ "kind": "Agent", "agent_id": "helper", "revision": 1, "package": package }),
        ],
    };
    let ack = cp.push_config(&fleet.base(), ORG, &update).await.unwrap();
    assert_eq!(ack, ConfigAck::Ok { applied_version: 1 });
    let tokens = Arc::new(OrgTokens::new(Some(cp.jwks_url()), Some("dp-local".into())));
    let edge = Edge::new(Gateway::new(fleet.topology()), &cp.url, tokens);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let edge_url = format!("http://{}", listener.local_addr().unwrap());
    let app = edge.router();
    tokio::spawn(async move { axum::serve(listener, app).await });

    let token = cp.org_token(ORG, "sessions:write").unwrap();
    // The edge's own gateway, as a client would reach it without the edge.
    let direct = Gateway::new(fleet.topology());
    let direct_app = everruns_cell::gateway::router(direct);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let direct_url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, direct_app).await });

    let created: Value = fleet
        .http
        .post(format!("{direct_url}/v1/sessions"))
        .bearer_auth(&token)
        .json(&json!({ "org": ORG, "agent_ref": "helper" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let session = created["id"].as_str().unwrap().to_owned();
    let path = format!("/v1/sessions/{session}/events");
    let time = |url: String, auth: String| {
        let http = fleet.http.clone();
        async move {
            let started = Instant::now();
            let reply = http
                .get(url)
                .header("authorization", auth)
                .send()
                .await
                .unwrap();
            assert!(reply.status().is_success(), "{}", reply.status());
            let _ = reply.bytes().await.unwrap();
            started.elapsed().as_secs_f64() * 1000.0
        }
    };
    // Warm both paths (the exchange, the session's location).
    for _ in 0..20 {
        time(format!("{edge_url}{path}"), "Bearer user-org-a".into()).await;
        time(format!("{direct_url}{path}"), format!("Bearer {token}")).await;
    }
    let (mut through, mut straight) = (Vec::new(), Vec::new());
    for _ in 0..CALLS {
        through.push(time(format!("{edge_url}{path}"), "Bearer user-org-a".into()).await);
        straight.push(time(format!("{direct_url}{path}"), format!("Bearer {token}")).await);
    }
    through.sort_by(f64::total_cmp);
    straight.sort_by(f64::total_cmp);
    let hop50 = percentile(&through, 0.5) - percentile(&straight, 0.5);
    let hop99 = percentile(&through, 0.99) - percentile(&straight, 0.99);
    eprintln!(
        "edge: {CALLS} calls; through the edge p50 {:.2} ms p99 {:.2} ms, straight p50 {:.2} ms p99 {:.2} ms; hop p50 {hop50:.2} ms, p99 {hop99:.2} ms",
        percentile(&through, 0.5),
        percentile(&through, 0.99),
        percentile(&straight, 0.5),
        percentile(&straight, 0.99),
    );
    assert_eq!(cp.exchanges(), 1, "one exchange, then the cached token");
    assert!(hop99 < 5.0, "the edge hop p99 is {hop99:.2} ms");
}

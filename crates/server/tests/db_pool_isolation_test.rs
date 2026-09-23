//! Background sweeps must not be starved by a request burst (EVE-1081).
//!
//! Requirements:
//! - PostgreSQL running with DATABASE_URL set (or the `just start-infra` default)
//!
//! Production evidence (Sentry EVERRUNS-13/15/16/B, release 364f27cf): four
//! independent background loops — the durable scheduler, the observer scoring
//! worker and the sweeps around them — all reported
//! `pool timed out while waiting for an open connection` within two seconds of
//! each other, with zero users impacted. Every one of them was drawing from the
//! single process-wide `sqlx` pool, so one request burst that held every
//! connection for longer than the 5s acquire timeout failed all of them at the
//! same instant.
//!
//! These tests pin both halves of that: the shared pool really does fail every
//! background acquire together, and the dedicated background pool really does
//! keep them running through the same burst.

use everruns_server::storage::repositories::{Database, DatabasePoolConfig};
use sqlx::Row;
use std::time::Duration;

fn database_url() -> String {
    std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        let port = std::env::var("DB_PORT").unwrap_or_else(|_| "9332".to_string());
        format!("postgres://everruns:everruns@localhost:{port}/everruns_test")
    })
}

/// Prod's shape, scaled down so the test runs in seconds: a small request pool
/// with a short acquire timeout, mirroring `DATABASE_POOL_MAX` / the 5s
/// `acquire_timeout` default.
fn scaled_config(background_max: u32) -> DatabasePoolConfig {
    DatabasePoolConfig {
        max_connections: 4,
        min_connections: 1,
        acquire_timeout: Duration::from_millis(500),
        idle_timeout: Duration::from_secs(300),
        background_max_connections: background_max,
        background_acquire_timeout: Duration::from_secs(10),
    }
}

/// Hold every connection in the request pool, the way a burst of HTTP handlers
/// does. The guards are returned so the caller controls when the burst ends.
async fn saturate(
    pool: &sqlx::PgPool,
    count: usize,
) -> Vec<sqlx::pool::PoolConnection<sqlx::Postgres>> {
    let mut held = Vec::new();
    for _ in 0..count {
        held.push(
            pool.acquire()
                .await
                .expect("request burst should be able to fill an idle pool"),
        );
    }
    held
}

/// The three background loops Sentry caught failing together.
const SWEEPS: [&str; 3] = ["durable_scheduler", "observer_worker", "tool_result_sweep"];

/// Run all three concurrently, each doing one trivial query. Prints how long
/// each waited and how it ended, so the run itself is the evidence: under one
/// shared pool they all end at the same instant.
async fn run_sweeps(db: &Database, label: &str) -> Vec<Result<(), String>> {
    let mut set = tokio::task::JoinSet::new();
    for name in SWEEPS {
        let db = db.clone();
        set.spawn(async move {
            let started = std::time::Instant::now();
            let outcome = sqlx::query("SELECT 1 AS one")
                .fetch_one(db.pool())
                .await
                .map(|row| {
                    let _: i32 = row.get("one");
                })
                .map_err(|e| e.to_string());
            (name, started.elapsed(), outcome)
        });
    }
    let mut out = Vec::new();
    while let Some(joined) = set.join_next().await {
        let (name, elapsed, outcome) = joined.expect("sweep task panicked");
        match &outcome {
            Ok(()) => println!("[{label}] {name}: ok after {elapsed:?}"),
            Err(error) => println!("[{label}] {name}: after {elapsed:?} -> {error}"),
        }
        out.push(outcome);
    }
    out
}

/// The bug: with one pool for everything, a request burst fails every
/// background sweep at once.
#[tokio::test(flavor = "multi_thread")]
async fn shared_pool_fails_every_background_sweep_at_once() {
    // `background_max_connections: 0` is the documented escape hatch that puts
    // background work back on the request pool — i.e. the pre-EVE-1081 shape.
    let db = Database::connect_with_config(&database_url(), scaled_config(0))
        .await
        .expect("connect");

    let _burst = saturate(db.pool(), 4).await;
    let results = run_sweeps(&db, "shared pool").await;

    assert!(
        results.iter().all(|r| r.is_err()),
        "expected the shared pool to starve every sweep, got {results:?}"
    );
    for result in &results {
        let error = result.as_ref().unwrap_err();
        assert!(
            error.contains("pool timed out"),
            "expected the production error, got {error}"
        );
    }
}

/// The fix: the same burst leaves the background pool untouched.
#[tokio::test(flavor = "multi_thread")]
async fn dedicated_background_pool_survives_a_request_burst() {
    let db = Database::connect_with_config(&database_url(), scaled_config(2))
        .await
        .expect("connect");
    let background = db.for_background();

    let _burst = saturate(db.pool(), 4).await;
    let results = run_sweeps(&background, "background pool").await;

    assert!(
        results.iter().all(|r| r.is_ok()),
        "background sweeps must survive a request burst, got {results:?}"
    );
}

/// The request pool still fails fast while the burst is in flight — the fix
/// must not paper over saturation on the path where a caller is waiting.
#[tokio::test(flavor = "multi_thread")]
async fn request_pool_still_fails_fast_under_the_same_burst() {
    let db = Database::connect_with_config(&database_url(), scaled_config(2))
        .await
        .expect("connect");

    let _burst = saturate(db.pool(), 4).await;
    let results = run_sweeps(&db, "request pool").await;

    assert!(
        results.iter().all(|r| r.is_err()),
        "request-path acquires must still time out, got {results:?}"
    );
}

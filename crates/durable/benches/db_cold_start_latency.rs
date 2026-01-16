//! Cold-start latency benchmark with PostgreSQL backend
//!
//! Measures the time from task enqueue to task execution start when the worker
//! is idle, using real PostgreSQL persistence. This measures the actual database
//! overhead in user-perceived responsiveness.
//!
//! Requirements:
//!   - PostgreSQL 17 running with DATABASE_URL set
//!   - Migrations applied from crates/control-plane/migrations/
//!
//! Usage:
//!   cargo bench -p everruns-durable --bench db_cold_start_latency
//!   cargo bench -p everruns-durable --bench db_cold_start_latency -- --save
//!   cargo bench -p everruns-durable --bench db_cold_start_latency -- --save --moniker ci-4cpu-8gb

use std::env;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use sqlx::PgPool;
use tokio::runtime::Runtime;

use everruns_durable::bench::{
    BenchmarkCheckpoint, BenchmarkMetrics, BenchmarkReport, CheckpointStore, EnvironmentInfo,
    ReportConfig,
};
use everruns_durable::persistence::{
    PostgresWorkflowEventStore, TaskDefinition, WorkflowEventStore,
};
use everruns_durable::workflow::ActivityOptions;
use uuid::Uuid;

/// Get test database URL from environment or use default
fn get_database_url() -> String {
    env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/everruns_test".to_string())
}

/// Configuration for a cold-start test
struct ColdStartConfig {
    /// Number of iterations to run
    iterations: usize,
    /// Number of workers
    worker_count: usize,
    /// Worker check interval (simulates current implementation behavior)
    check_interval: Duration,
    /// Name for this scenario
    name: String,
}

/// Run a single cold-start measurement
///
/// Returns the duration from enqueue to task claim (schedule-to-start latency)
async fn measure_single_cold_start(
    store: &Arc<PostgresWorkflowEventStore>,
    workflow_id: Uuid,
    task_num: usize,
    task_claimed: &Arc<AtomicBool>,
    claim_time: &Arc<parking_lot::Mutex<Option<Instant>>>,
) -> Duration {
    // Signal that we're about to enqueue
    task_claimed.store(false, Ordering::SeqCst);

    // Wait for worker to be in idle state
    tokio::time::sleep(Duration::from_millis(10)).await;

    // Record enqueue time and enqueue task
    let enqueue_time = Instant::now();
    store
        .enqueue_task(TaskDefinition {
            workflow_id,
            activity_id: format!("cold-start-{}", task_num),
            activity_type: "db_cold_start_activity".to_string(),
            input: serde_json::json!({ "task_num": task_num }),
            options: ActivityOptions::default(),
        })
        .await
        .unwrap();

    // Wait for worker to claim the task
    while !task_claimed.load(Ordering::SeqCst) {
        tokio::time::sleep(Duration::from_micros(10)).await;
    }

    // Get the claim time recorded by the worker
    let claimed_at = claim_time.lock().take().unwrap();
    claimed_at.duration_since(enqueue_time)
}

/// Run cold-start latency scenario with PostgreSQL
async fn run_cold_start_scenario(
    pool: PgPool,
    config: ColdStartConfig,
) -> Arc<BenchmarkMetrics> {
    let metrics = Arc::new(BenchmarkMetrics::new(&config.name));
    let store = Arc::new(PostgresWorkflowEventStore::new(pool.clone()));
    let workflow_id = Uuid::now_v7();

    // Create workflow
    store
        .create_workflow(
            workflow_id,
            "db_cold_start_benchmark",
            serde_json::json!({}),
            None,
        )
        .await
        .unwrap();

    println!("\n🗄️  Running (PostgreSQL): {}", config.name);
    println!(
        "   Iterations: {}, Workers: {}, Check interval: {:?}",
        config.iterations, config.worker_count, config.check_interval
    );

    // Synchronization primitives
    let task_claimed = Arc::new(AtomicBool::new(false));
    let claim_time = Arc::new(parking_lot::Mutex::new(None));
    let shutdown = Arc::new(AtomicBool::new(false));
    let tasks_completed = Arc::new(AtomicU64::new(0));

    // Spawn worker(s)
    let mut worker_handles = Vec::new();
    for worker_id in 0..config.worker_count {
        let store = store.clone();
        let check_interval = config.check_interval;
        let task_claimed = task_claimed.clone();
        let claim_time = claim_time.clone();
        let shutdown = shutdown.clone();
        let tasks_completed = tasks_completed.clone();

        worker_handles.push(tokio::spawn(async move {
            let worker_name = format!("db-cold-start-worker-{}", worker_id);

            loop {
                if shutdown.load(Ordering::SeqCst) {
                    break;
                }

                // Check for tasks
                let claimed = store
                    .claim_task(&worker_name, &["db_cold_start_activity".to_string()], 1)
                    .await
                    .unwrap();

                if !claimed.is_empty() {
                    // Record claim time
                    *claim_time.lock() = Some(Instant::now());
                    task_claimed.store(true, Ordering::SeqCst);

                    // Complete the task
                    for task in claimed {
                        store
                            .complete_task(task.id, &worker_name, serde_json::json!({"ok": true}))
                            .await
                            .unwrap();
                        tasks_completed.fetch_add(1, Ordering::Relaxed);
                    }
                }

                // Wait before next check
                tokio::time::sleep(check_interval).await;
            }
        }));
    }

    // Run iterations
    let start = Instant::now();
    let mut latencies = Vec::with_capacity(config.iterations);

    for i in 0..config.iterations {
        let latency =
            measure_single_cold_start(&store, workflow_id, i, &task_claimed, &claim_time).await;

        latencies.push(latency);
        metrics.schedule_to_start.record(latency);
        metrics.end_to_end.record(latency); // For cold start, S2S == E2E
        metrics.tasks_completed.increment();

        // Brief pause between iterations to ensure worker returns to idle
        tokio::time::sleep(config.check_interval * 2).await;
    }

    // Shutdown workers
    shutdown.store(true, Ordering::SeqCst);
    for handle in worker_handles {
        handle.abort();
    }

    let elapsed = start.elapsed();

    // Cleanup test data
    cleanup_workflow(&pool, workflow_id).await;

    // Calculate statistics
    latencies.sort();
    let p50 = latencies[latencies.len() / 2];
    let p99 = latencies[(latencies.len() as f64 * 0.99) as usize];
    let min = latencies[0];
    let max = latencies[latencies.len() - 1];
    let avg: Duration = latencies.iter().sum::<Duration>() / latencies.len() as u32;

    println!(
        "✅ Completed {} iterations in {:.2}s",
        config.iterations,
        elapsed.as_secs_f64()
    );
    println!(
        "   Cold-start latency: min={:.2}ms avg={:.2}ms p50={:.2}ms p99={:.2}ms max={:.2}ms",
        min.as_secs_f64() * 1000.0,
        avg.as_secs_f64() * 1000.0,
        p50.as_secs_f64() * 1000.0,
        p99.as_secs_f64() * 1000.0,
        max.as_secs_f64() * 1000.0,
    );

    metrics
}

/// Cleanup test data after the benchmark
async fn cleanup_workflow(pool: &PgPool, workflow_id: Uuid) {
    // Delete in reverse dependency order
    sqlx::query("DELETE FROM durable_signals WHERE workflow_id = $1")
        .bind(workflow_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM durable_dead_letter_queue WHERE workflow_id = $1")
        .bind(workflow_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM durable_task_queue WHERE workflow_id = $1")
        .bind(workflow_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM durable_workflow_events WHERE workflow_id = $1")
        .bind(workflow_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM durable_workflow_instances WHERE id = $1")
        .bind(workflow_id)
        .execute(pool)
        .await
        .ok();
}

/// CLI options parsed from arguments
struct CliOptions {
    save_checkpoint: bool,
    moniker: Option<String>,
}

fn parse_args() -> CliOptions {
    let args: Vec<String> = env::args().collect();
    let mut opts = CliOptions {
        save_checkpoint: false,
        moniker: None,
    };

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--save" => opts.save_checkpoint = true,
            "--moniker" => {
                if i + 1 < args.len() {
                    opts.moniker = Some(args[i + 1].clone());
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }

    opts
}

fn main() {
    let opts = parse_args();
    let rt = Runtime::new().unwrap();

    // Connect to PostgreSQL
    let database_url = get_database_url();
    let pool = rt.block_on(async {
        PgPool::connect(&database_url)
            .await
            .expect("Failed to connect to PostgreSQL. Set DATABASE_URL or ensure postgres is running.")
    });

    println!("═══════════════════════════════════════════════════════════");
    println!("       Cold-Start Latency Benchmark (PostgreSQL)");
    println!("═══════════════════════════════════════════════════════════");
    println!("Database: {}", database_url);
    println!("\nMeasures idle worker → task pickup latency with real DB overhead.");
    println!("This is what users experience when sending a message to an idle agent.\n");

    // PostgreSQL baseline - same parameters as in-memory version
    let baseline = rt.block_on(run_cold_start_scenario(
        pool.clone(),
        ColdStartConfig {
            iterations: 20,
            worker_count: 1,
            check_interval: Duration::from_millis(100),
            name: "db_cold_start_baseline".to_string(),
        },
    ));

    // Summary
    println!("\n═══════════════════════════════════════════════════════════");
    println!("                    Summary");
    println!("═══════════════════════════════════════════════════════════");

    let s2s = baseline.schedule_to_start.summary();
    println!("\nCold-start latency (idle → work pickup) with PostgreSQL:");
    println!("   P50: {:.2}ms", s2s.p50.as_secs_f64() * 1000.0);
    println!("   P99: {:.2}ms", s2s.p99.as_secs_f64() * 1000.0);
    println!("   Max: {:.2}ms", s2s.max.as_secs_f64() * 1000.0);

    // Target guidance (slightly relaxed for DB overhead)
    let p99_ms = s2s.p99.as_secs_f64() * 1000.0;
    if p99_ms < 100.0 {
        println!("\n   ✅ P99 < 100ms: Excellent responsiveness (with DB)");
    } else if p99_ms < 300.0 {
        println!(
            "\n   ⚠️  P99 {:.0}ms: Acceptable, but check DB latency",
            p99_ms
        );
    } else {
        println!(
            "\n   ❌ P99 {:.0}ms: High latency - investigate DB connection",
            p99_ms
        );
    }

    // Generate HTML report
    println!("\n📊 Generating HTML report...");

    let report_config = ReportConfig {
        title: "Cold-Start Latency Benchmark (PostgreSQL)".to_string(),
        filename_prefix: Some("db_cold_start".to_string()),
        ..Default::default()
    };

    let report = BenchmarkReport::new(report_config);
    match report.generate(&baseline) {
        Ok(path) => println!("   ✅ {}", path),
        Err(e) => println!("   ❌ {}", e),
    }

    // Save checkpoint
    if opts.save_checkpoint {
        println!("\n💾 Saving checkpoint...");

        let env = match &opts.moniker {
            Some(m) => EnvironmentInfo::detect_with_moniker(m),
            None => EnvironmentInfo::detect(),
        };
        let store = CheckpointStore::new(format!(
            "{}/benches/checkpoints",
            env!("CARGO_MANIFEST_DIR")
        ));

        let checkpoint =
            BenchmarkCheckpoint::new("db_cold_start_baseline", env, baseline.snapshot());
        match store.save(&checkpoint) {
            Ok(path) => println!("   ✅ {}", path.display()),
            Err(e) => println!("   ❌ {}", e),
        }
    } else {
        println!("\n💡 Tip: Use --save to save checkpoint for historical comparison");
    }

    println!("\n═══════════════════════════════════════════════════════════");
}

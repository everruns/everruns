//! Cold-start latency benchmark
//!
//! Measures the time from task enqueue to task execution start when the worker
//! is idle. This directly measures the impact of poll interval on user-perceived
//! responsiveness.
//!
//! Usage:
//!   cargo bench -p everruns-durable --bench cold_start_latency
//!   cargo bench -p everruns-durable --bench cold_start_latency -- --save
//!   cargo bench -p everruns-durable --bench cold_start_latency -- --save --moniker ci-4cpu-8gb

use std::env;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use tokio::runtime::Runtime;
use tokio::sync::Notify;

use everruns_durable::bench::{
    BenchmarkCheckpoint, BenchmarkMetrics, BenchmarkReport, CheckpointStore, EnvironmentInfo,
    ReportConfig,
};
use everruns_durable::persistence::{InMemoryWorkflowEventStore, TaskDefinition, WorkflowEventStore};
use everruns_durable::workflow::ActivityOptions;
use uuid::Uuid;

/// Configuration for a cold-start test
struct ColdStartConfig {
    /// Number of iterations to run
    iterations: usize,
    /// Number of workers polling for tasks
    worker_count: usize,
    /// Simulated poll interval (how often workers check for tasks)
    poll_interval: Duration,
    /// Name for this scenario
    name: String,
}

/// Run a single cold-start measurement
///
/// Returns the duration from enqueue to task claim (schedule-to-start latency)
async fn measure_single_cold_start(
    store: &Arc<InMemoryWorkflowEventStore>,
    workflow_id: Uuid,
    task_num: usize,
    _worker_ready: &Arc<Notify>,
    task_enqueued: &Arc<Notify>,
    task_claimed: &Arc<AtomicBool>,
    claim_time: &Arc<parking_lot::Mutex<Option<Instant>>>,
) -> Duration {
    // Signal that we're about to enqueue
    task_claimed.store(false, Ordering::SeqCst);

    // Wait for worker to be in idle polling state
    tokio::time::sleep(Duration::from_millis(10)).await;

    // Record enqueue time and enqueue task
    let enqueue_time = Instant::now();
    store
        .enqueue_task(TaskDefinition {
            workflow_id,
            activity_id: format!("cold-start-{}", task_num),
            activity_type: "cold_start_activity".to_string(),
            input: serde_json::json!({ "task_num": task_num }),
            options: ActivityOptions::default(),
        })
        .await
        .unwrap();

    // Notify worker that task is available
    task_enqueued.notify_one();

    // Wait for worker to claim the task
    while !task_claimed.load(Ordering::SeqCst) {
        tokio::time::sleep(Duration::from_micros(10)).await;
    }

    // Get the claim time recorded by the worker
    let claimed_at = claim_time.lock().take().unwrap();
    claimed_at.duration_since(enqueue_time)
}

/// Run cold-start latency scenario
async fn run_cold_start_scenario(config: ColdStartConfig) -> Arc<BenchmarkMetrics> {
    let metrics = Arc::new(BenchmarkMetrics::new(&config.name));
    let store = Arc::new(InMemoryWorkflowEventStore::new());
    let workflow_id = Uuid::now_v7();

    // Create workflow
    store
        .create_workflow(workflow_id, "cold_start_benchmark", serde_json::json!({}), None)
        .await
        .unwrap();

    println!("\n🚀 Running: {}", config.name);
    println!(
        "   Iterations: {}, Workers: {}, Poll interval: {:?}",
        config.iterations, config.worker_count, config.poll_interval
    );

    // Synchronization primitives
    let worker_ready = Arc::new(Notify::new());
    let task_enqueued = Arc::new(Notify::new());
    let task_claimed = Arc::new(AtomicBool::new(false));
    let claim_time = Arc::new(parking_lot::Mutex::new(None));
    let shutdown = Arc::new(AtomicBool::new(false));
    let tasks_completed = Arc::new(AtomicU64::new(0));

    // Spawn worker(s) that poll with the configured interval
    let mut worker_handles = Vec::new();
    for worker_id in 0..config.worker_count {
        let store = store.clone();
        let poll_interval = config.poll_interval;
        let task_claimed = task_claimed.clone();
        let claim_time = claim_time.clone();
        let shutdown = shutdown.clone();
        let tasks_completed = tasks_completed.clone();

        worker_handles.push(tokio::spawn(async move {
            let worker_name = format!("cold-start-worker-{}", worker_id);

            loop {
                if shutdown.load(Ordering::SeqCst) {
                    break;
                }

                // Poll for tasks (simulating real worker behavior)
                let claimed = store
                    .claim_task(&worker_name, &["cold_start_activity".to_string()], 1)
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

                // Wait for poll interval before next check
                tokio::time::sleep(poll_interval).await;
            }
        }));
    }

    // Run iterations
    let start = Instant::now();
    let mut latencies = Vec::with_capacity(config.iterations);

    for i in 0..config.iterations {
        let latency = measure_single_cold_start(
            &store,
            workflow_id,
            i,
            &worker_ready,
            &task_enqueued,
            &task_claimed,
            &claim_time,
        )
        .await;

        latencies.push(latency);
        metrics.schedule_to_start.record(latency);
        metrics.end_to_end.record(latency); // For cold start, S2S == E2E
        metrics.tasks_completed.increment();

        // Brief pause between iterations to ensure worker returns to idle
        tokio::time::sleep(config.poll_interval * 2).await;
    }

    // Shutdown workers
    shutdown.store(true, Ordering::SeqCst);
    for handle in worker_handles {
        handle.abort();
    }

    let elapsed = start.elapsed();

    // Calculate statistics
    latencies.sort();
    let p50 = latencies[latencies.len() / 2];
    let p99 = latencies[(latencies.len() as f64 * 0.99) as usize];
    let min = latencies[0];
    let max = latencies[latencies.len() - 1];
    let avg: Duration = latencies.iter().sum::<Duration>() / latencies.len() as u32;

    println!("✅ Completed {} iterations in {:.2}s", config.iterations, elapsed.as_secs_f64());
    println!(
        "   Cold-start latency: min={:.2}ms avg={:.2}ms p50={:.2}ms p99={:.2}ms max={:.2}ms",
        min.as_secs_f64() * 1000.0,
        avg.as_secs_f64() * 1000.0,
        p50.as_secs_f64() * 1000.0,
        p99.as_secs_f64() * 1000.0,
        max.as_secs_f64() * 1000.0,
    );

    // Interpretation
    let poll_ms = config.poll_interval.as_secs_f64() * 1000.0;
    let expected_avg = poll_ms / 2.0; // Uniform distribution expectation
    let actual_avg = avg.as_secs_f64() * 1000.0;
    println!(
        "   💡 Expected avg ~{:.1}ms (poll/2), actual {:.1}ms",
        expected_avg, actual_avg
    );

    if p99.as_secs_f64() * 1000.0 <= poll_ms * 1.1 {
        println!("   ✅ P99 within expected bounds (poll interval)");
    } else {
        println!(
            "   ⚠️  P99 {:.1}ms exceeds poll interval {:.1}ms",
            p99.as_secs_f64() * 1000.0,
            poll_ms
        );
    }

    metrics
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

    println!("═══════════════════════════════════════════════════════════");
    println!("           Cold-Start Latency Benchmark");
    println!("═══════════════════════════════════════════════════════════");
    println!("\nMeasures idle worker → task pickup latency");
    println!("This is what users experience when sending a message to an idle agent.\n");

    // Scenario 1: Current production setting (100ms poll)
    let poll_100ms = rt.block_on(run_cold_start_scenario(ColdStartConfig {
        iterations: 100,
        worker_count: 1,
        poll_interval: Duration::from_millis(100),
        name: "cold_start_100ms_poll".to_string(),
    }));

    // Scenario 2: Previous production setting (1s poll) for comparison
    let poll_1s = rt.block_on(run_cold_start_scenario(ColdStartConfig {
        iterations: 50,
        worker_count: 1,
        poll_interval: Duration::from_secs(1),
        name: "cold_start_1s_poll".to_string(),
    }));

    // Scenario 3: Aggressive polling (10ms) - theoretical minimum with polling
    let poll_10ms = rt.block_on(run_cold_start_scenario(ColdStartConfig {
        iterations: 100,
        worker_count: 1,
        poll_interval: Duration::from_millis(10),
        name: "cold_start_10ms_poll".to_string(),
    }));

    // Scenario 4: Multiple workers (should have similar latency, just for verification)
    let multi_worker = rt.block_on(run_cold_start_scenario(ColdStartConfig {
        iterations: 100,
        worker_count: 4,
        poll_interval: Duration::from_millis(100),
        name: "cold_start_4_workers_100ms".to_string(),
    }));

    // Summary
    println!("\n═══════════════════════════════════════════════════════════");
    println!("                    Summary");
    println!("═══════════════════════════════════════════════════════════");
    println!("\nCold-start latency by poll interval:");
    println!(
        "{:<35} {:>12} {:>12} {:>12}",
        "Scenario", "P50", "P99", "Max"
    );
    println!("{:-<35} {:->12} {:->12} {:->12}", "", "", "", "");

    for (name, m) in [
        ("cold_start_10ms_poll", &poll_10ms),
        ("cold_start_100ms_poll", &poll_100ms),
        ("cold_start_1s_poll", &poll_1s),
        ("cold_start_4_workers_100ms", &multi_worker),
    ] {
        let s2s = m.schedule_to_start.summary();
        println!(
            "{:<35} {:>10.2}ms {:>10.2}ms {:>10.2}ms",
            name,
            s2s.p50.as_secs_f64() * 1000.0,
            s2s.p99.as_secs_f64() * 1000.0,
            s2s.max.as_secs_f64() * 1000.0,
        );
    }

    // Calculate improvement
    let old_p50 = poll_1s.schedule_to_start.summary().p50.as_secs_f64() * 1000.0;
    let new_p50 = poll_100ms.schedule_to_start.summary().p50.as_secs_f64() * 1000.0;
    let improvement = ((old_p50 - new_p50) / old_p50) * 100.0;

    println!("\n📊 Impact of 1s → 100ms poll interval change:");
    println!("   P50 latency: {:.1}ms → {:.1}ms ({:.0}% improvement)", old_p50, new_p50, improvement);

    // Generate HTML reports
    println!("\n📊 Generating HTML reports...");

    let report_config = ReportConfig {
        title: "Cold-Start Latency Benchmark".to_string(),
        filename_prefix: Some("cold_start".to_string()),
        ..Default::default()
    };

    for (name, m) in [
        ("cold_start_100ms_poll", &poll_100ms),
        ("cold_start_1s_poll", &poll_1s),
    ] {
        let report = BenchmarkReport::new(report_config.clone());
        match report.generate(m) {
            Ok(path) => println!("   ✅ {}: {}", name, path),
            Err(e) => println!("   ❌ {}: {}", name, e),
        }
    }

    // Save checkpoints
    if opts.save_checkpoint {
        println!("\n💾 Saving checkpoints...");

        let env = match &opts.moniker {
            Some(m) => EnvironmentInfo::detect_with_moniker(m),
            None => EnvironmentInfo::detect(),
        };
        let store = CheckpointStore::new(format!(
            "{}/benches/checkpoints",
            env!("CARGO_MANIFEST_DIR")
        ));

        for (name, m) in [
            ("cold_start_100ms_poll", &poll_100ms),
            ("cold_start_1s_poll", &poll_1s),
            ("cold_start_10ms_poll", &poll_10ms),
        ] {
            let checkpoint = BenchmarkCheckpoint::new(name, env.clone(), m.snapshot());
            match store.save(&checkpoint) {
                Ok(path) => println!("   ✅ {}: {}", name, path.display()),
                Err(e) => println!("   ❌ {}: {}", name, e),
            }
        }
    } else {
        println!("\n💡 Tip: Use --save to save checkpoints for historical comparison");
    }

    println!("\n═══════════════════════════════════════════════════════════");
}

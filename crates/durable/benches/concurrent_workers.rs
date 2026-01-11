//! Concurrent workers load test
//!
//! Tests the durable execution engine under realistic load with multiple workers.
//! Generates HTML reports similar to Gatling.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use indicatif::{ProgressBar, ProgressStyle};
use tokio::runtime::Runtime;
use tokio::sync::Semaphore;

use everruns_durable::bench::{ActivityDuration, BenchmarkMetrics, BenchmarkReport, ReportConfig};
use everruns_durable::persistence::{
    InMemoryWorkflowEventStore, TaskDefinition, WorkflowEventStore,
};
use everruns_durable::workflow::ActivityOptions;
use uuid::Uuid;

/// Shared test scenario state
struct TestScenario {
    store: Arc<InMemoryWorkflowEventStore>,
    workflow_id: Uuid,
    task_count: u64,
    enqueue_times: Arc<parking_lot::Mutex<std::collections::HashMap<Uuid, Instant>>>,
    completed: Arc<AtomicU64>,
    /// Whether to simulate realistic activity durations
    simulate_execution: bool,
    /// Workers claiming tasks
    worker_count: usize,
}

impl TestScenario {
    fn new(task_count: u64, worker_count: usize, simulate_execution: bool) -> Self {
        Self {
            store: Arc::new(InMemoryWorkflowEventStore::new()),
            workflow_id: Uuid::now_v7(),
            task_count,
            enqueue_times: Arc::new(parking_lot::Mutex::new(std::collections::HashMap::new())),
            completed: Arc::new(AtomicU64::new(0)),
            simulate_execution,
            worker_count,
        }
    }

    async fn setup(&self) {
        self.store
            .create_workflow(self.workflow_id, "benchmark", serde_json::json!({}), None)
            .await
            .unwrap();
    }

    async fn enqueue_all_tasks(&self) {
        for i in 0..self.task_count {
            let enqueue_time = Instant::now();
            let task_id = self
                .store
                .enqueue_task(TaskDefinition {
                    workflow_id: self.workflow_id,
                    activity_id: format!("task-{}", i),
                    activity_type: "benchmark_activity".to_string(),
                    input: serde_json::json!({ "task_num": i }),
                    options: ActivityOptions::default(),
                })
                .await
                .unwrap();

            self.enqueue_times.lock().insert(task_id, enqueue_time);
        }
    }

    async fn run_workers(&self, metrics: &BenchmarkMetrics, pb: &ProgressBar) {
        let semaphore = Arc::new(Semaphore::new(self.worker_count));
        let mut handles = Vec::new();

        for worker_id in 0..self.worker_count {
            let store = self.store.clone();
            let enqueue_times = self.enqueue_times.clone();
            let completed = self.completed.clone();
            let task_count = self.task_count;
            let simulate_execution = self.simulate_execution;
            let schedule_to_start = metrics.schedule_to_start.clone();
            let execution = metrics.execution.clone();
            let end_to_end = metrics.end_to_end.clone();
            let tasks_completed = metrics.tasks_completed.clone();
            let semaphore = semaphore.clone();
            let pb = pb.clone();

            handles.push(tokio::spawn(async move {
                let worker_name = format!("worker-{}", worker_id);

                loop {
                    // Check if we're done
                    if completed.load(Ordering::Relaxed) >= task_count {
                        break;
                    }

                    // Acquire permit (concurrency control)
                    let _permit = semaphore.acquire().await.unwrap();

                    // Claim a task
                    let claimed = store
                        .claim_task(&worker_name, &["benchmark_activity".to_string()], 1)
                        .await
                        .unwrap();

                    if claimed.is_empty() {
                        if completed.load(Ordering::Relaxed) >= task_count {
                            break;
                        }
                        // Brief pause before retry
                        tokio::time::sleep(Duration::from_micros(100)).await;
                        continue;
                    }

                    let claim_time = Instant::now();

                    for task in claimed {
                        // Calculate schedule-to-start latency
                        if let Some(enqueue_time) = enqueue_times.lock().get(&task.id).copied() {
                            let s2s = claim_time.duration_since(enqueue_time);
                            schedule_to_start.record(s2s);
                        }

                        // Simulate activity execution
                        let exec_start = Instant::now();
                        if simulate_execution {
                            let duration = ActivityDuration::sample();
                            // Cap at 100ms for benchmark speed
                            let capped = duration.min(Duration::from_millis(100));
                            tokio::time::sleep(capped).await;
                        }
                        let exec_time = exec_start.elapsed();
                        execution.record(exec_time);

                        // Complete task
                        store
                            .complete_task(task.id, serde_json::json!({"ok": true}))
                            .await
                            .unwrap();

                        // Record end-to-end
                        if let Some(enqueue_time) = enqueue_times.lock().get(&task.id).copied() {
                            let e2e = Instant::now().duration_since(enqueue_time);
                            end_to_end.record(e2e);
                        }

                        tasks_completed.increment();
                        let current = completed.fetch_add(1, Ordering::Relaxed) + 1;
                        pb.set_position(current);
                    }
                }
            }));
        }

        for handle in handles {
            handle.await.unwrap();
        }
    }
}

/// Run a single load test scenario
async fn run_scenario(
    name: &str,
    task_count: u64,
    worker_count: usize,
    simulate_execution: bool,
) -> Arc<BenchmarkMetrics> {
    let metrics = Arc::new(BenchmarkMetrics::new(name));
    let scenario = TestScenario::new(task_count, worker_count, simulate_execution);

    println!("\n🚀 Running: {}", name);
    println!(
        "   Tasks: {}, Workers: {}, Simulate execution: {}",
        task_count, worker_count, simulate_execution
    );

    // Setup
    scenario.setup().await;

    // Enqueue all tasks first (burst load)
    let enqueue_start = Instant::now();
    scenario.enqueue_all_tasks().await;
    let enqueue_time = enqueue_start.elapsed();
    println!(
        "   Enqueued {} tasks in {:.2}ms ({:.0} tasks/sec)",
        task_count,
        enqueue_time.as_secs_f64() * 1000.0,
        task_count as f64 / enqueue_time.as_secs_f64()
    );

    // Create progress bar
    let pb = ProgressBar::new(task_count);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("   {spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({per_sec})")
            .unwrap()
            .progress_chars("=>-"),
    );

    // Start metrics sampling
    let metrics_clone = metrics.clone();
    let sampling_handle = tokio::spawn(async move {
        loop {
            metrics_clone.sample();
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    });

    // Run workers
    let run_start = Instant::now();
    scenario.run_workers(&metrics, &pb).await;
    let run_time = run_start.elapsed();

    sampling_handle.abort();
    metrics.sample(); // Final sample
    pb.finish_and_clear();

    // Print summary
    let e2e = metrics.end_to_end.summary();
    let s2s = metrics.schedule_to_start.summary();
    let exec = metrics.execution.summary();

    println!("✅ Completed in {:.2}s", run_time.as_secs_f64());
    println!(
        "   Throughput:      {:.1} tasks/sec    (sustained processing rate)",
        task_count as f64 / run_time.as_secs_f64()
    );
    println!(
        "   Schedule-to-Start: P50={:.2}ms P99={:.2}ms    (queue wait time)",
        s2s.p50.as_secs_f64() * 1000.0,
        s2s.p99.as_secs_f64() * 1000.0
    );
    println!(
        "   End-to-End:        P50={:.2}ms P99={:.2}ms    (total latency)",
        e2e.p50.as_secs_f64() * 1000.0,
        e2e.p99.as_secs_f64() * 1000.0
    );

    // Interpretation
    let s2s_p99_ms = s2s.p99.as_secs_f64() * 1000.0;
    if s2s_p99_ms < 10.0 {
        println!("   💡 S2S P99 < 10ms: Excellent job pickup latency");
    } else if s2s_p99_ms < 50.0 {
        println!(
            "   💡 S2S P99 {:.1}ms: Consider adding more workers",
            s2s_p99_ms
        );
    }

    let overhead_ms =
        (e2e.p50.as_secs_f64() - s2s.p50.as_secs_f64() - exec.p50.as_secs_f64()) * 1000.0;
    if overhead_ms > 5.0 {
        println!(
            "   💡 Scheduling overhead {:.1}ms: May indicate contention",
            overhead_ms.max(0.0)
        );
    }

    metrics
}

fn main() {
    let rt = Runtime::new().unwrap();

    println!("═══════════════════════════════════════════════════════════");
    println!("           Durable Execution Engine Load Test");
    println!("═══════════════════════════════════════════════════════════");

    // Scenario 1: Baseline - single worker, no execution simulation
    let baseline = rt.block_on(run_scenario("baseline_1_worker", 10_000, 1, false));

    // Scenario 2: Worker scaling (no execution)
    let scale_10 = rt.block_on(run_scenario("scale_10_workers", 10_000, 10, false));
    let scale_50 = rt.block_on(run_scenario("scale_50_workers", 10_000, 50, false));
    let scale_100 = rt.block_on(run_scenario("scale_100_workers", 10_000, 100, false));

    // Scenario 3: Realistic execution (with simulated I/O wait)
    let realistic_10 = rt.block_on(run_scenario("realistic_10_workers", 1_000, 10, true));
    let realistic_50 = rt.block_on(run_scenario("realistic_50_workers", 1_000, 50, true));
    let realistic_100 = rt.block_on(run_scenario("realistic_100_workers", 1_000, 100, true));

    // Scenario 4: High volume burst
    let burst = rt.block_on(run_scenario("burst_50k_tasks", 50_000, 100, false));

    println!("\n═══════════════════════════════════════════════════════════");
    println!("                    Summary");
    println!("═══════════════════════════════════════════════════════════");
    println!("\nMetric definitions:");
    println!("  Throughput: Tasks processed per second (higher is better)");
    println!("  P50 S2S:    Median schedule-to-start latency (lower is better)");
    println!("  P99 S2S:    99th percentile S2S - tail latency (target: <10ms)");

    // Print comparison table
    println!(
        "\n{:<30} {:>12} {:>12} {:>12}",
        "Scenario", "Throughput", "P50 S2S", "P99 S2S"
    );
    println!("{:-<30} {:->12} {:->12} {:->12}", "", "", "", "");

    for (name, m) in [
        ("baseline_1_worker", &baseline),
        ("scale_10_workers", &scale_10),
        ("scale_50_workers", &scale_50),
        ("scale_100_workers", &scale_100),
        ("realistic_10_workers", &realistic_10),
        ("realistic_50_workers", &realistic_50),
        ("realistic_100_workers", &realistic_100),
        ("burst_50k_tasks", &burst),
    ] {
        let throughput = m.tasks_completed.throughput();
        let s2s = m.schedule_to_start.summary();
        println!(
            "{:<30} {:>10.1}/s {:>10.2}ms {:>10.2}ms",
            name,
            throughput,
            s2s.p50.as_secs_f64() * 1000.0,
            s2s.p99.as_secs_f64() * 1000.0
        );
    }

    // Generate HTML reports
    println!("\n📊 Generating HTML reports...");

    let report_config = ReportConfig {
        output_dir: "target/benchmark-reports".to_string(),
        title: "Durable Execution Benchmark".to_string(),
        include_raw_data: false,
    };

    for (name, m) in [
        ("baseline_1_worker", &baseline),
        ("scale_100_workers", &scale_100),
        ("realistic_100_workers", &realistic_100),
        ("burst_50k_tasks", &burst),
    ] {
        let report = BenchmarkReport::new(report_config.clone());
        match report.generate(m) {
            Ok(path) => println!("   ✅ {}: {}", name, path),
            Err(e) => println!("   ❌ {}: {}", name, e),
        }
    }

    println!("\n═══════════════════════════════════════════════════════════");
}

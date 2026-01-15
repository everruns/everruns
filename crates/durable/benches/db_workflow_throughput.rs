//! Workflow throughput benchmark with PostgreSQL backend
//!
//! Tests the target scenario with real database persistence: thousands of parallel
//! workflows, each with many sequential activities.
//!
//! Requirements:
//!   - PostgreSQL 17 running with DATABASE_URL set
//!   - Migrations applied from crates/control-plane/migrations/
//!
//! Usage:
//!   cargo bench -p everruns-durable --bench db_workflow_throughput
//!   cargo bench -p everruns-durable --bench db_workflow_throughput -- --save
//!   cargo bench -p everruns-durable --bench db_workflow_throughput -- --save --moniker ci-4cpu-8gb

use std::env;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use indicatif::{ProgressBar, ProgressStyle};
use sqlx::PgPool;
use tokio::runtime::Runtime;

use everruns_durable::bench::{
    BenchmarkCheckpoint, BenchmarkMetrics, BenchmarkReport, CheckpointStore, EnvironmentInfo,
    ReportConfig, clear_terminal_progress, set_terminal_progress,
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

/// Workflow state tracking
struct WorkflowState {
    id: Uuid,
    current_step: AtomicU64,
    total_steps: u64,
    completed: std::sync::atomic::AtomicBool,
}

/// Multi-workflow scenario with PostgreSQL backend
struct DbWorkflowScenario {
    store: Arc<PostgresWorkflowEventStore>,
    workflows: Vec<Arc<WorkflowState>>,
    workflow_count: usize,
    steps_per_workflow: u64,
    worker_count: usize,
    /// Activity type for routing
    activity_type: String,
    /// Track task enqueue times for latency measurement
    enqueue_times: Arc<parking_lot::Mutex<std::collections::HashMap<Uuid, Instant>>>,
}

impl DbWorkflowScenario {
    fn new(pool: PgPool, workflow_count: usize, steps_per_workflow: u64, worker_count: usize) -> Self {
        Self {
            store: Arc::new(PostgresWorkflowEventStore::new(pool)),
            workflows: Vec::new(),
            workflow_count,
            steps_per_workflow,
            worker_count,
            activity_type: "db_workflow_step".to_string(),
            enqueue_times: Arc::new(parking_lot::Mutex::new(std::collections::HashMap::new())),
        }
    }

    async fn setup(&mut self) {
        // Create all workflows
        for _ in 0..self.workflow_count {
            let workflow_id = Uuid::now_v7();

            self.store
                .create_workflow(
                    workflow_id,
                    "db_benchmark_workflow",
                    serde_json::json!({ "steps": self.steps_per_workflow }),
                    None,
                )
                .await
                .expect("Failed to create workflow");

            self.workflows.push(Arc::new(WorkflowState {
                id: workflow_id,
                current_step: AtomicU64::new(0),
                total_steps: self.steps_per_workflow,
                completed: std::sync::atomic::AtomicBool::new(false),
            }));
        }
    }

    /// Enqueue the first step for all workflows
    async fn start_workflows(&self) {
        for workflow in &self.workflows {
            self.enqueue_step(workflow, 0).await;
        }
    }

    /// Enqueue a step for a workflow
    async fn enqueue_step(&self, workflow: &WorkflowState, step: u64) {
        let enqueue_time = Instant::now();
        let task_id = self
            .store
            .enqueue_task(TaskDefinition {
                workflow_id: workflow.id,
                activity_id: format!("step-{}", step),
                activity_type: self.activity_type.clone(),
                input: serde_json::json!({ "step": step }),
                options: ActivityOptions::default(),
            })
            .await
            .expect("Failed to enqueue task");

        self.enqueue_times.lock().insert(task_id, enqueue_time);
    }

    /// Run workers that process tasks and advance workflows
    async fn run(
        &self,
        metrics: &BenchmarkMetrics,
        simulate_execution: bool,
        pb: &ProgressBar,
    ) -> (u64, Duration) {
        let start = Instant::now();
        let completed_workflows = Arc::new(AtomicU64::new(0));
        let total_tasks_completed = Arc::new(AtomicU64::new(0));

        let mut handles = Vec::new();

        for worker_id in 0..self.worker_count {
            let store = self.store.clone();
            let workflows = self.workflows.clone();
            let enqueue_times = self.enqueue_times.clone();
            let completed_workflows = completed_workflows.clone();
            let total_tasks_completed = total_tasks_completed.clone();
            let activity_type = self.activity_type.clone();
            let workflow_count = self.workflow_count;
            let schedule_to_start = metrics.schedule_to_start.clone();
            let execution = metrics.execution.clone();
            let end_to_end = metrics.end_to_end.clone();
            let tasks_completed_counter = metrics.tasks_completed.clone();
            let pb = pb.clone();

            handles.push(tokio::spawn(async move {
                let worker_name = format!("db-wf-worker-{}", worker_id);

                loop {
                    // Check if all workflows are done
                    if completed_workflows.load(Ordering::Relaxed) >= workflow_count as u64 {
                        break;
                    }

                    // Claim a task
                    let claimed = store
                        .claim_task(&worker_name, std::slice::from_ref(&activity_type), 1)
                        .await
                        .unwrap();

                    if claimed.is_empty() {
                        if completed_workflows.load(Ordering::Relaxed) >= workflow_count as u64 {
                            break;
                        }
                        tokio::time::sleep(Duration::from_micros(50)).await;
                        continue;
                    }

                    let claim_time = Instant::now();

                    for task in claimed {
                        // Record schedule-to-start
                        if let Some(enqueue_time) = enqueue_times.lock().get(&task.id).copied() {
                            schedule_to_start.record(claim_time.duration_since(enqueue_time));
                        }

                        // Simulate execution
                        let exec_start = Instant::now();
                        if simulate_execution {
                            // Use faster durations for benchmark (1-10ms instead of 100ms+)
                            let duration =
                                Duration::from_micros(1000 + rand::random::<u64>() % 9000);
                            tokio::time::sleep(duration).await;
                        }
                        execution.record(exec_start.elapsed());

                        // Complete task (pass worker_name to verify ownership)
                        store
                            .complete_task(task.id, &worker_name, serde_json::json!({"ok": true}))
                            .await
                            .unwrap();

                        // Record end-to-end
                        if let Some(enqueue_time) = enqueue_times.lock().get(&task.id).copied() {
                            end_to_end.record(Instant::now().duration_since(enqueue_time));
                        }

                        tasks_completed_counter.increment();
                        let current = total_tasks_completed.fetch_add(1, Ordering::Relaxed) + 1;
                        pb.set_position(current);

                        // Update terminal progress (Ghostty, iTerm2, etc.)
                        if let Some(total) = pb.length() {
                            let percent = ((current as f64 / total as f64) * 100.0) as u8;
                            set_terminal_progress(percent);
                        }

                        // Find the workflow and advance it
                        if let Some(workflow) = workflows.iter().find(|w| w.id == task.workflow_id)
                        {
                            let current = workflow.current_step.fetch_add(1, Ordering::SeqCst);
                            let next_step = current + 1;

                            if next_step >= workflow.total_steps {
                                // Workflow complete
                                workflow.completed.store(true, Ordering::Release);
                                completed_workflows.fetch_add(1, Ordering::Relaxed);
                            } else {
                                // Enqueue next step
                                let enqueue_time = Instant::now();
                                let next_task_id = store
                                    .enqueue_task(TaskDefinition {
                                        workflow_id: workflow.id,
                                        activity_id: format!("step-{}", next_step),
                                        activity_type: activity_type.clone(),
                                        input: serde_json::json!({ "step": next_step }),
                                        options: ActivityOptions::default(),
                                    })
                                    .await
                                    .unwrap();

                                enqueue_times.lock().insert(next_task_id, enqueue_time);
                            }
                        }
                    }
                }
            }));
        }

        for handle in handles {
            handle.await.unwrap();
        }

        let elapsed = start.elapsed();
        let total = total_tasks_completed.load(Ordering::Relaxed);

        (total, elapsed)
    }

    /// Cleanup all test data
    async fn cleanup(&self) {
        let pool = self.store.pool();

        for workflow in &self.workflows {
            // Delete in reverse dependency order
            sqlx::query("DELETE FROM durable_signals WHERE workflow_id = $1")
                .bind(workflow.id)
                .execute(pool)
                .await
                .ok();
            sqlx::query("DELETE FROM durable_dead_letter_queue WHERE workflow_id = $1")
                .bind(workflow.id)
                .execute(pool)
                .await
                .ok();
            sqlx::query("DELETE FROM durable_task_queue WHERE workflow_id = $1")
                .bind(workflow.id)
                .execute(pool)
                .await
                .ok();
            sqlx::query("DELETE FROM durable_workflow_events WHERE workflow_id = $1")
                .bind(workflow.id)
                .execute(pool)
                .await
                .ok();
            sqlx::query("DELETE FROM durable_workflow_instances WHERE id = $1")
                .bind(workflow.id)
                .execute(pool)
                .await
                .ok();
        }
    }
}

/// Run a workflow throughput test with PostgreSQL
async fn run_db_workflow_test(
    pool: PgPool,
    name: &str,
    workflow_count: usize,
    steps_per_workflow: u64,
    worker_count: usize,
    simulate_execution: bool,
) -> Arc<BenchmarkMetrics> {
    let metrics = Arc::new(BenchmarkMetrics::new(name));
    let total_tasks = workflow_count as u64 * steps_per_workflow;

    println!("\n🗄️  Running (PostgreSQL): {}", name);
    println!(
        "   Workflows: {}, Steps/workflow: {}, Workers: {}",
        workflow_count, steps_per_workflow, worker_count
    );
    println!("   Total tasks: {}", total_tasks);

    let mut scenario = DbWorkflowScenario::new(pool, workflow_count, steps_per_workflow, worker_count);

    // Setup
    scenario.setup().await;

    // Create progress bar
    let pb = ProgressBar::new(total_tasks);
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

    // Start all workflows (enqueue first step)
    scenario.start_workflows().await;

    // Run until all workflows complete
    let (completed_tasks, elapsed) = scenario.run(&metrics, simulate_execution, &pb).await;

    sampling_handle.abort();
    metrics.sample();
    pb.finish_and_clear();
    clear_terminal_progress();

    // Cleanup test data
    scenario.cleanup().await;

    // Summary
    let e2e = metrics.end_to_end.summary();
    let s2s = metrics.schedule_to_start.summary();

    println!(
        "✅ Completed {} workflows in {:.2}s",
        workflow_count,
        elapsed.as_secs_f64()
    );
    println!(
        "   Task throughput:     {:.1} tasks/sec    (sustained task processing)",
        completed_tasks as f64 / elapsed.as_secs_f64()
    );
    println!(
        "   Workflow throughput: {:.1} workflows/sec    (end-to-end workflow completion)",
        workflow_count as f64 / elapsed.as_secs_f64()
    );
    println!(
        "   Schedule-to-Start:   P50={:.2}ms P99={:.2}ms    (queue wait time)",
        s2s.p50.as_secs_f64() * 1000.0,
        s2s.p99.as_secs_f64() * 1000.0
    );
    println!(
        "   End-to-End (task):   P50={:.2}ms P99={:.2}ms    (per-task latency)",
        e2e.p50.as_secs_f64() * 1000.0,
        e2e.p99.as_secs_f64() * 1000.0
    );

    // Interpretation
    let s2s_p99_ms = s2s.p99.as_secs_f64() * 1000.0;
    if s2s_p99_ms < 10.0 {
        println!("   💡 S2S P99 < 10ms: Excellent - tasks picked up instantly");
    } else if s2s_p99_ms < 50.0 {
        println!(
            "   💡 S2S P99 {:.1}ms: Good, but could add more workers",
            s2s_p99_ms
        );
    } else {
        println!(
            "   💡 S2S P99 {:.1}ms: High queue wait - workers are backlogged",
            s2s_p99_ms
        );
    }

    let overhead_ms = (e2e.p50.as_secs_f64()
        - s2s.p50.as_secs_f64()
        - metrics.execution.summary().p50.as_secs_f64())
        * 1000.0;
    if overhead_ms > 5.0 {
        println!(
            "   💡 Scheduling overhead {:.1}ms: Check for DB contention",
            overhead_ms.max(0.0)
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

    // Connect to PostgreSQL
    let database_url = get_database_url();
    let pool = rt.block_on(async {
        PgPool::connect(&database_url)
            .await
            .expect("Failed to connect to PostgreSQL. Set DATABASE_URL or ensure postgres is running.")
    });

    println!("═══════════════════════════════════════════════════════════");
    println!("       Workflow Throughput Benchmark (PostgreSQL)");
    println!("═══════════════════════════════════════════════════════════");
    println!("Database: {}", database_url);
    println!("\nThis benchmark simulates the target scenario:");
    println!("  - Parallel workflows with sequential steps (activities)");
    println!("  - Workers claim and execute tasks, advancing workflows");
    println!("  - Real PostgreSQL persistence for production-like metrics");

    // Reduced task counts compared to in-memory due to DB overhead
    // Scenario 1: Small scale (baseline)
    let small = rt.block_on(run_db_workflow_test(
        pool.clone(),
        "db_small_10wf_10steps",
        10,    // workflows
        10,    // steps per workflow (100 total tasks)
        10,    // workers
        false, // no execution simulation
    ));

    // Scenario 2: Medium scale
    let medium = rt.block_on(run_db_workflow_test(
        pool.clone(),
        "db_medium_50wf_20steps",
        50, // workflows
        20, // steps per workflow (1,000 total tasks)
        50, // workers
        false,
    ));

    // Scenario 3: Target scale (reduced from in-memory)
    let target = rt.block_on(run_db_workflow_test(
        pool.clone(),
        "db_target_100wf_50steps",
        100, // workflows
        50,  // steps per workflow (5,000 total tasks)
        100, // workers
        false,
    ));

    // Scenario 4: Target scale with execution simulation
    let target_exec = rt.block_on(run_db_workflow_test(
        pool.clone(),
        "db_target_100wf_50steps_exec",
        100, // workflows
        50,  // steps per workflow
        100, // workers
        true, // simulate execution (1-10ms per task)
    ));

    // Scenario 5: High parallelism (more workflows, fewer steps)
    let high_parallel = rt.block_on(run_db_workflow_test(
        pool.clone(),
        "db_parallel_500wf_10steps",
        500, // workflows
        10,  // steps per workflow (5,000 total tasks)
        200, // workers
        false,
    ));

    // Scenario 6: Deep workflows (fewer workflows, many steps)
    let deep = rt.block_on(run_db_workflow_test(
        pool.clone(),
        "db_deep_20wf_100steps",
        20,  // workflows
        100, // steps per workflow (2,000 total tasks)
        50,  // workers
        false,
    ));

    println!("\n═══════════════════════════════════════════════════════════");
    println!("                    Summary");
    println!("═══════════════════════════════════════════════════════════");
    println!("\nMetric definitions:");
    println!("  Tasks/sec: Total task throughput (higher is better)");
    println!("  WF/sec:    Workflow completion rate (end-to-end)");
    println!("  P50 S2S:   Median schedule-to-start latency (lower is better)");
    println!("  P99 S2S:   99th percentile S2S - tail latency (target: <10ms)");

    println!(
        "\n{:<30} {:>12} {:>12} {:>12} {:>12}",
        "Scenario", "Tasks/sec", "WF/sec", "P50 S2S", "P99 S2S"
    );
    println!(
        "{:-<30} {:->12} {:->12} {:->12} {:->12}",
        "", "", "", "", ""
    );

    for (name, m, wf_count) in [
        ("db_small_10wf_10steps", &small, 10),
        ("db_medium_50wf_20steps", &medium, 50),
        ("db_target_100wf_50steps", &target, 100),
        ("db_target_100wf_50steps_exec", &target_exec, 100),
        ("db_parallel_500wf_10steps", &high_parallel, 500),
        ("db_deep_20wf_100steps", &deep, 20),
    ] {
        let task_throughput = m.tasks_completed.throughput();
        let wf_throughput = wf_count as f64 / m.elapsed().as_secs_f64();
        let s2s = m.schedule_to_start.summary();
        println!(
            "{:<30} {:>10.1}/s {:>10.1}/s {:>10.2}ms {:>10.2}ms",
            name,
            task_throughput,
            wf_throughput,
            s2s.p50.as_secs_f64() * 1000.0,
            s2s.p99.as_secs_f64() * 1000.0
        );
    }

    // Generate HTML reports for key scenarios
    println!("\n📊 Generating HTML reports...");

    let report_config = ReportConfig {
        title: "Workflow Throughput Benchmark (PostgreSQL)".to_string(),
        filename_prefix: Some("db_workflow_throughput".to_string()),
        ..Default::default()
    };

    for (name, m) in [
        ("db_target_100wf_50steps", &target),
        ("db_target_100wf_50steps_exec", &target_exec),
        ("db_parallel_500wf_10steps", &high_parallel),
    ] {
        let report = BenchmarkReport::new(report_config.clone());
        match report.generate(m) {
            Ok(path) => println!("   ✅ {}: {}", name, path),
            Err(e) => println!("   ❌ {}: {}", name, e),
        }
    }

    // Save checkpoints for historical comparison (only with --save flag)
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
            ("db_workflow_throughput_target_100x50", &target),
            ("db_workflow_throughput_target_100x50_exec", &target_exec),
            ("db_workflow_throughput_parallel_500x10", &high_parallel),
            ("db_workflow_throughput_deep_20x100", &deep),
        ] {
            let checkpoint = BenchmarkCheckpoint::new(name, env.clone(), m.snapshot());
            match store.save(&checkpoint) {
                Ok(path) => println!("   ✅ {}: {}", name, path.display()),
                Err(e) => println!("   ❌ {}: {}", name, e),
            }
        }

        // Show comparison with previous runs if available
        println!("\n📊 Historical comparison (vs last run):");
        for name in [
            "db_workflow_throughput_target_100x50",
            "db_workflow_throughput_parallel_500x10",
        ] {
            match store.get_comparison(name, Some(&env.moniker), 2) {
                Ok(checkpoints) if checkpoints.len() >= 2 => {
                    let comparison = everruns_durable::bench::CheckpointComparison::compare(
                        &checkpoints[0],
                        &checkpoints[1],
                    );
                    println!("\n   {}", name);
                    println!(
                        "      Throughput: {:.1} → {:.1} ({:+.1}%)",
                        comparison.baseline.throughput,
                        comparison.current.throughput,
                        comparison.changes.throughput_pct
                    );
                    println!(
                        "      E2E P99:    {:.2}ms → {:.2}ms ({:+.1}%)",
                        comparison.baseline.e2e_p99_ms,
                        comparison.current.e2e_p99_ms,
                        comparison.changes.e2e_p99_pct
                    );
                }
                Ok(_) => println!("   {} - first run, no baseline yet", name),
                Err(e) => println!("   {} - {}", name, e),
            }
        }
    } else {
        println!("\n💡 Tip: Use --save to save checkpoints for historical comparison");
    }

    println!("\n═══════════════════════════════════════════════════════════");
}

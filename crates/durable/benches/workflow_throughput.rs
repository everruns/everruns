//! Workflow throughput benchmark
//!
//! Tests the target scenario: thousands of parallel workflows,
//! each with many sequential activities.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::runtime::Runtime;

use everruns_durable::bench::{BenchmarkMetrics, BenchmarkReport, ReportConfig};
use everruns_durable::persistence::{
    InMemoryWorkflowEventStore, TaskDefinition, WorkflowEventStore,
};
use everruns_durable::workflow::ActivityOptions;
use uuid::Uuid;

/// Workflow state tracking
struct WorkflowState {
    id: Uuid,
    current_step: AtomicU64,
    total_steps: u64,
    completed: std::sync::atomic::AtomicBool,
}

/// Multi-workflow scenario
struct WorkflowScenario {
    store: Arc<InMemoryWorkflowEventStore>,
    workflows: Vec<Arc<WorkflowState>>,
    workflow_count: usize,
    steps_per_workflow: u64,
    worker_count: usize,
    /// Activity type for routing
    activity_type: String,
    /// Track task enqueue times for latency measurement
    enqueue_times: Arc<parking_lot::Mutex<std::collections::HashMap<Uuid, Instant>>>,
}

impl WorkflowScenario {
    fn new(workflow_count: usize, steps_per_workflow: u64, worker_count: usize) -> Self {
        Self {
            store: Arc::new(InMemoryWorkflowEventStore::new()),
            workflows: Vec::new(),
            workflow_count,
            steps_per_workflow,
            worker_count,
            activity_type: "workflow_step".to_string(),
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
                    "benchmark_workflow",
                    serde_json::json!({ "steps": self.steps_per_workflow }),
                    None,
                )
                .await
                .unwrap();

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
            .unwrap();

        self.enqueue_times.lock().insert(task_id, enqueue_time);
    }

    /// Run workers that process tasks and advance workflows
    async fn run(&self, metrics: &BenchmarkMetrics, simulate_execution: bool) -> (u64, Duration) {
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

            handles.push(tokio::spawn(async move {
                let worker_name = format!("worker-{}", worker_id);

                loop {
                    // Check if all workflows are done
                    if completed_workflows.load(Ordering::Relaxed) >= workflow_count as u64 {
                        break;
                    }

                    // Claim a task
                    let claimed = store
                        .claim_task(&worker_name, &[activity_type.clone()], 1)
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

                        // Complete task
                        store
                            .complete_task(task.id, serde_json::json!({"ok": true}))
                            .await
                            .unwrap();

                        // Record end-to-end
                        if let Some(enqueue_time) = enqueue_times.lock().get(&task.id).copied() {
                            end_to_end.record(Instant::now().duration_since(enqueue_time));
                        }

                        tasks_completed_counter.increment();
                        total_tasks_completed.fetch_add(1, Ordering::Relaxed);

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
}

/// Run a workflow throughput test
async fn run_workflow_test(
    name: &str,
    workflow_count: usize,
    steps_per_workflow: u64,
    worker_count: usize,
    simulate_execution: bool,
) -> Arc<BenchmarkMetrics> {
    let metrics = Arc::new(BenchmarkMetrics::new(name));

    println!("\n🚀 Running: {}", name);
    println!(
        "   Workflows: {}, Steps/workflow: {}, Workers: {}",
        workflow_count, steps_per_workflow, worker_count
    );
    println!(
        "   Total tasks: {}",
        workflow_count as u64 * steps_per_workflow
    );

    let mut scenario = WorkflowScenario::new(workflow_count, steps_per_workflow, worker_count);

    // Setup
    scenario.setup().await;

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
    let (total_tasks, elapsed) = scenario.run(&metrics, simulate_execution).await;

    sampling_handle.abort();
    metrics.sample();

    // Summary
    let e2e = metrics.end_to_end.summary();
    let s2s = metrics.schedule_to_start.summary();

    println!(
        "✅ Completed {} workflows in {:.2}s",
        workflow_count,
        elapsed.as_secs_f64()
    );
    println!(
        "   Task throughput: {:.1} tasks/sec",
        total_tasks as f64 / elapsed.as_secs_f64()
    );
    println!(
        "   Workflow throughput: {:.1} workflows/sec",
        workflow_count as f64 / elapsed.as_secs_f64()
    );
    println!(
        "   Schedule-to-Start: P50={:.2}ms P99={:.2}ms",
        s2s.p50.as_secs_f64() * 1000.0,
        s2s.p99.as_secs_f64() * 1000.0
    );
    println!(
        "   End-to-End (per task): P50={:.2}ms P99={:.2}ms",
        e2e.p50.as_secs_f64() * 1000.0,
        e2e.p99.as_secs_f64() * 1000.0
    );

    metrics
}

fn main() {
    let rt = Runtime::new().unwrap();

    println!("═══════════════════════════════════════════════════════════");
    println!("         Workflow Throughput Benchmark");
    println!("═══════════════════════════════════════════════════════════");
    println!("\nThis benchmark simulates the target scenario:");
    println!("  - Thousands of parallel workflows");
    println!("  - Each workflow has many sequential steps (activities)");
    println!("  - Workers claim and execute tasks, advancing workflows");

    // Scenario 1: Small scale (baseline)
    let small = rt.block_on(run_workflow_test(
        "small_10wf_10steps",
        10,    // workflows
        10,    // steps per workflow (100 total tasks)
        10,    // workers
        false, // no execution simulation
    ));

    // Scenario 2: Medium scale
    let medium = rt.block_on(run_workflow_test(
        "medium_100wf_50steps",
        100, // workflows
        50,  // steps per workflow (5,000 total tasks)
        50,  // workers
        false,
    ));

    // Scenario 3: Target scale (1000 workflows x 100 steps)
    let target = rt.block_on(run_workflow_test(
        "target_1000wf_100steps",
        1000, // workflows
        100,  // steps per workflow (100,000 total tasks)
        100,  // workers
        false,
    ));

    // Scenario 4: Target scale with execution simulation
    let target_exec = rt.block_on(run_workflow_test(
        "target_1000wf_100steps_exec",
        1000, // workflows
        100,  // steps per workflow
        100,  // workers
        true, // simulate execution (1-10ms per task)
    ));

    // Scenario 5: High parallelism (more workflows, fewer steps)
    let high_parallel = rt.block_on(run_workflow_test(
        "parallel_5000wf_20steps",
        5000, // workflows
        20,   // steps per workflow (100,000 total tasks)
        200,  // workers
        false,
    ));

    // Scenario 6: Deep workflows (fewer workflows, many steps)
    let deep = rt.block_on(run_workflow_test(
        "deep_100wf_500steps",
        100, // workflows
        500, // steps per workflow (50,000 total tasks)
        50,  // workers
        false,
    ));

    println!("\n═══════════════════════════════════════════════════════════");
    println!("                    Summary");
    println!("═══════════════════════════════════════════════════════════");

    println!(
        "\n{:<30} {:>12} {:>12} {:>12} {:>12}",
        "Scenario", "Tasks/sec", "WF/sec", "P50 S2S", "P99 S2S"
    );
    println!(
        "{:-<30} {:->12} {:->12} {:->12} {:->12}",
        "", "", "", "", ""
    );

    for (name, m, wf_count) in [
        ("small_10wf_10steps", &small, 10),
        ("medium_100wf_50steps", &medium, 100),
        ("target_1000wf_100steps", &target, 1000),
        ("target_1000wf_100steps_exec", &target_exec, 1000),
        ("parallel_5000wf_20steps", &high_parallel, 5000),
        ("deep_100wf_500steps", &deep, 100),
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
        output_dir: "target/benchmark-reports".to_string(),
        title: "Workflow Throughput Benchmark".to_string(),
        include_raw_data: false,
    };

    for (name, m) in [
        ("target_1000wf_100steps", &target),
        ("target_1000wf_100steps_exec", &target_exec),
        ("parallel_5000wf_20steps", &high_parallel),
    ] {
        let report = BenchmarkReport::new(report_config.clone());
        match report.generate(m) {
            Ok(path) => println!("   ✅ {}: {}", name, path),
            Err(e) => println!("   ❌ {}: {}", name, e),
        }
    }

    println!("\n═══════════════════════════════════════════════════════════");
}

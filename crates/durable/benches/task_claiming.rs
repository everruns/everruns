//! Task claiming benchmark
//!
//! Benchmarks the critical path: task enqueue → claim → complete
//! This is the core scheduling performance metric.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use tokio::runtime::Runtime;

use everruns_durable::persistence::{
    InMemoryWorkflowEventStore, TaskDefinition, WorkflowEventStore,
};
use everruns_durable::workflow::ActivityOptions;
use uuid::Uuid;

/// Benchmark single-threaded task claiming (baseline)
fn bench_claim_single(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    let mut group = c.benchmark_group("task_claiming/single");
    group.throughput(Throughput::Elements(1));

    for batch_size in [1, 5, 10] {
        group.bench_with_input(
            BenchmarkId::new("batch", batch_size),
            &batch_size,
            |b, &batch_size| {
                b.to_async(&rt).iter_custom(|iters| async move {
                    let store = Arc::new(InMemoryWorkflowEventStore::new());
                    let workflow_id = Uuid::now_v7();

                    // Create workflow
                    store
                        .create_workflow(workflow_id, "test", serde_json::json!({}), None)
                        .await
                        .unwrap();

                    // Pre-enqueue tasks
                    let task_count = (iters * batch_size as u64).max(100);
                    for i in 0..task_count {
                        store
                            .enqueue_task(TaskDefinition {
                                workflow_id,
                                activity_id: format!("task-{}", i),
                                activity_type: "test_activity".to_string(),
                                input: serde_json::json!({}),
                                options: ActivityOptions::default(),
                            })
                            .await
                            .unwrap();
                    }

                    // Measure claim time
                    let start = Instant::now();
                    let mut claimed_total = 0u64;

                    while claimed_total < task_count {
                        let claimed = store
                            .claim_task("worker-1", &["test_activity".to_string()], batch_size)
                            .await
                            .unwrap();

                        claimed_total += claimed.len() as u64;

                        // Complete tasks so they don't block
                        for task in claimed {
                            store
                                .complete_task(task.id, serde_json::json!({"ok": true}))
                                .await
                                .unwrap();
                        }
                    }

                    start.elapsed()
                });
            },
        );
    }

    group.finish();
}

/// Benchmark concurrent task claiming (contention)
fn bench_claim_concurrent(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    let mut group = c.benchmark_group("task_claiming/concurrent");
    // Increase sample size for more stable measurements
    group.sample_size(20);

    for workers in [2, 4, 8] {
        let task_count = 5000u64; // More tasks for stable measurement
        group.throughput(Throughput::Elements(task_count));
        group.bench_with_input(
            BenchmarkId::new("workers", workers),
            &workers,
            |b, &workers| {
                b.to_async(&rt).iter(|| async {
                    let store = Arc::new(InMemoryWorkflowEventStore::new());
                    let workflow_id = Uuid::now_v7();

                    // Create workflow
                    store
                        .create_workflow(workflow_id, "test", serde_json::json!({}), None)
                        .await
                        .unwrap();

                    // Pre-enqueue tasks
                    for i in 0..task_count {
                        store
                            .enqueue_task(TaskDefinition {
                                workflow_id,
                                activity_id: format!("task-{}", i),
                                activity_type: "test_activity".to_string(),
                                input: serde_json::json!({}),
                                options: ActivityOptions::default(),
                            })
                            .await
                            .unwrap();
                    }

                    let claimed_total = Arc::new(AtomicU64::new(0));

                    // Run workers concurrently
                    let mut handles = Vec::new();
                    for worker_id in 0..workers {
                        let store = store.clone();
                        let claimed_total = claimed_total.clone();

                        handles.push(tokio::spawn(async move {
                            let worker_name = format!("worker-{}", worker_id);
                            loop {
                                let current = claimed_total.load(Ordering::Relaxed);
                                if current >= task_count {
                                    break;
                                }

                                let claimed = store
                                    .claim_task(&worker_name, &["test_activity".to_string()], 1)
                                    .await
                                    .unwrap();

                                if claimed.is_empty() {
                                    tokio::task::yield_now().await;
                                    continue;
                                }

                                for task in claimed {
                                    store
                                        .complete_task(task.id, serde_json::json!({}))
                                        .await
                                        .unwrap();
                                    claimed_total.fetch_add(1, Ordering::Relaxed);
                                }
                            }
                        }));
                    }

                    for handle in handles {
                        handle.await.unwrap();
                    }
                });
            },
        );
    }

    group.finish();
}

/// Benchmark enqueue latency
fn bench_enqueue(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    let mut group = c.benchmark_group("task_claiming/enqueue");
    group.throughput(Throughput::Elements(1));

    group.bench_function("single", |b| {
        b.to_async(&rt).iter_custom(|iters| async move {
            let store = Arc::new(InMemoryWorkflowEventStore::new());
            let workflow_id = Uuid::now_v7();

            store
                .create_workflow(workflow_id, "test", serde_json::json!({}), None)
                .await
                .unwrap();

            let start = Instant::now();
            for i in 0..iters {
                store
                    .enqueue_task(TaskDefinition {
                        workflow_id,
                        activity_id: format!("task-{}", i),
                        activity_type: "test_activity".to_string(),
                        input: serde_json::json!({}),
                        options: ActivityOptions::default(),
                    })
                    .await
                    .unwrap();
            }
            start.elapsed()
        });
    });

    group.finish();
}

/// Benchmark schedule-to-start latency
fn bench_schedule_to_start(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    let mut group = c.benchmark_group("task_claiming/schedule_to_start");
    group.throughput(Throughput::Elements(100));

    for workers in [1, 4, 8, 16] {
        group.bench_with_input(
            BenchmarkId::new("workers", workers),
            &workers,
            |b, &workers| {
                b.to_async(&rt).iter_custom(|_iters| async move {
                    let store = Arc::new(InMemoryWorkflowEventStore::new());
                    let workflow_id = Uuid::now_v7();
                    let task_count = 100u64;

                    store
                        .create_workflow(workflow_id, "test", serde_json::json!({}), None)
                        .await
                        .unwrap();

                    // Track enqueue times
                    let enqueue_times: Arc<parking_lot::Mutex<Vec<(Uuid, Instant)>>> =
                        Arc::new(parking_lot::Mutex::new(Vec::new()));

                    // Enqueue tasks and record times
                    for i in 0..task_count {
                        let enqueue_time = Instant::now();
                        let task_id = store
                            .enqueue_task(TaskDefinition {
                                workflow_id,
                                activity_id: format!("task-{}", i),
                                activity_type: "test_activity".to_string(),
                                input: serde_json::json!({}),
                                options: ActivityOptions::default(),
                            })
                            .await
                            .unwrap();
                        enqueue_times.lock().push((task_id, enqueue_time));
                    }

                    let total_latency = Arc::new(AtomicU64::new(0));
                    let claimed_count = Arc::new(AtomicU64::new(0));

                    // Workers claim and measure schedule-to-start
                    let mut handles = Vec::new();
                    for worker_id in 0..workers {
                        let store = store.clone();
                        let enqueue_times = enqueue_times.clone();
                        let total_latency = total_latency.clone();
                        let claimed_count = claimed_count.clone();

                        handles.push(tokio::spawn(async move {
                            let worker_name = format!("worker-{}", worker_id);
                            loop {
                                let claimed = store
                                    .claim_task(&worker_name, &["test_activity".to_string()], 1)
                                    .await
                                    .unwrap();

                                if claimed.is_empty() {
                                    if claimed_count.load(Ordering::Relaxed) >= task_count {
                                        break;
                                    }
                                    tokio::task::yield_now().await;
                                    continue;
                                }

                                let claim_time = Instant::now();

                                for task in &claimed {
                                    let times = enqueue_times.lock();
                                    if let Some((_, enqueue_time)) =
                                        times.iter().find(|(id, _)| *id == task.id)
                                    {
                                        let latency = claim_time.duration_since(*enqueue_time);
                                        total_latency.fetch_add(
                                            latency.as_micros() as u64,
                                            Ordering::Relaxed,
                                        );
                                    }
                                }

                                for task in claimed {
                                    store
                                        .complete_task(task.id, serde_json::json!({}))
                                        .await
                                        .unwrap();
                                    claimed_count.fetch_add(1, Ordering::Relaxed);
                                }
                            }
                        }));
                    }

                    for handle in handles {
                        handle.await.unwrap();
                    }

                    // Return average latency as the measured time
                    let avg_latency_micros =
                        total_latency.load(Ordering::Relaxed) / task_count.max(1);
                    Duration::from_micros(avg_latency_micros)
                });
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_claim_single,
    bench_claim_concurrent,
    bench_enqueue,
    bench_schedule_to_start,
);

criterion_main!(benches);

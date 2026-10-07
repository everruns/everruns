# everruns-durable

> PostgreSQL-backed durable execution engine for Everruns: event-sourced workflows, a claimable task queue, retries, circuit breakers and schedules.

[![Crates.io](https://img.shields.io/crates/v/everruns-durable.svg)](https://crates.io/crates/everruns-durable)
[![Documentation](https://docs.rs/everruns-durable/badge.svg)](https://docs.rs/everruns-durable)
[![License](https://img.shields.io/crates/l/everruns-durable.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

`everruns-durable` keeps work alive across crashes and restarts. State lives in
PostgreSQL, workers claim tasks with `SELECT ... FOR UPDATE SKIP LOCKED`, and
anything a dead worker held is reclaimed and retried. It needs no
infrastructure beyond PostgreSQL.

It is a focused crate in the [Everruns](https://everruns.com) ecosystem. The
Everruns control plane and workers use it to keep long-running agent turns
progressing, and it works on its own for any workflow or job queue that should
survive restarts.

```sh
cargo add everruns-durable
```

## What It Provides

- Event-sourced, deterministic workflows (`Workflow`, `WorkflowExecutor`) with
  replay, snapshots, timers, child workflows, signals and `continue_as_new`
  (experimental, `workflows` feature, on by default)
- A PostgreSQL task queue with priorities, `SKIP LOCKED` claiming, heartbeats,
  stale-claim reclamation and a dead letter queue
- Retry policies, timeouts and distributed circuit breakers
- A worker pool with bounded concurrency and backpressure (`workflows`)
- Cron and interval schedules with leader-safe claiming
- A self-contained PostgreSQL schema (`PostgresWorkflowEventStore::migrate`)
  and an in-memory store for tests

The crate is a generic engine: it knows workflows, activities, tasks, signals
and schedules, and depends on no other Everruns crate. It has no notion of
agents, sessions or turns.

## How Everruns uses it

Agent turns run on the task queue directly. Each turn step (`process_input`,
`reason`, `act`) is a task; the turn driver in
[`everruns-durable-engine`](https://crates.io/crates/everruns-durable-engine)
runs a claimed step, checkpoints the turn's state, and enqueues the next task.
The Everruns worker and the `everruns` framework's experimental `durable`
feature both run turns that way. This crate owns persistence, retries and
scheduling; turn semantics, signal payloads and what a sealed task means to a
session live in `everruns-durable-engine` and the Everruns server.

The general-purpose workflow engine (`WorkflowExecutor` over the
`Workflow` trait) is the other half of the crate:
deterministic state machines replayed from an event log. It is experimental
and sits behind the default `workflows` feature. Everruns itself builds the
crate with `default-features = false`, so its services compile only the store,
queue, reliability, scheduler and worker pool core. The example below uses the engine, and
[`examples/order_pipeline.rs`](examples/order_pipeline.rs) is a runnable order
pipeline (three activities, one retried) on the in-memory store:

```sh
cargo run -p everruns-durable --example order_pipeline
```

## Quick start

A two-step workflow, run end to end against the in-memory store. The same code
runs against PostgreSQL by swapping in `PostgresWorkflowEventStore::new(pool)`.

```rust
use everruns_durable::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Serialize, Deserialize)]
struct Input {
    name: String,
}

/// Fetch a greeting, then shout it.
struct Greet {
    name: String,
    output: Option<String>,
}

impl Workflow for Greet {
    const TYPE: &'static str = "greet";
    type Input = Input;
    type Output = String;

    fn new(input: Input) -> Self {
        Self { name: input.name, output: None }
    }

    fn on_start(&mut self) -> Vec<WorkflowAction> {
        vec![WorkflowAction::schedule_activity("fetch", "fetch_greeting", json!({ "name": self.name }))]
    }

    fn on_activity_completed(&mut self, activity_id: &str, result: Value) -> Vec<WorkflowAction> {
        match activity_id {
            "fetch" => vec![WorkflowAction::schedule_activity("shout", "shout", result)],
            _ => {
                let text = result.as_str().unwrap_or_default().to_string();
                self.output = Some(text.clone());
                vec![WorkflowAction::complete(json!(text))]
            }
        }
    }

    fn on_activity_failed(&mut self, _: &str, error: &ActivityError) -> Vec<WorkflowAction> {
        vec![WorkflowAction::fail(WorkflowError::new(&error.message))]
    }

    fn is_completed(&self) -> bool {
        self.output.is_some()
    }

    fn result(&self) -> Option<String> {
        self.output.clone()
    }
}

/// What a worker does for each claimed task.
fn run_activity(activity_type: &str, input: &Value) -> Value {
    match activity_type {
        "fetch_greeting" => json!(format!("hello, {}", input["name"].as_str().unwrap())),
        "shout" => json!(input.as_str().unwrap().to_uppercase()),
        other => unreachable!("no handler for {other}"),
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut executor = WorkflowExecutor::new(InMemoryWorkflowEventStore::new());
    executor.register::<Greet>();

    let id = executor
        .start_workflow::<Greet>(Input { name: "durable".into() }, None)
        .await?;

    // A minimal worker: register, then claim, execute, report back.
    let types = ["fetch_greeting".to_string(), "shout".to_string()];
    executor.store().register_worker(WorkerInfo::new("worker-1", types.clone())).await?;
    loop {
        let tasks = executor.store().claim_task("worker-1", &types, 10).await?;
        if tasks.is_empty() {
            break;
        }
        for task in tasks {
            let output = run_activity(&task.activity_type, &task.input);
            executor.store().complete_task(task.id, "worker-1", output.clone()).await?;
            executor
                .on_activity_completed(task.workflow_id.unwrap(), &task.activity_id, output)
                .await?;
        }
    }

    let info = executor.store().get_workflow_info(id).await?;
    assert_eq!(info.status, WorkflowStatus::Completed);
    assert_eq!(info.result, Some(json!("HELLO, DURABLE")));
    Ok(())
}
```

## Concepts

| Piece | Role |
| --- | --- |
| `Workflow` | Deterministic state machine. Handlers (`on_start`, `on_activity_completed`, `on_activity_failed`, `on_timer_fired`, `on_child_workflow_completed`, `on_child_workflow_failed`, `on_signal`) return `WorkflowAction`s. |
| `WorkflowEvent` | Append-only history. Replaying it rebuilds workflow state after a crash. |
| `WorkflowExecutor` | Starts workflows, appends events, replays history and applies the actions it has not recorded yet, so re-processing is idempotent. Optional snapshots bound replay cost; `continue_as_new` rolls over long histories. |
| `WorkflowEventStore` | Umbrella storage contract, blanket-implemented over focused traits: `EventLog`, `TaskQueue`, `SignalStore`, `WorkerRegistry`, `DeadLetters`, `CircuitBreakers`, `Schedules`, `DurableAdmin`. A worker needs only `TaskQueue + SignalStore + WorkerRegistry`. No method silently succeeds by default, so a new store must implement each one. `PostgresWorkflowEventStore` for production, `InMemoryWorkflowEventStore` for tests and benches. |
| `TaskDefinition` / `ClaimedTask` | A queued activity. `workflow_id: None` makes it a standalone queue task. |
| `WorkerPool` | Polls for tasks, runs registered handlers with bounded concurrency, heartbeats, reclaims stale work and applies backpressure. |
| `DurableScheduler` | Cron and interval schedules that start workflows or tasks, with leader-safe claiming. |

### Timers and child workflows

`WorkflowAction::timer` fires `on_timer_fired` once its duration has passed.
`WorkflowAction::child_workflow` starts a registered workflow type and reports
its outcome to `on_child_workflow_completed` or `on_child_workflow_failed`.

Both run as the engine's own tasks on the ordinary queue, so they survive a
crash and retry like any activity. Something has to claim them: register a
worker for `SYSTEM_ACTIVITY_TYPES` and call `run_system_tasks` in a loop.

```rust,ignore
let engine = WorkerInfo::new("engine-1", SYSTEM_ACTIVITY_TYPES);
executor.store().register_worker(engine).await?;
loop {
    if executor.run_system_tasks("engine-1", 100).await? == 0 {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
```

A child's id is derived from its parent and the id the parent gave it, so a
retried start never creates a second child. The rustdoc of
`WorkflowExecutor::run_system_tasks` has a runnable example.

## Reliability

Every activity carries `ActivityOptions`: a retry policy, timeouts and a
priority. Retries back off exponentially with jitter, and errors whose type is
listed as non-retryable fail at once.

```rust
use std::time::Duration;
use everruns_durable::{ActivityOptions, RetryPolicy};

let options = ActivityOptions::default()
    .with_retry(
        RetryPolicy::exponential()
            .with_max_attempts(5)
            .with_initial_interval(Duration::from_millis(200))
            .with_max_interval(Duration::from_secs(10))
            .with_non_retryable_error("validation"),
    )
    .with_start_to_close_timeout(Duration::from_secs(60))
    .with_heartbeat(Duration::from_secs(15));

assert!(options.retry_policy.should_retry(Some("rate_limited")));
assert!(!options.retry_policy.should_retry(Some("validation")));
```

The rest of the toolkit:

- **Stale reclamation.** A task whose worker stops heartbeating is returned to
  the queue. Completing a reclaimed task from the old worker fails with
  `StoreError::TaskNotOwned`, so a slow worker cannot schedule work twice.
- **Forward-progress guard.** A task reclaimed repeatedly without the workflow
  advancing is sealed into the dead letter queue instead of looping
  (`DURABLE_NO_PROGRESS_SEAL_THRESHOLD`, default 3).
- **Dead letter queue.** Exhausted and sealed tasks keep their input and error
  for inspection and `requeue_from_dlq`.
- **Circuit breakers.** `DistributedCircuitBreaker` shares open, half-open
  and closed state through the store, so every worker trips together.
- **Backpressure.** Workers stop claiming above a load watermark and advertise
  `accepting_tasks` in their heartbeats.

## Running against PostgreSQL

The crate ships its own schema: the `durable_*` tables, their indexes, and the
trigger functions behind worker wake-ups and health counters. Apply it with
`PostgresWorkflowEventStore::migrate` before building the store, or connect
and migrate in one call with `PostgresWorkflowEventStore::connect(url)`. It is
idempotent, runs in one transaction, serializes concurrent callers on an
advisory lock, and only reads a database that already took this exact schema,
so calling it on every start-up is safe. It needs PostgreSQL 14
or newer and no extensions. Objects are created in the first schema of the
connection's `search_path`.

```rust,no_run
use everruns_durable::prelude::*;

# async fn run() -> Result<(), Box<dyn std::error::Error>> {
let pool = sqlx::PgPool::connect("postgres://localhost/my_app").await?;
PostgresWorkflowEventStore::migrate(&pool).await?;

let mut executor = WorkflowExecutor::new(PostgresWorkflowEventStore::new(pool));
# let _ = &mut executor;
# Ok(()) }
```

If your application manages schema with its own migration tool, copy
`PostgresWorkflowEventStore::SCHEMA_SQL` into a migration instead. A database
whose durable tables were created by the Everruns server migrations already
matches this schema, and `migrate` leaves it unchanged; a CI drift test keeps
the two in step.

## Testing

```sh
# Unit tests, in-memory store (no database)
cargo test -p everruns-durable

# The store, queue and scheduler core without the workflow engine
cargo test -p everruns-durable --no-default-features --lib

# The isolated workflow example
cargo test -p everruns-durable --example order_pipeline

# PostgreSQL integration tests (DATABASE_URL, default port 9332, migrated
# with the server migrations, which schema_drift_test compares against)
cargo test -p everruns-durable --features postgres-tests \
  --test postgres_integration_test --test postgres_repository_test \
  --test heartbeat_cancel_test --test store_conformance_test \
  --test schema_drift_test -- --test-threads=1

# Failure injection: fail-rs failpoints inside the PostgreSQL store
cargo test -p everruns-durable --features "failpoints,postgres-tests" \
  --test failure_injection_test --test agent_reliability_test -- --test-threads=1
```

Coverage across all of the above (needs `cargo-llvm-cov`):

```sh
cargo llvm-cov -p everruns-durable --features "failpoints,postgres-tests" \
  --no-fail-fast -- --test-threads=1
```

`store_conformance_test` runs one set of cases against both stores, so the
in-memory store stays a faithful stand-in for PostgreSQL: registered workers
only, priority then FIFO claim order, retry backoff, stale-claim reclaim.
`agent_reliability_test` drives whole workflows through worker crashes,
control-plane restarts and database outages. All of these run in CI on the
`durable` PostgreSQL shard, along with `examples/order_pipeline.rs`. The
examples in this README are compiled and run as doctests by
`cargo test -p everruns-durable`.

## Benchmarks

```sh
just durable bench                 # in-memory store
just durable bench-db              # PostgreSQL store (DATABASE_URL)
just durable bench --save my-box   # also write a checkpoint for comparison
```

Scenarios cover worker scaling (1 to 100 workers, burst load), workflow
throughput (many workflows with many sequential steps) and cold-start latency.
Each run writes HTML reports under `target/benchmark-reports/`.

The bench binaries and the `bench` support module need the `bench` feature,
which the commands above enable. Every bench takes the same flags: `--smoke` runs each scenario at a tiny scale,
and `--summary <file>` appends one JSON line per scenario.

- **Pull requests** run every bench with `--smoke` on the `durable` CI shard, so
  a broken bench fails the change that broke it.
- **Weekly**, the `Durable Benchmarks` workflow runs them at full scale and
  compares throughput with `benches/baseline.jsonl`, failing on a drop of more
  than 30%. Refresh the baseline from a trusted run's `summary.jsonl`
  artifacts when a change is expected.
- **Checkpoints** in `benches/checkpoints/` keep compact history (1000-point
  quantile sketches, not raw samples).

## Feature flags

| Flag | Effect |
| --- | --- |
| `workflows` | Default. The experimental workflow engine: `Workflow`, `Activity`, `WorkflowExecutor` (timers, child workflows, system tasks), `TimeoutManager`. |
| `postgres-tests` | Compiles the tests that need a live PostgreSQL. |
| `failpoints` | Enables `fail-rs` failpoints in the PostgreSQL store. Zero cost when off. |
| `bench` | Builds the benchmark support module and bench binaries; implies `workflows`. Not a supported API. |

## Documentation

- [Durable execution](https://docs.everruns.com/explanation/durable-execution/)
- [API reference](https://docs.rs/everruns-durable)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).


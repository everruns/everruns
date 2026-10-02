# everruns-durable

> PostgreSQL-backed durable execution engine for Everruns: event-sourced
> workflows, a claimable task queue, retries, circuit breakers and schedules.

`everruns-durable` keeps work alive across crashes and restarts. State lives in
PostgreSQL, workers claim tasks with `SELECT ... FOR UPDATE SKIP LOCKED`, and
anything a dead worker held is reclaimed and retried. It needs no
infrastructure beyond PostgreSQL.

It is an internal crate of the [Everruns](https://everruns.com) workspace
(`publish = false`). The control plane and workers use it to keep long-running
agent turns progressing.

## How Everruns uses it

Agent turns run on the task queue directly. The worker claims `reason` and
`act` tasks, advances the turn through `DurableExecution`, the checkpointed
driver for the shared `everruns-engine::Execution` contract, and enqueues the
next task. This crate owns persistence, retries and scheduling; turn semantics
stay in `everruns-engine`.

The general-purpose workflow engine (`WorkflowExecutor` over the
`Workflow` trait) is the other half of the crate: deterministic state
machines replayed from an event log. The example below uses it.

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
| `Workflow` | Deterministic state machine. Handlers (`on_start`, `on_activity_completed`, `on_activity_failed`, `on_timer_fired`, `on_signal`) return `WorkflowAction`s. |
| `WorkflowEvent` | Append-only history. Replaying it rebuilds workflow state after a crash. |
| `WorkflowExecutor` | Starts workflows, appends events, replays history and applies the actions it has not recorded yet, so re-processing is idempotent. Optional snapshots bound replay cost; `continue_as_new` rolls over long histories. |
| `WorkflowEventStore` | Storage contract: event log, task queue, workers, DLQ, circuit breakers, schedules. `PostgresWorkflowEventStore` for production, `InMemoryWorkflowEventStore` for tests and benches. |
| `TaskDefinition` / `ClaimedTask` | A queued activity. `workflow_id: None` makes it a standalone queue task. |
| `WorkerPool` | Polls for tasks, runs registered handlers with bounded concurrency, heartbeats, reclaims stale work and applies backpressure. |
| `DurableScheduler` | Cron and interval schedules that start workflows or tasks, with leader-safe claiming. |
| `DurableExecution` | Checkpointed driver of `everruns-engine` turns. |

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

The schema lives in the server's migrations (`durable_*` tables):

```sh
sqlx migrate run --source crates/server/migrations
```

Then build the store from a pool:

```rust,no_run
use everruns_durable::PostgresWorkflowEventStore;

# async fn connect() -> Result<(), sqlx::Error> {
let pool = sqlx::PgPool::connect("postgres://localhost/everruns").await?;
let store = PostgresWorkflowEventStore::new(pool);
# Ok(()) }
```

## Testing

```sh
# Unit tests, in-memory store (no database)
cargo test -p everruns-durable

# PostgreSQL integration tests (DATABASE_URL, default port 9332, migrated)
cargo test -p everruns-durable --features postgres-tests \
  --test postgres_integration_test --test postgres_repository_test \
  --test heartbeat_cancel_test --test store_conformance_test -- --test-threads=1

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
`durable` PostgreSQL shard. The examples in this README are compiled and run
as doctests.

## Benchmarks

```sh
just durable bench                 # in-memory store
just durable bench-db              # PostgreSQL store (DATABASE_URL)
just durable bench --save my-box   # also write a checkpoint for comparison
```

Scenarios cover worker scaling (1 to 100 workers, burst load), workflow
throughput (many workflows with many sequential steps) and cold-start latency.
Each run writes HTML reports to `crates/durable/target/benchmark-reports/`.

Every bench takes the same flags: `--smoke` runs each scenario at a tiny scale,
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
| `postgres-tests` | Compiles the tests that need a live PostgreSQL. |
| `failpoints` | Enables `fail-rs` failpoints in the PostgreSQL store. Zero cost when off. |

## Documentation

- [Durable execution](https://docs.everruns.com/explanation/durable-execution/)
- Design of record: `knowledge/operations/durable-execution-engine.md`

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).


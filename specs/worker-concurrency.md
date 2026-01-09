# Worker Concurrency Specification

## Abstract

This specification defines the redesign of worker task execution to support high-concurrency IO-intensive workloads. The current implementation processes tasks sequentially within a worker, limiting throughput. The new design enables thousands of concurrent tasks per worker while preserving workflow ordering guarantees and implementing resource-based backpressure.

**Key invariant**: Tasks within a single workflow execute sequentially. Tasks from different workflows execute in parallel.

## Current State

```
Worker polls → Claims 10 tasks → Executes sequentially → Polls again
```

- Single worker processes ~1 task at a time
- `max_concurrent_tasks` config is misleading (batch claim size, not concurrency)
- No backpressure mechanism
- 1-second poll interval adds latency when workers are busy

## Requirements

### Phase 1: Concurrent Task Execution

#### 1.1 Parallel Execution Across Workflows

Tasks from different workflows MUST execute concurrently within a single worker.

```
Workflow A: [task A1] → [task A2] → [task A3]  (sequential)
Workflow B: [task B1] → [task B2]              (sequential)

Execution timeline:
t0: A1 starts, B1 starts     (parallel - different workflows)
t1: A1 completes → A2 starts (sequential - same workflow)
t2: B1 completes → B2 starts (sequential - same workflow)
t3: A2 completes → A3 starts
```

#### 1.2 Workflow-Level Sequential Guarantee

Tasks within the same workflow MUST execute sequentially. A worker MUST NOT start task N+1 of a workflow until task N completes.

Implementation: Track in-flight workflows. When claiming tasks, skip tasks whose workflow already has an in-flight task on this worker.

#### 1.3 Semaphore-Based Concurrency Limit

Each worker MUST limit total in-flight tasks using a semaphore.

```rust
struct DurableWorker {
    /// Controls max concurrent tasks across all workflows
    in_flight_semaphore: Arc<Semaphore>,  // e.g., 1000 permits

    /// Tracks which workflows have in-flight tasks (for sequential guarantee)
    in_flight_workflows: Arc<DashSet<Uuid>>,
}
```

Configuration:
- `MAX_CONCURRENT_TASKS` env var: Maximum in-flight tasks (default: 1000)
- Semaphore blocks new task spawns when limit reached

#### 1.4 Task Spawning Model

Replace sequential `for` loop with `tokio::spawn`:

```rust
async fn poll_and_execute(&self) -> Result<usize> {
    let available = self.in_flight_semaphore.available_permits();
    if available == 0 {
        return Ok(0);  // Backpressure: at capacity
    }

    let tasks = store.claim_tasks(
        &self.config.worker_id,
        &self.config.activity_types,
        min(available, self.config.claim_batch_size),
    ).await?;

    for task in tasks {
        // Skip if workflow already has in-flight task (preserve ordering)
        if self.in_flight_workflows.contains(&task.workflow_id) {
            // Return task to queue (or just don't start it)
            store.release_task(task.id).await?;
            continue;
        }

        let permit = self.in_flight_semaphore.clone().acquire_owned().await?;
        self.in_flight_workflows.insert(task.workflow_id);

        let worker = self.clone();
        tokio::spawn(async move {
            let result = worker.execute_task(&task).await;
            worker.in_flight_workflows.remove(&task.workflow_id);
            drop(permit);

            // Report completion/failure
            match result {
                Ok(output) => store.complete_task(task.id, output).await,
                Err(e) => store.fail_task(task.id, &e.to_string()).await,
            }
        });
    }

    Ok(tasks.len())
}
```

#### 1.5 Configuration Changes

| Config | Old Default | New Default | Description |
|--------|-------------|-------------|-------------|
| `MAX_CONCURRENT_TASKS` | 10 (misleading) | 1000 | True concurrency limit |
| `CLAIM_BATCH_SIZE` | N/A | 50 | Tasks to claim per poll |
| `POLL_INTERVAL_MS` | 1000 | 100 | Poll interval when idle |

### Phase 2: Even Distribution and Backpressure

#### 2.1 Capacity-Aware Claiming

Workers MUST only claim tasks they can process. Claim size adapts to available capacity.

```rust
fn calculate_claim_size(&self) -> usize {
    let available_permits = self.in_flight_semaphore.available_permits();

    // Don't claim more than we can handle
    min(available_permits, self.config.claim_batch_size)
}
```

#### 2.2 Smaller Claims, Faster Polls

To improve distribution across workers:

- Reduce `CLAIM_BATCH_SIZE` to 10-50 (from claiming all available)
- Reduce `POLL_INTERVAL_MS` to 50-100ms (from 1000ms)

This creates more frequent, smaller claims → better work distribution.

```
Old: Worker claims 100 tasks, other workers starve
New: Worker claims 10 tasks, polls again in 50ms, others get fair share
```

#### 2.3 Resource-Based Backpressure

Workers MUST reduce claim size under resource pressure.

Backpressure signals:
1. **Semaphore saturation**: `available_permits / max_permits < 0.1` → stop claiming
2. **Memory pressure**: `memory_usage > 80%` → reduce claim size by 50%
3. **Task latency**: `p99_latency > threshold` → reduce claim size by 30%

```rust
fn calculate_claim_size(&self) -> usize {
    let base = self.config.claim_batch_size;
    let available = self.in_flight_semaphore.available_permits();

    // Factor 1: Available permits
    if available < 10 {
        return 0;  // At capacity
    }

    let mut factor = 1.0;

    // Factor 2: Memory pressure
    if let Ok(mem) = sys_info::mem_info() {
        let usage = 1.0 - (mem.avail as f64 / mem.total as f64);
        if usage > 0.8 {
            factor *= 0.5;
        }
    }

    // Factor 3: Latency pressure (from metrics)
    if self.metrics.p99_latency() > self.config.latency_threshold {
        factor *= 0.7;
    }

    min(available, (base as f64 * factor) as usize)
}
```

#### 2.4 Metrics Export

Workers MUST export metrics for observability:

- `worker_in_flight_tasks` (gauge): Current in-flight task count
- `worker_tasks_completed_total` (counter): Total completed tasks
- `worker_task_duration_seconds` (histogram): Task execution duration
- `worker_claims_total` (counter): Total claim attempts
- `worker_claim_size` (histogram): Tasks claimed per poll

#### 2.5 Heartbeat with Capacity Reporting

Extend heartbeat to report worker capacity:

```protobuf
message HeartbeatDurableTaskRequest {
    string task_id = 1;
    string worker_id = 2;

    // New: capacity reporting
    int32 available_capacity = 3;
    int32 max_capacity = 4;
    float memory_usage = 5;
}
```

Control-plane can use this for monitoring/alerting (not for routing in Phase 2).

## Dismissed Options (Phase 3 Candidates)

### Push-Based Task Distribution

**Status**: Dismissed for now (Phase 3 candidate)

**What it is**: Control-plane pushes tasks to workers via gRPC streaming instead of workers polling.

```
Control-Plane                    Workers
     │                              │
     │◀── WorkerReady(capacity) ────│
     │                              │
     │─── PushTask(task) ──────────▶│
     │                              │
     │◀── TaskComplete(result) ─────│
```

**Why considered**:
- Eliminates polling latency entirely (~0ms vs 50-100ms)
- Control-plane has global view for optimal routing
- Natural load balancing (push to workers with capacity)

**Why dismissed for now**:
- Significantly more complex (bidirectional streaming, connection management)
- Requires control-plane to track worker connections and state
- Current pull model sufficient for expected scale (1000s of tasks/sec)
- Can achieve good distribution with smaller claims + faster polls

**May revisit when**:
- Sub-10ms task latency required
- Pull-based distribution proves inadequate at scale
- Need intelligent routing (affinity, locality, priority queues)

### Work Stealing

**Status**: Dismissed for now (Phase 3 candidate)

**What it is**: Idle workers "steal" tasks from busy workers' local queues.

**Why considered**:
- Excellent load balancing without central coordinator
- Handles heterogeneous task durations well
- Proven pattern (Tokio, Go runtime, Cilk)

**Why dismissed for now**:
- Workers don't have local queues (tasks in PostgreSQL)
- Would require architectural change to local buffering
- Adds complexity for worker-to-worker communication
- Pull model with small claims approximates work stealing

**May revisit when**:
- Need sub-millisecond scheduling decisions
- Move to local task queues for performance

### Workflow Affinity

**Status**: Dismissed for now (Phase 3 candidate)

**What it is**: Route all tasks from a workflow to the same worker.

**Why considered**:
- Simplifies sequential guarantee (single worker handles entire workflow)
- Better cache locality for workflow state
- Reduces coordination overhead

**Why dismissed for now**:
- Creates hot spots (long workflows block one worker)
- Worker failure requires re-routing entire workflow
- Current per-task claiming with workflow tracking is sufficient
- Cross-workflow parallelism more valuable than affinity

**May revisit when**:
- Workflow state caching becomes a bottleneck
- Need workflow-level resource isolation

### Priority Queues per Worker

**Status**: Dismissed for now (Phase 3 candidate)

**What it is**: Workers maintain local priority queues, high-priority tasks preempt low-priority.

**Why considered**:
- Ensures important tasks get processed first
- Reduces head-of-line blocking from long tasks

**Why dismissed for now**:
- PostgreSQL query already supports `ORDER BY priority DESC`
- Preemption adds complexity (task state management)
- Can achieve priority via separate activity types if needed

**May revisit when**:
- Strict SLA requirements for certain task types
- Need preemption for latency-sensitive tasks

### LISTEN/NOTIFY for Instant Wake

**Status**: Dismissed for now (Phase 3 candidate)

**What it is**: Use PostgreSQL `LISTEN/NOTIFY` to wake workers immediately when tasks arrive.

```sql
-- On task insert
NOTIFY task_available, 'workflow_id';

-- Worker listens
LISTEN task_available;
```

**Why considered**:
- Eliminates polling entirely when queue is empty
- Instant wake-up on new tasks (~1ms)
- Native PostgreSQL feature, no additional infrastructure

**Why dismissed for now**:
- Adds connection management complexity (dedicated listener connection)
- With 50-100ms poll interval, latency acceptable for most use cases
- Notification fan-out to all workers (thundering herd potential)

**May revisit when**:
- Need sub-100ms task pickup latency
- Polling overhead becomes significant at scale

## Migration Path

### Phase 1 Implementation

1. Add `in_flight_semaphore` and `in_flight_workflows` to `DurableWorker`
2. Replace sequential `for` loop with `tokio::spawn`
3. Add workflow tracking to preserve sequential guarantee
4. Add `release_task` gRPC method for returning unclaimed tasks
5. Update configuration defaults

### Phase 2 Implementation

1. Implement `calculate_claim_size` with backpressure factors
2. Add metrics collection and export
3. Extend heartbeat protocol with capacity reporting
4. Tune poll interval and claim batch size
5. Add memory/latency monitoring

## Testing Requirements

1. **Sequential guarantee test**: Submit workflow with 10 tasks, verify execution order preserved
2. **Parallel execution test**: Submit 100 workflows, verify concurrent execution
3. **Backpressure test**: Submit 10,000 tasks, verify worker doesn't OOM
4. **Distribution test**: Run 3 workers, verify roughly even task distribution
5. **Failure recovery test**: Kill worker mid-task, verify task reclaimed and completed

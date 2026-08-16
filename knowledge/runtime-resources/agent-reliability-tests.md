---
type: Specification
title: "Agent Reliability Tests"
description: "Agent execution reliability tests."
tags:
  - everruns
  - runtime-resources
---
# Agent Reliability Tests

## Abstract

End-to-end reliability tests that verify agent execution survives infrastructure failures. Four failure domains: worker crashes, control plane restarts, worker↔CP network partitions, and CP↔DB network partitions.

Status: audited in EVE-349. This binary is not part of required CI coverage
yet because a clean PostgreSQL-backed run still fails several restart/reclaim
scenarios; keep it as an explicit manual harness until those cases are
stabilized.

## Goals

1. Verify workflow completion when workers crash mid-task (stale reclamation path)
2. Verify workflow resumption after control plane restart (event sourcing replay)
3. Verify worker recovery from gRPC/network failures (reconnect + retry)
4. Verify state consistency after database connectivity loss and recovery

## Non-Goals

1. Full process-level chaos (use infrastructure chaos tools for that)
2. Performance under failure (covered by load tests + benchmarks)
3. Multi-node partition testing (requires Docker/k8s)

## Test Scenarios

### Scenario 1: Worker Killed Mid-Task

**What happens in production:** Worker process crashes (OOM, SIGKILL, panic) while executing an activity. Heartbeat stops. After `stale_threshold` (default 60s), control plane reclaims the task and marks it pending. Another worker claims and completes it. Workflow completes normally via event replay.

**Test approach:** Use the PostgreSQL store directly with the `WorkflowExecutor`. Start a multi-step workflow, simulate worker crash by abandoning a claimed task with stale heartbeat, trigger reclamation, have a second worker complete the task, and verify the workflow completes.

**Variants:**
- Single worker crash → reclaim → same worker type completes
- Crash during first step of multi-step workflow
- Crash during middle step (verify partial progress preserved)
- Repeated crashes (task retries exhaust → DLQ)

### Scenario 2: Control Plane Restart

**What happens in production:** Server process restarts. Workers lose gRPC connections and retry. On restart, the executor replays events from PostgreSQL and resumes workflows from their last persisted state. No in-flight state is lost because all state is event-sourced.

**Test approach:** Create a `WorkflowExecutor`, start a workflow, drop the executor (simulating crash), create a new executor with the same store, and verify it can process the workflow from its persisted event log.

**Variants:**
- Restart with no in-flight activities
- Restart with pending tasks in queue (tasks survive in PostgreSQL)
- Restart mid-workflow (events replayed, next activity scheduled)

### Scenario 3: Network Between Control Plane and Worker

**What happens in production:** gRPC connection drops. Worker detects stream disconnect, falls back to polling, attempts reconnection with exponential backoff (1s→60s). Tasks in flight continue executing locally but can't report completion. If heartbeat times out, tasks are reclaimed. When network recovers, worker reconnects and late completions get `TaskNotOwned` (idempotent).

**Test approach:** Two complementary layers.

1. **Store-layer failpoints** (existing): use `fail-rs` failpoints in the persistence layer to simulate gRPC-induced store failures. Test sequences: claim succeeds → complete fails → retry complete → verify idempotent; claim fails → retry → succeeds.
2. **Transport-layer simulation** (in progress): use [`turmoil`](https://github.com/tokio-rs/turmoil) to drop, partition, and reorder packets between worker and control plane with a simulated clock. Scaffold lives at `crates/worker/tests/network_reliability_test.rs`. Currently exercises a TCP heartbeat loop; the next step is wiring the real tonic `WorkerService` through a turmoil connector so the actual `GrpcDurableStore` client retry/backoff paths are covered. This adds determinism that fail-rs cannot provide for transport faults (RST mid-stream, latency spikes, message reorder).

**Variants:**
- Transient failure (1 failure then success)
- Extended outage (multiple failures then recovery)
- Task ownership lost during outage (TaskNotOwned on late completion)

### Scenario 4: Network Between Control Plane and DB

**What happens in production:** PostgreSQL becomes unreachable. All store operations fail. Circuit breaker opens to protect against cascading failures. When DB recovers, operations resume. No data corruption because all writes are transactional.

**Test approach:** Use failpoints to inject persistent then transient DB failures. Verify operations fail cleanly, then succeed after recovery. Verify no partial writes (transaction rollback). Test circuit breaker behavior under sustained failures.

**Variants:**
- Brief DB blip (single failure, retry succeeds)
- Extended DB outage (all operations fail, then recovery)
- DB failure during event append (verify rollback, no partial writes)
- Concurrent workflow operations during DB recovery

## Fail Points

### Existing (reused)

| Name | Location | Used In |
|------|----------|---------|
| `postgres_append_events_after_insert` | postgres.rs | Scenarios 2, 4 |
| `postgres_append_events_before_commit` | postgres.rs | Scenario 4 |
| `postgres_claim_task_after_query` | postgres.rs | Scenarios 1, 3 |
| `postgres_heartbeat_update` | postgres.rs | Scenarios 1, 3 |
| `postgres_complete_task_after_update` | postgres.rs | Scenarios 1, 3 |
| `circuit_breaker_get_state_db_fetch` | distributed_circuit_breaker.rs | Scenario 4 |

### New

| Name | Location | Purpose |
|------|----------|---------|
| `postgres_enqueue_task_after_insert` | postgres.rs | Simulate failure after task enqueue |
| `postgres_load_events_after_query` | postgres.rs | Simulate failure during event replay |
| `postgres_reclaim_stale_after_update` | postgres.rs | Simulate failure during reclamation |

## Running Tests

```bash
# All reliability tests
cargo test -p everruns-durable --test agent_reliability_test --features "failpoints,postgres-tests" -- --test-threads=1

# Specific scenario
cargo test -p everruns-durable --test agent_reliability_test worker_crash --features "failpoints,postgres-tests"
```

These tests are still manual reliability harnesses. Do not claim durable
recovery coverage from CI until this binary passes consistently under a clean
PostgreSQL-backed run.

## Decisions

### Store-Level Testing

**Decision:** Test at the store + executor level, not full server process management.

**Rationale:** Store-level tests are deterministic, fast, and can exercise all failure paths without Docker/process management. The store is the source of truth, if store-level recovery works, the system recovers.

### Executor-Driven Workflows

**Decision:** Use `WorkflowExecutor` with test workflow types to test full workflow lifecycle.

**Rationale:** Existing failure tests only test individual store operations. Reliability tests need to verify that workflows *complete* despite failures, that requires driving the full executor→store→claim→complete→process cycle.

## Related Testing Specs

See also: [fail-rs-testing.md](../evaluation/fail-rs-testing.md) (failure injection), [load-testing.md](../operations/load-testing.md) (performance), [test-cases.md](../evaluation/test-cases.md) (manual tests), [evals.md](../evaluation/evals.md) (behavioral evals)

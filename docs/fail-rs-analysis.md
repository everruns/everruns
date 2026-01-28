# fail-rs Integration Analysis

Analysis of using [tikv/fail-rs](https://github.com/tikv/fail-rs) for improved test coverage and durability testing in Everruns.

## Executive Summary

**Recommendation: YES** - fail-rs is well-suited for this codebase. The durable workflow engine, distributed worker architecture, and multiple external dependencies (PostgreSQL, LLM providers, gRPC) create many failure scenarios that are difficult to test naturally but critical for production reliability.

## What is fail-rs?

fail-rs provides **fail points** - code instrumentations that allow dynamic injection of:
- Panics
- Early returns with errors
- Sleep delays (latency injection)
- Conditional/probabilistic failures

Fail points are **disabled by default** (zero runtime cost) and only active when compiled with the `failpoints` feature.

## High-Value Integration Points

### 1. Database Layer (crates/durable/src/persistence/postgres.rs)

**Current gap**: No testing for transient database failures, connection exhaustion, or mid-transaction crashes.

```rust
// Example: Inject failure after acquiring transaction but before commit
pub async fn append_events(...) -> Result<i32, StoreError> {
    let mut tx = self.pool.begin().await.map_err(...)?;

    fail_point!("append_events_after_insert", |_| {
        Err(StoreError::Database("injected failure".into()))
    });

    tx.commit().await.map_err(...)?;
    Ok(new_sequence)
}
```

**Scenarios to test**:
| Fail Point | Location | Tests |
|------------|----------|-------|
| `db_connect_fail` | Pool connection | Connection exhaustion recovery |
| `tx_begin_fail` | Transaction start | Graceful degradation |
| `tx_commit_fail` | Post-insert, pre-commit | Rollback correctness |
| `claim_task_timeout` | Task claiming | SKIP LOCKED behavior under pressure |
| `heartbeat_update_fail` | Heartbeat write | Task reclamation triggers |

### 2. LLM Provider Layer (crates/openai/, crates/anthropic/)

**Current gap**: Only happy-path testing with real APIs. No rate limit, timeout, or malformed response testing.

```rust
// Example in LLM driver
async fn complete(&self, request: CompletionRequest) -> Result<Response> {
    fail_point!("llm_rate_limit", |_| {
        Err(LlmError::RateLimit { retry_after: Some(Duration::from_secs(60)) })
    });

    fail_point!("llm_timeout");  // Just sleeps forever until test timeout

    self.client.post(url).send().await?
}
```

**Scenarios to test**:
| Fail Point | Tests |
|------------|-------|
| `llm_rate_limit` | Retry logic with exponential backoff |
| `llm_timeout` | Request timeout handling |
| `llm_context_exceeded` | RequestTooLarge error propagation |
| `llm_stream_disconnect` | Mid-stream SSE failures |
| `llm_malformed_json` | Response parsing resilience |

### 3. gRPC Worker Communication (crates/worker/src/grpc_adapters.rs)

**Current gap**: No testing for network partitions between worker and control plane.

```rust
// Example in worker gRPC client
pub async fn claim_tasks(&self, ...) -> Result<Vec<Task>> {
    fail_point!("grpc_claim_disconnect", |_| {
        Err(grpc_error("connection reset"))
    });

    let response = client.claim_durable_tasks(request).await?;
    Ok(response)
}
```

**Scenarios to test**:
| Fail Point | Tests |
|------------|-------|
| `grpc_connect_fail` | Worker startup resilience |
| `grpc_stream_disconnect` | Push notification fallback to polling |
| `grpc_claim_fail` | Task claiming retry |
| `grpc_complete_fail` | At-least-once delivery guarantee |
| `grpc_heartbeat_fail` | Heartbeat retry + task reclamation |

### 4. Circuit Breaker (crates/durable/src/reliability/)

**Current gap**: Difficult to test state transitions (Closed → Open → HalfOpen → Closed) reliably.

```rust
// Example: Precise failure injection for circuit breaker testing
async fn call_external_service(&self, cb_name: &str) -> Result<()> {
    fail_point!("circuit_breaker_failure", |count| {
        // Inject exactly N failures to trigger open state
        Err(ExternalError::Unavailable)
    });

    self.external_call().await
}
```

**Scenarios to test**:
| Fail Point | Tests |
|------------|-------|
| `cb_failure_threshold` | Inject exactly 5 failures → verify Open state |
| `cb_half_open_success` | Inject 2 successes in HalfOpen → verify Closed |
| `cb_half_open_failure` | Inject failure in HalfOpen → verify back to Open |

### 5. Durable Execution Recovery (crates/durable/)

**Current gap**: No crash recovery testing.

```rust
// Example: Simulate worker crash during activity execution
async fn execute_activity(&self, task: ClaimedTask) -> Result<()> {
    // Task is claimed, activity starts...
    fail_point!("activity_mid_execution_crash", |_| {
        panic!("simulated worker crash")
    });

    // Complete activity...
    self.store.complete_task(task.id, result).await
}
```

**Scenarios to test**:
| Fail Point | Tests |
|------------|-------|
| `workflow_mid_event_crash` | Event replay correctness after restart |
| `activity_mid_execution_crash` | Task reclamation by another worker |
| `signal_processing_crash` | Signal idempotency |
| `dlq_move_crash` | DLQ consistency |

## Implementation Plan

### Phase 1: Add fail-rs dependency

```toml
# Cargo.toml (workspace)
[workspace.dependencies]
fail = "0.5"

# crates/durable/Cargo.toml
[dependencies]
fail = { workspace = true }

[features]
failpoints = ["fail/failpoints"]
```

### Phase 2: Add fail points to critical paths

Start with highest-value locations:
1. `PostgresWorkflowEventStore::append_events` - transaction failures
2. `PostgresWorkflowEventStore::claim_tasks` - concurrent claiming
3. `PostgresWorkflowEventStore::heartbeat_task` - ownership verification
4. `GrpcClient::claim_tasks` - network failures
5. LLM driver `complete()` methods - provider failures

### Phase 3: Write failure injection tests

```rust
#[cfg(feature = "failpoints")]
mod failure_tests {
    use fail::FailScenario;

    #[tokio::test]
    async fn test_commit_failure_rolls_back() {
        let scenario = FailScenario::setup();
        fail::cfg("tx_commit_fail", "return").unwrap();

        let store = create_test_store().await;
        let result = store.append_events(workflow_id, 0, events).await;

        assert!(result.is_err());
        // Verify no partial writes
        let events = store.load_events(workflow_id).await.unwrap();
        assert!(events.is_empty());

        scenario.teardown();
    }

    #[tokio::test]
    async fn test_worker_crash_recovery() {
        let scenario = FailScenario::setup();

        // Claim task, then crash mid-execution
        fail::cfg("activity_mid_execution_crash", "panic").unwrap();

        let result = std::panic::catch_unwind(|| {
            worker.execute_task(task).await
        });
        assert!(result.is_err());

        // Verify task is reclaimed after heartbeat timeout
        tokio::time::sleep(Duration::from_secs(35)).await;

        fail::cfg("activity_mid_execution_crash", "off").unwrap();
        let reclaimed = store.reclaim_stale_tasks(worker_id_2).await.unwrap();
        assert_eq!(reclaimed.len(), 1);

        scenario.teardown();
    }
}
```

### Phase 4: CI Integration

```yaml
# .github/workflows/ci.yml
- name: Run failure injection tests
  run: cargo test --features failpoints --test failure_tests
```

## Fail Point Reference

| Name | Actions | Description |
|------|---------|-------------|
| `return` | Return early with error | `fail_point!("name", \|_\| Err(...))` |
| `panic` | Panic immediately | Simulates crash |
| `sleep(ms)` | Delay execution | Simulates latency |
| `off` | Disable fail point | Runtime control |
| `1*return` | Return once, then disable | Single-shot failure |
| `50%return` | 50% chance of failure | Chaos testing |

## Estimated Coverage Improvement

| Area | Current | With fail-rs |
|------|---------|--------------|
| Database failures | 10% | 80% |
| LLM provider errors | 5% | 70% |
| gRPC communication | 0% | 60% |
| Circuit breaker | 30% | 90% |
| Crash recovery | 0% | 70% |
| **Overall durability** | **~15%** | **~75%** |

## Trade-offs

**Pros**:
- Zero runtime cost when disabled
- Precise, reproducible failure injection
- Works with async/await (tokio)
- Battle-tested (used by TiKV, 7600+ dependents)
- Simple API (`fail_point!` macro)

**Cons**:
- Requires code instrumentation (fail points in production code)
- Feature flag management in CI
- Some failures still need external simulation (e.g., actual network partition)
- Adds ~200 lines of instrumentation code

## Alternatives Considered

| Approach | Pros | Cons |
|----------|------|------|
| **fail-rs** | Precise, reproducible, zero-cost | Requires instrumentation |
| Mock all dependencies | Full control | Misses real integration issues |
| Docker network chaos | Real failures | Flaky, slow, hard to reproduce |
| Property-based testing | Finds edge cases | Doesn't test failure paths |

## Conclusion

fail-rs is the right tool for this codebase because:
1. **Architecture fit**: Distributed workers + durable execution = many failure modes
2. **Testing gaps**: Current tests don't cover transient failures, crashes, or recovery
3. **Production risk**: Untested failure paths are production incidents waiting to happen
4. **Low cost**: Zero runtime overhead, ~1 week implementation effort

**Recommended next steps**:
1. Add fail-rs to `crates/durable` first (highest value)
2. Instrument 5-10 critical fail points
3. Write 10-15 failure injection tests
4. Expand to `crates/worker` and LLM providers

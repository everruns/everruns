# fail-rs Testing Specification

## Abstract

Durability testing via fail-rs failure injection for the durable execution engine. Fail points enable testing of error recovery paths without external chaos tools.

## Goals

1. Test recovery from database failures during transactions
2. Verify task reclamation after heartbeat failures
3. Validate circuit breaker state persistence under failures
4. Ensure no partial writes on commit failures
5. Enable deterministic testing of race conditions

## Non-Goals

1. Network partition simulation (use Docker/infrastructure chaos)
2. LLM provider failure testing (separate concern in core crate)
3. Performance testing under failures (use benchmarks)

## Requirements

### Dependency Configuration

Workspace `Cargo.toml`:
```toml
[workspace.dependencies]
fail = "0.5"
```

Crate `Cargo.toml`:
```toml
[dependencies]
fail = { workspace = true, optional = true }

[features]
failpoints = ["fail/failpoints"]
```

### Fail Point Naming Convention

Pattern: `{module}_{operation}_{phase}`

| Module | Operations |
|--------|------------|
| `postgres` | append_events, claim_task, complete_task, heartbeat |
| `circuit_breaker` | get_state, record_failure, transition |

### Fail Point Catalog

#### Persistence Layer (postgres.rs)

| Name | Location | Purpose |
|------|----------|---------|
| `postgres_append_events_after_insert` | After event inserts | Verify rollback on late failure |
| `postgres_append_events_before_commit` | Before tx.commit() | Test transaction atomicity |
| `postgres_claim_task_after_query` | After SKIP LOCKED query | Test claim recovery |
| `postgres_heartbeat_update` | After heartbeat UPDATE | Test heartbeat retry |
| `postgres_complete_task_after_update` | After completion UPDATE | Test completion retry |

#### Circuit Breaker (distributed_circuit_breaker.rs)

| Name | Location | Purpose |
|------|----------|---------|
| `circuit_breaker_get_state_db_fetch` | Before store.get | Test cache fallback |
| `circuit_breaker_record_failure_before_update` | Before failure record | Test state consistency |
| `circuit_breaker_transition_to_open` | Before state transition | Test transition atomicity |

### Test Patterns

#### Async Test with FailScenario

```rust
#[tokio::test]
async fn test_failure_scenario() {
    let scenario = FailScenario::setup();

    fail::cfg("postgres_append_events_before_commit", "return").unwrap();

    // Test code that should handle failure
    let result = store.append_events(...).await;
    assert!(result.is_err());

    fail::cfg("postgres_append_events_before_commit", "off").unwrap();
    scenario.teardown();
}
```

#### Standard Actions

| Action | Effect |
|--------|--------|
| `return` | Return error immediately |
| `panic` | Simulate crash |
| `sleep(ms)` | Inject latency |
| `1*return` | Fail once, then succeed |
| `50%return` | 50% failure rate |

### Running Tests

```bash
# Run all failure injection tests
cargo test -p everruns-durable --test failure_injection_test --features failpoints -- --test-threads=1

# Run specific test
cargo test -p everruns-durable --test failure_injection_test test_append_events_failure --features failpoints
```

## Decisions

### Fail Points in Production Code

**Decision**: Place `fail_point!` macros in production code, not test modules.

**Rationale**: Zero runtime cost when `failpoints` feature disabled. Tests actual code paths. Matches TiKV pattern.

### Single Feature Flag

**Decision**: Single `failpoints` feature enables all fail points.

**Rationale**: Simpler than per-module flags. All-or-nothing for testing.

### Separate Test File

**Decision**: `tests/failure_injection_test.rs` separate from integration tests.

**Rationale**: Different feature requirements. Cleaner separation of concerns.

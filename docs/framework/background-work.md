---
title: Session work and wakes
description: Run immediate and scheduled background work with explicit delivery and restart semantics.
sidebar:
  order: 3
---

# Session work and wakes

`everruns::work` lets an application request and handle work owned by a session
without importing runtime registries, platform stores, or task-kind constants.
The application chooses its own work kinds and JSON payloads.

```rust
use std::time::Duration;
use everruns::work::{TaskOutcome, TaskRequest, WakePolicy, WorkQueue};
use serde_json::json;

# async fn example() -> Result<(), everruns::work::WorkError> {
let queue = WorkQueue::in_memory();
let work = queue.for_session("session_123");

work.submit(
    TaskRequest::new("thumbnail", json!({ "image": "cover.png" }))
        .idempotency_key("thumbnail:cover.png")
        .wake_policy(WakePolicy::OnCompletion),
).await?;

for delivery in queue.claim_due(Duration::from_secs(30), 16).await? {
    // Route on delivery.task.kind and check cancellation before side effects.
    queue.finish(
        &delivery,
        TaskOutcome::success(json!({ "path": "cover-thumb.png" })),
    ).await?;
}
# Ok(())
# }
```

The runnable version is
[`session_work.rs`](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/session_work.rs).

## Contract

- **Ownership:** every task and wake has one opaque `session_id`. A
  `SessionWork` handle fixes that owner for all requests and reads.
- **Persistence and restart:** `WorkQueue::in_memory()` is process-local and
  database-free. State survives replacing the queue only while the same
  `Arc<InMemoryWorkBackend>` is retained. It does not survive a process
  restart. A host that needs durable recovery supplies a `WorkBackend`.
- **Scheduling:** `Immediate` work is claimable now; `At(SystemTime)` work is
  not claimable early. Both are one-shot requests. The host owns polling;
  recurring calendars and schedule runners stay host concerns. There is no
  hidden scheduler or database in the default build.
- **Delivery:** task and wake claims are leased and at least once. If a process
  stops before settlement or acknowledgment, the item is claimable after the
  lease expires. Each retry has a new token and attempt; stale attempts cannot
  settle newer work.
- **Idempotency:** task keys and direct-wake keys each have a session-scoped
  namespace. Repeating the same request returns the original task or wake.
  Reusing a key for different input fails with `IdempotencyConflict`. Workers
  should also deduplicate external side effects on the stable task id or
  submission key.
- **Cancellation:** pending work cancels immediately. Running work records
  cancellation intent; the worker checks the latest task snapshot, stops
  cooperatively, then reports `TaskOutcome::Canceled`. A task may still
  succeed if it passes its safe cancellation point first.
- **Wakes:** applications can request an immediate wake directly. A task can
  also create one atomic completion wake with `WakePolicy::OnCompletion`.
  Wakes use the same lease/retry/acknowledgment model as tasks.

Durable platform scheduling, retention, distributed polling, and multi-host
coordination belong in the host's `WorkBackend`; they are not enabled by the
offline Framework default. The in-memory backend retains accepted payloads
until it is dropped and does not enforce admission quotas, so hosted providers
must apply tenant authorization, payload limits, quotas, and retention at their
own boundary.

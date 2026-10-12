---
title: Durable Execution Engine Setup
description: "Run Everruns with the PostgreSQL-backed durable execution engine: database setup, migrations, and worker configuration."
---

This guide explains how to run Everruns with the custom PostgreSQL-backed durable execution engine.

## Overview

Durable execution is PostgreSQL-backed and provides:
- Turns that survive worker crashes: each session is leased by one worker, and the turn resumes from the session's event log on another
- Background tasks with automatic retries
- Distributed task queue with backpressure support
- Circuit breakers and dead letter queues
- No additional infrastructure required (uses existing PostgreSQL)

## Quick Start

### 1. Prerequisites

- PostgreSQL running and accessible
- `DATABASE_URL` environment variable set
- Migrations applied (includes durable tables)

### 2. Start the API

The durable engine is the only execution engine, so there is no mode to select.

```bash
export DATABASE_URL="postgres://postgres:postgres@localhost/everruns"

# Start the API server
cargo run -p everruns-server
```

### 3. Start Durable Worker

In a separate terminal:

```bash
# Workers only need gRPC address - NO DATABASE_URL required!
export SERVER_GRPC_ADDRESS="127.0.0.1:9001"

# Start the task worker
cargo run -p everruns-worker
```

**Important:** Workers communicate with the control-plane via gRPC and do not
require direct database access. This improves security and simplifies deployment.

Or programmatically:

```rust
use everruns_worker::{TaskWorkerConfig, WorkerAppBuilder};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    WorkerAppBuilder::new(TaskWorkerConfig::from_env())
        .run()
        .await
}
```

## Configuration

### Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `DATABASE_URL` | PostgreSQL connection URL (server only; workers do not need it) | Required |
| `SERVER_GRPC_ADDRESS` | Server gRPC address (`WORKER_GRPC_ADDRESS` legacy alias) | `127.0.0.1:9001` |
| `WORKER_GRPC_AUTH_TOKEN` | Bearer token for gRPC auth | Unset (disabled) |
| `WORKER_ID` | Unique worker identifier | Auto-generated |
| `MAX_CONCURRENT_TASKS` | Max tasks per worker | `50` |
| `MAX_CONCURRENT_MODEL_WAITS` | Max reason steps per worker that free their slot while waiting on the model (`0` = off) | `200` |

### Database Tables

The durable engine uses these tables (created by migration 002_durable_execution):

- `durable_workflow_instances` - Workflow state and metadata
- `durable_workflow_events` - Event sourcing log
- `durable_task_queue` - Distributed task queue
- `durable_dead_letter_queue` - Failed tasks for manual inspection
- `durable_workers` - Worker registration and heartbeats
- `durable_signals` - Workflow signals (cancel, custom)
- `durable_circuit_breaker_state` - Circuit breaker states

## Turn Lifecycle

Turns do not run on the task queue. Each session is an actor in `session_actors`:

1. **Message Created**: the API stores the message and marks the session runnable in the same transaction
2. **Lease**: a worker leases the session (`LeaseActors`) and runs the whole turn, renewing the lease every few seconds
3. **Steering**: messages that arrive mid-turn reach the turn before its next model call
4. **Completion**: the worker releases the session; a message that arrived as the turn ended keeps it runnable for the next turn
5. **Crash**: an unrenewed lease expires after 10 seconds and another worker resumes the turn from the session's event log

The task queue carries the background activities: leased-resource cleanup, the session task reaper, and scheduled channel and agent-trigger invocations.

### Check Running Sessions

```sql
SELECT session_id, org_id, lease_holder, lease_expires_at, runnable_since, attempts
FROM session_actors
WHERE runnable_since IS NOT NULL OR lease_holder IS NOT NULL
ORDER BY runnable_since;
```

A session runnable for a long time with no holder means no worker is leasing (workers down or drained). A high `attempts` means its turn keeps losing its worker; past `DURABLE_NO_PROGRESS_SEAL_THRESHOLD` (default 3) the next worker seals the turn.

## Monitoring

### Check Active Workflows

```sql
SELECT id, workflow_type, status, created_at
FROM durable_workflow_instances
WHERE status IN ('pending', 'running')
ORDER BY created_at DESC;
```

### Check Pending Tasks

```sql
SELECT id, workflow_id, activity_type, status, attempt
FROM durable_task_queue
WHERE status = 'pending'
ORDER BY created_at;
```

### Check Dead Letter Queue

```sql
SELECT id, workflow_id, activity_type, last_error, dead_at
FROM durable_dead_letter_queue
ORDER BY dead_at DESC;
```

### Check Worker Status

```sql
SELECT id, status, current_load, last_heartbeat_at
FROM durable_workers
WHERE status = 'active';
```

## Crash Recovery

The durable execution engine provides automatic crash recovery through:

### Worker Heartbeats

Workers send heartbeats every 10 seconds while executing tasks. If a worker crashes:

1. The task remains in `claimed` status with stale `heartbeat_at`
2. Control-plane background task detects stale tasks (30s threshold)
3. Stale tasks are automatically reset to `pending` status
4. Another worker can claim and retry the task

### Stale Task Reclamation

The control-plane runs a background task (every 10s) that:

- Finds tasks with `status = 'claimed'` and `heartbeat_at` older than 30s
- Resets them to `pending` status
- Logs reclaimed task IDs for monitoring

```sql
-- View tasks that may need reclamation
SELECT id, workflow_id, activity_type, claimed_by, heartbeat_at
FROM durable_task_queue
WHERE status = 'claimed'
  AND heartbeat_at < NOW() - INTERVAL '30 seconds';
```

## Troubleshooting

### Worker Not Processing Tasks

1. Check worker is running and connected to correct `SERVER_GRPC_ADDRESS`
2. Verify `activity_types` match task types in queue
3. Check worker heartbeat in `durable_workers` table

### Workflows Stuck in Running

1. Check for claimed tasks that haven't completed
2. Look for errors in worker logs
3. Check DLQ for failed tasks
4. Wait for stale task reclamation (30s threshold)

### Task Retries Exhausted

Tasks moved to DLQ after exhausting retries:

```sql
-- View DLQ entries
SELECT * FROM durable_dead_letter_queue ORDER BY dead_at DESC;

-- Requeue a task
UPDATE durable_dead_letter_queue SET requeued_at = NOW() WHERE id = '<dlq_id>';
```

## Implementation Status

| Phase | Status | Description |
|-------|--------|-------------|
| Phase 1-4 | ✅ Complete | Core abstractions, persistence, reliability, worker pool |
| Phase 6 | 🔄 Planned | Scale Testing (1000+ concurrent workers) |
| Phase 7 | ✅ Core Complete | gRPC-based worker integration, crash recovery |

Metrics and tracing are covered under [Observability](/observability/opentelemetry/).

---
title: Durable Execution Engine Setup
description: How to run Everruns with the custom durable execution engine instead of Temporal
---

# Durable Execution Engine Setup Guide

This guide explains how to run Everruns with the custom durable execution engine instead of Temporal.

## Overview

The durable execution engine is a PostgreSQL-backed workflow orchestration system that provides:
- Event-sourced workflows with automatic retries
- Distributed task queue with backpressure support
- Circuit breakers and dead letter queues
- No additional infrastructure required (uses existing PostgreSQL)

## Quick Start

### 1. Prerequisites

- PostgreSQL running and accessible
- `DATABASE_URL` environment variable set
- Migrations applied (includes durable tables)

### 2. Start API in Durable Mode

```bash
# Set runner mode to durable
export RUNNER_MODE=durable
export DATABASE_URL="postgres://postgres:postgres@localhost/everruns"

# Start the API server
cargo run -p everruns-control-plane
```

You should see:
```
Using Durable execution engine runner (PostgreSQL-backed)
```

### 3. Start Durable Worker

In a separate terminal:

```bash
# Workers only need gRPC address - NO DATABASE_URL required!
export GRPC_ADDRESS="127.0.0.1:9001"

# Start the durable worker
cargo run -p everruns-worker --bin durable-worker
```

**Important:** Workers communicate with the control-plane via gRPC and do not
require direct database access. This improves security and simplifies deployment.

Or programmatically:

```rust
use everruns_worker::{DurableWorker, DurableWorkerConfig};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut worker = DurableWorker::from_env().await?;
    worker.run().await
}
```

## Configuration

### Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `RUNNER_MODE` | Runner mode: `temporal` or `durable` | `temporal` |
| `DATABASE_URL` | PostgreSQL connection URL | Required |
| `GRPC_ADDRESS` | Control-plane gRPC address | `127.0.0.1:9001` |
| `WORKER_ID` | Unique worker identifier | Auto-generated |
| `MAX_CONCURRENT_TASKS` | Max tasks per worker | `10` |

### Database Tables

The durable engine uses these tables (created by migration 008):

- `durable_workflow_instances` - Workflow state and metadata
- `durable_workflow_events` - Event sourcing log
- `durable_task_queue` - Distributed task queue
- `durable_dead_letter_queue` - Failed tasks for manual inspection
- `durable_workers` - Worker registration and heartbeats
- `durable_signals` - Workflow signals (cancel, custom)
- `durable_circuit_breaker_state` - Circuit breaker states

## Testing

### Unit Tests (No Dependencies)

```bash
cargo test -p everruns-durable --lib
```

Expected: 91+ tests passing

### Integration Tests (Requires PostgreSQL)

```bash
# Create test database
psql -U postgres -c "CREATE DATABASE everruns_test;"

# Run migrations
DATABASE_URL="postgres://postgres:postgres@localhost/everruns_test" \
  sqlx migrate run --source crates/control-plane/migrations

# Run integration tests
DATABASE_URL="postgres://postgres:postgres@localhost/everruns_test" \
  cargo test -p everruns-durable --test postgres_integration_test -- --test-threads=1
```

Expected: 17 tests passing

## Switching Between Modes

### Temporal Mode (Default)

```bash
export RUNNER_MODE=temporal
# Or simply don't set RUNNER_MODE
```

Requires:
- Temporal server running
- `TEMPORAL_ADDRESS` (default: `localhost:7233`)
- `TEMPORAL_NAMESPACE` (default: `default`)
- `TEMPORAL_TASK_QUEUE` (default: `everruns-agent-runs`)

### Durable Mode

```bash
export RUNNER_MODE=durable
```

Requires:
- PostgreSQL with migrations applied
- `DATABASE_URL` set

## Workflow Lifecycle

1. **Message Created**: User sends message via API
2. **Workflow Started**: `DurableRunner` creates workflow and enqueues `process_input` task
3. **Input Processing**: Worker claims task, processes input, enqueues `reason` task
4. **LLM Reasoning**: Worker executes LLM call, may enqueue `act` tasks for tools
5. **Completion**: Workflow marked as `completed` after final response

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

1. Check worker is running and connected to correct `GRPC_ADDRESS`
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

## Architecture Comparison

| Feature | Temporal | Durable |
|---------|----------|---------|
| Infrastructure | Temporal Server + DB | PostgreSQL only |
| Task Queue | Temporal queues | PostgreSQL table |
| Event Sourcing | Temporal history | `durable_workflow_events` |
| Circuit Breakers | Client-side | PostgreSQL-backed |
| Worker Registry | Temporal server | `durable_workers` table |
| Scalability | Proven at scale | Designed for 1000+ workers |

## Implementation Status

| Phase | Status | Description |
|-------|--------|-------------|
| Phase 1-4 | ✅ Complete | Core abstractions, persistence, reliability, worker pool |
| Phase 5 | 🔄 Planned | Observability & Metrics (OpenTelemetry integration) |
| Phase 6 | 🔄 Planned | Scale Testing (1000+ concurrent workers) |
| Phase 7 | ✅ Core Complete | gRPC-based worker integration, crash recovery |

The durable execution engine is production-ready for single-instance deployments.
Both Temporal and Durable modes are supported concurrently.

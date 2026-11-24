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
- Server startup applies the embedded migrations, including session actors and background-task tables

### 2. Start the API

The Platform uses session actors for agent turns and the durable task queue for background work; there is no legacy workflow mode to select. For a local-only two-process setup, configure a development deployment grade without enabling `DEV_MODE` (which runs workers in-process):

```bash
export DATABASE_URL="postgres://postgres:postgres@localhost/everruns"
export DEPLOYMENT_GRADE=dev
export AUTH_MODE=none
export WORKER_GRPC_AUTH_TOKEN="$(openssl rand -hex 32)"
export SECRETS_ENCRYPTION_KEY="kek-v1:$(openssl rand -base64 32)"

# Start the API server
cargo run -p everruns-server
```

### 3. Start Durable Worker

In a separate terminal:

```bash
# Workers need the matching gRPC token, but NO DATABASE_URL.
export WORKER_GRPC_AUTH_TOKEN="<same value as the server>"
export SERVER_GRPC_ADDRESS="127.0.0.1:9001"

# Start the task worker
cargo run -p everruns-worker
```

For a non-development deployment, configure `AUTH_MODE=admin`, `full`, or `external` and its required settings as described in [Authentication](/sre/runbooks/authentication/). Keep the encryption key stable across restarts.

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
| `WORKER_GRPC_AUTH_TOKEN` | Same bearer token on server and workers | Required unless `DEV_MODE=true` |
| `WORKER_ID` | Unique worker identifier | Auto-generated |
| `MAX_CONCURRENT_TASKS` | Max queued background tasks per worker (session turns do not count) | `50` |
| `WORKER_MAX_RUNNING_TURNS` | Max session turns per worker (`0` = from the memory limit) | from the memory limit |

### Database Tables

Current embedded migrations create two distinct sets of records. Agent turns use:

- `session_actors` - Runnable state, lease ownership, fencing and inbox cursor
- `events` - Durable session journal used to reconstruct turns

The generic background-task subsystem also retains:

- `durable_workflow_instances` - Workflow state and metadata
- `durable_workflow_events` - Event sourcing log
- `durable_task_queue` - Distributed task queue
- `durable_dead_letter_queue` - Failed tasks for manual inspection
- `durable_workers` - Worker registration and heartbeats
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

### Check Generic Workflow Records

These are not session turns. Use the `session_actors` query above to inspect running agents.

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

Session turns recover through lease expiry and journal reconstruction described above. The heartbeat and stale-task mechanisms below apply to queued background activities, not to each reason/tool step of an agent turn.

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

### Background Tasks Stuck in Running

1. Check for claimed tasks that haven't completed
2. Look for errors in worker logs
3. Check DLQ for failed tasks
4. Wait for stale task reclamation (30s threshold)

### Task Retries Exhausted

Tasks moved to DLQ after exhausting retries:

```sql
-- View DLQ entries
SELECT * FROM durable_dead_letter_queue ORDER BY dead_at DESC;
```

To retry after addressing the cause, use the platform-admin endpoint
`POST /v1/platform/durable/dlq/{dlq_id}/retry` (under your configured API prefix).
It creates a new queued task and records the requeue in one transaction.
Updating `requeued_at` alone does not enqueue any work. Inspect the task's
external effects before retrying it; a retry can repeat them.

Metrics and tracing are covered under [Observability](/observability/opentelemetry/).

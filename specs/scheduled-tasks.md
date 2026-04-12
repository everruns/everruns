# Scheduled Tasks Specification

## Abstract

Scheduled tasks extend the durable execution engine with cron-based scheduling capabilities. Users can define recurring tasks that automatically trigger workflows or activities at specified intervals. This feature integrates seamlessly with existing durable infrastructure—sharing the same reliability guarantees, observability, and management APIs.

See `specs/localization.md` for how schedule timezone interacts with session and user timezone defaults during execution.

## Goals

1. **Cron-based scheduling**: Support standard cron expressions for flexible scheduling
2. **Durability**: Schedules survive restarts; missed executions handled gracefully
3. **Multi-instance safe**: Multiple control-plane instances can run without duplicate triggers
4. **Observable**: Same OTel tracing, admin UI, and APIs as other durable features
5. **Testable**: In-memory implementation for unit tests; smoke tests for integration

## Non-Goals

1. Sub-second precision (minimum granularity is 1 second)
2. Complex DAG scheduling (use workflows for dependencies)
3. Calendar-based scheduling (holidays, business days)

## Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│                     Control Plane                                    │
│  ┌─────────────────┐    ┌──────────────────────────────────────┐   │
│  │  HTTP API       │    │  DurableScheduler                     │   │
│  │  /v1/durable/   │    │  - Polls durable_schedules table      │   │
│  │  schedules/*    │    │  - Claims due schedules (SKIP LOCKED) │   │
│  └────────┬────────┘    │  - Creates workflow/activity          │   │
│           │             │  - Updates next_trigger_at            │   │
│           │             └──────────────────┬───────────────────┘   │
│           │                                │                        │
│           ▼                                ▼                        │
│  ┌─────────────────────────────────────────────────────────────┐   │
│  │                    PostgreSQL                                │   │
│  │  ┌───────────────────┐  ┌─────────────────────────────────┐ │   │
│  │  │ durable_schedules │  │ durable_schedule_executions     │ │   │
│  │  │ - cron expression │  │ - execution history             │ │   │
│  │  │ - target workflow │  │ - status, duration, errors      │ │   │
│  │  │ - next_trigger_at │  └─────────────────────────────────┘ │   │
│  │  └───────────────────┘                                       │   │
│  └─────────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────┘
```

## Data Models

### Schedule

See the `durable_schedules` migration for the full schema. Key fields: `id` (UUIDv7), `name` (unique), `cron_expression`, `timezone` (IANA, default UTC), `target_type` (workflow/activity), `target_name`, `target_input` (JSONB), `enabled`, `max_concurrent`, `catch_up_missed`, `retry_policy`, `next_trigger_at` (indexed for polling).

### ScheduleExecution

See the `durable_schedule_executions` migration for the full schema. Tracks each trigger attempt with `schedule_id`, `scheduled_at`, `status` (pending/running/completed/failed/skipped), linked `workflow_id` or `task_id`, and `duration_ms`.

### Cron Expression Format

Standard 5-field cron with optional 6th field for seconds. Parsed via the `cron` crate (v0.13). Examples: `*/30 * * * *` (every 30 min), `0 9 * * MON-FRI` (9 AM weekdays).

## Timezone Semantics

- `schedule.timezone` is authoritative for cron interpretation.
- When a schedule fires, `schedule.timezone` becomes the default execution timezone for that scheduled turn.
- `session.timezone` is still relevant as a fallback if a scheduled trigger lacks a specific timezone in the future, but it must not override cron interpretation for an existing schedule.
- Interactive browser timezone is never used for already-running background schedules.

## API Endpoints

### Schedules

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | `/v1/durable/schedules` | Create schedule |
| GET | `/v1/durable/schedules` | List schedules |
| GET | `/v1/durable/schedules/{id}` | Get schedule details |
| PATCH | `/v1/durable/schedules/{id}` | Update schedule |
| DELETE | `/v1/durable/schedules/{id}` | Delete schedule |
| POST | `/v1/durable/schedules/{id}/trigger` | Manual trigger |
| POST | `/v1/durable/schedules/{id}/pause` | Pause schedule |
| POST | `/v1/durable/schedules/{id}/resume` | Resume schedule |

### Executions

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/v1/durable/schedules/{id}/executions` | List executions for schedule |
| GET | `/v1/durable/executions/{id}` | Get execution details |

### Request/Response Types

For the complete request/response schemas (`CreateScheduleRequest`, `ScheduleTarget`, `ScheduleResponse`, `ScheduleStats`), run `./scripts/export-openapi.sh` or see the generated OpenAPI spec. Key design choices: `ScheduleTarget` is a tagged enum (`workflow` or `activity`) with type-specific input; responses include aggregated `ScheduleStats`.

## Scheduler Component

### DurableScheduler

The `DurableScheduler` component polls for due schedules on a configurable interval. For each due schedule it: checks `max_concurrent`, creates an execution record, triggers the target (workflow or activity), and updates the execution status. See `crates/durable/src/scheduler.rs` for implementation.

### Multi-Instance Safety

The scheduler uses `SELECT ... FOR UPDATE SKIP LOCKED` to claim due schedules, ensuring only one instance processes each due schedule. Stale claims (no update for 30s) are reclaimed.

### Catch-Up Handling

On startup or when re-enabling a schedule: if `catch_up_missed` is false, just advance `next_trigger_at` to the future. Otherwise, calculate missed triggers since `last_triggered_at` (capped by `max_catch_up`, default 1) and execute them.

## WorkflowEventStore Extensions

The `WorkflowEventStore` trait is extended with schedule CRUD, scheduler claiming/triggering, execution tracking, and stats methods. See `crates/durable/src/store.rs` for the full trait definition.

## Database Schema

See the scheduled_tasks migration in `crates/server/migrations/` for the full DDL. Key tables: `durable_schedules` (schedule definitions with polling index on `next_trigger_at`), `durable_schedule_executions` (execution history with indexes for schedule listing and running-count queries), and `durable_scheduler_instances` (heartbeat-based instance registration for horizontal scaling). Note: no `org_id` — multi-tenancy will be added to the entire durable engine in a future PR.

## UI Design

### Pages

1. **Schedules List** (`/durable/schedules`)
   - Table with: Name, Cron, Target, Status, Next Run, Last Run, Success Rate
   - Actions: Create, Edit, Delete, Pause/Resume, Trigger Now
   - Filters: Status (all, enabled, disabled), Target Type

2. **Schedule Detail** (`/durable/schedules/{id}`)
   - Header: Name, description, status badge
   - Info cards: Cron expression (human-readable), Timezone, Target details
   - Stats cards: Total runs, Success rate, Avg duration, Next run countdown
   - Executions table: Recent executions with status, duration, links to workflows

3. **Create/Edit Schedule** (Modal or page)
   - Form fields for all schedule properties
   - Cron expression builder with preview of next 5 runs
   - Target type selector with dynamic input fields

### Components

UI components live in `apps/ui/src/components/schedules/` (schedule-list, schedule-card, schedule-form, schedule-stats, execution-list, cron-builder, cron-preview).

### API Client and Hooks

See `apps/ui/src/lib/api/schedules.ts` for the TypeScript API client and `apps/ui/src/hooks/use-schedules.ts` for React Query hooks. Standard CRUD + trigger/pause/resume operations with query cache invalidation and SSE integration.

## SSE Integration

Extends existing durable SSE with schedule events: `ScheduleCreated`, `ScheduleUpdated`, `ScheduleDeleted`, `ScheduleTriggered`, `ExecutionCompleted`. See the `DurableSSEEvent` enum in server code.

## Testing

Key test scenarios (unit and integration):
- Cron parsing and next-trigger calculation
- Schedule CRUD via in-memory store
- Multi-instance claiming (concurrent `claim_due_schedules` — only one wins)
- `max_concurrent` enforcement (second trigger skipped when limit reached)
- End-to-end: schedule triggers workflow, execution record created with workflow_id

## Test Cases (Manual)

### TC001: Create Schedule - Basic

**Description**: Verify basic schedule creation with required fields.

**Preconditions**:
- API server is running
- At least one workflow type is registered

**Test Data**:
| Field | Value |
|-------|-------|
| Name | ping-service |
| Cron | */30 * * * * |
| Target Type | workflow |
| Workflow Type | health_check |

**Steps**:
1. Navigate to Durable > Schedules
2. Click "Create Schedule"
3. Enter test data
4. Click "Create"

**Expected Result**:
- Schedule appears in list
- Status shows "Enabled"
- Next Run shows time within 30 minutes

### TC002: Manual Trigger

**Description**: Verify manual trigger creates execution.

**Preconditions**:
- Schedule "ping-service" exists and is enabled

**Steps**:
1. Navigate to schedule detail page
2. Click "Trigger Now" button
3. Confirm in dialog

**Expected Result**:
- Execution appears in history immediately
- Status shows "Running" then "Completed"
- Workflow ID is populated

### TC003: Pause and Resume

**Description**: Verify pause prevents triggers, resume restores them.

**Preconditions**:
- Schedule exists with 1-minute cron

**Steps**:
1. Click "Pause" on schedule
2. Wait 2 minutes
3. Verify no new executions
4. Click "Resume"
5. Wait 2 minutes

**Expected Result**:
- After pause: No new executions, Next Run shows "-"
- After resume: Next Run recalculated, new execution within 2 minutes

### TC004: Max Concurrent Enforcement

**Description**: Verify max_concurrent prevents overlapping executions.

**Preconditions**:
- Schedule with max_concurrent = 1
- Target workflow takes 5+ minutes

**Steps**:
1. Trigger schedule manually
2. Wait for execution to start
3. Trigger again immediately

**Expected Result**:
- Second trigger is skipped
- Only one execution running at a time

### TC005: Execution History

**Description**: Verify execution history displays correctly.

**Preconditions**:
- Schedule with multiple past executions

**Steps**:
1. Navigate to schedule detail
2. View executions table
3. Click on completed execution
4. Click on failed execution (if any)

**Expected Result**:
- Executions sorted by most recent first
- Each shows: scheduled time, duration, status
- Clicking links to workflow detail

## Smoke Test Scenarios

Smoke test script at `.claude/skills/smoke-test/smoke-test-schedules.sh` covers: create schedule, verify in list, manual trigger, wait for execution completion, pause, resume, delete.

## Implementation Phases

### Phase 1: Core Infrastructure
1. Add cron parsing library dependency
2. Create database migration
3. Implement `WorkflowEventStore` trait extensions
4. Implement `InMemoryWorkflowEventStore` schedule operations
5. Implement `PostgresWorkflowEventStore` schedule operations

### Phase 2: Scheduler Component
1. Implement `DurableScheduler` component
2. Add scheduler startup to control-plane
3. Implement multi-instance coordination
4. Add catch-up logic

### Phase 3: API Layer
1. Add schedule routes to `durable.rs`
2. Add request/response types
3. Update OpenAPI spec
4. Add SSE events

### Phase 4: UI
1. Create API client and hooks
2. Build schedule list page
3. Build schedule detail page
4. Build create/edit form with cron builder
5. Add SSE integration

### Phase 5: Testing & Documentation
1. Unit tests for cron parsing
2. Unit tests for store operations
3. Integration tests for scheduler
4. Manual test cases
5. Smoke test script
6. Update documentation

## Dependencies

- **Rust**: `cron` (cron expression parsing), `chrono-tz` (timezone handling)
- **npm**: `cronstrue` (human-readable cron descriptions), `cron-parser` (next-run preview)

## Decisions

1. **Cron library**: Using `cron` crate (most popular, well-maintained)
2. **Timezone handling**: Store IANA timezone, convert to UTC for next_trigger_at calculations
3. **Polling interval**: 1 second default (configurable)
4. **Claim timeout**: 30 seconds (same as task heartbeat timeout)
5. **Activity-only schedules**: Supported for fire-and-forget jobs without workflow overhead

## Multi-Tenancy

**Note**: Multi-tenancy is deferred until the entire durable engine supports it. The core durable tables (workflows, tasks, events) don't have `org_id`, so schedules are consistent with them. Multi-tenancy will be added in a future PR that updates all durable tables simultaneously.

## Resource Limits

### System Limits

| Limit | Default | Description |
|-------|---------|-------------|
| `scheduler_poll_interval_ms` | 1000 | How often scheduler checks for due schedules |
| `scheduler_batch_size` | 100 | Maximum schedules to claim per poll cycle |
| `scheduler_claim_timeout_s` | 30 | Reclaim schedules from dead scheduler instances |
| `max_pending_triggers` | 10000 | Backpressure: pause scheduling if queue exceeds this |

## Horizontal Scalability

The scheduler supports multiple control-plane instances running concurrently.

### Multi-Instance Coordination

1. **Schedule claiming**: `SELECT ... FOR UPDATE SKIP LOCKED` ensures only one instance processes each due schedule
2. **Heartbeats**: Each scheduler instance registers in `durable_scheduler_instances` table
3. **Leader election**: Not required - all instances are equal, work is distributed via SKIP LOCKED

### Scheduler Instance Registration

Each scheduler instance registers in `durable_scheduler_instances` with heartbeats. Stale instances (no heartbeat for 60s) are ignored when reclaiming schedules.

### Load Distribution

With SKIP LOCKED:
- Multiple instances can claim schedules concurrently
- No hot spots or contention
- Linear horizontal scaling

## Non-Functional Requirements

### Service Level Objectives (SLOs)

| Metric | Target | Measurement |
|--------|--------|-------------|
| Trigger latency P50 | < 2s | `started_at - scheduled_at` |
| Trigger latency P99 | < 10s | `started_at - scheduled_at` |
| Scheduler availability | 99.9% | Uptime of scheduler process |
| Execution success rate | 99% | `completed / (completed + failed)` |

### Metrics (Future Implementation)

Prometheus metrics on `/metrics` endpoint (future): counters for triggers/executions by status, gauges for active schedules and queue depth, histograms for trigger latency and execution duration.

### Alerting Thresholds (Future Implementation)

| Alert | Condition | Severity |
|-------|-----------|----------|
| SchedulerDown | No heartbeat for 60s | Critical |
| TriggerLatencyHigh | P99 > 30s for 5m | Warning |
| QueueBackpressure | pending_triggers > 8000 | Warning |
| ExecutionFailureSpike | failure_rate > 10% for 5m | Warning |

## Fail-Rs Testing

Add fail points following naming convention `{module}_{operation}_{phase}` per `specs/fail-rs-testing.md`.

### Fail Points Catalog

| Fail Point | Purpose |
|------------|---------|
| `postgres_claim_due_schedules_query` | Test scheduler recovery from DB failure during claiming |
| `postgres_create_schedule_execution_insert` | Test trigger failure handling |
| `postgres_complete_schedule_trigger_update` | Test partial completion scenarios |
| `scheduler_trigger_workflow_create` | Test workflow creation failure |
| `scheduler_trigger_activity_enqueue` | Test activity enqueue failure |

Tests verify scheduler recovery: inject failure via fail point, confirm error, disable fail point, confirm success.

## Benchmarks

Add to `crates/durable/benches/`:

| Benchmark | Target | Description |
|-----------|--------|-------------|
| `scheduler_throughput` | 1000 triggers/second | Sustained trigger rate |
| `scheduler_cold_start` | P50 < 2s | Time from due to trigger start |

Benchmarks in `crates/durable/benches/` using Criterion.

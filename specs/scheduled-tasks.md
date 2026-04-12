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

```rust
pub struct DurableScheduler<S: WorkflowEventStore> {
    store: Arc<S>,
    registry: Arc<WorkflowRegistry>,
    poll_interval: Duration,
    instance_id: String,
}

impl<S: WorkflowEventStore> DurableScheduler<S> {
    pub async fn run(&self, shutdown: CancellationToken) {
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => break,
                _ = tokio::time::sleep(self.poll_interval) => {
                    if let Err(e) = self.process_due_schedules().await {
                        tracing::error!(error = %e, "Failed to process schedules");
                    }
                }
            }
        }
    }

    async fn process_due_schedules(&self) -> Result<()> {
        // Claim due schedules (FOR UPDATE SKIP LOCKED)
        let due = self.store.claim_due_schedules(10).await?;

        for schedule in due {
            // Check max_concurrent
            if let Some(max) = schedule.max_concurrent {
                let running = self.store.count_running_executions(schedule.id).await?;
                if running >= max {
                    // Skip this trigger, update next_trigger_at
                    self.store.skip_schedule_trigger(schedule.id).await?;
                    continue;
                }
            }

            // Create execution record
            let execution_id = self.store.create_schedule_execution(
                schedule.id,
                schedule.next_trigger_at,
            ).await?;

            // Trigger target
            let result = match &schedule.target {
                ScheduleTarget::Workflow { workflow_type, input } => {
                    self.trigger_workflow(workflow_type, input.clone()).await
                }
                ScheduleTarget::Activity { activity_type, input } => {
                    self.trigger_activity(activity_type, input.clone()).await
                }
            };

            // Update execution and schedule
            match result {
                Ok(target_id) => {
                    self.store.complete_schedule_trigger(
                        schedule.id,
                        execution_id,
                        target_id,
                    ).await?;
                }
                Err(e) => {
                    self.store.fail_schedule_trigger(
                        schedule.id,
                        execution_id,
                        &e.to_string(),
                    ).await?;
                }
            }
        }
        Ok(())
    }
}
```

### Multi-Instance Safety

The scheduler uses `SELECT ... FOR UPDATE SKIP LOCKED` to claim due schedules:

```sql
-- Claim due schedules
UPDATE durable_schedules
SET claimed_by = $1, claimed_at = NOW()
WHERE id IN (
    SELECT id FROM durable_schedules
    WHERE enabled = true
      AND next_trigger_at <= NOW()
      AND (claimed_by IS NULL OR claimed_at < NOW() - INTERVAL '30 seconds')
    ORDER BY next_trigger_at
    LIMIT $2
    FOR UPDATE SKIP LOCKED
)
RETURNING *;
```

### Catch-Up Handling

On startup or when re-enabling a schedule:

```rust
async fn handle_catch_up(&self, schedule: &Schedule) -> Result<()> {
    if !schedule.catch_up_missed {
        // Just update next_trigger_at to future
        return self.store.update_next_trigger(schedule.id).await;
    }

    let missed = self.calculate_missed_triggers(
        schedule.last_triggered_at,
        &schedule.cron_expression,
        schedule.max_catch_up.unwrap_or(1),
    );

    for trigger_time in missed {
        self.trigger_schedule(schedule, trigger_time).await?;
    }

    Ok(())
}
```

## WorkflowEventStore Extensions

Add to `WorkflowEventStore` trait:

```rust
// Schedule operations
async fn create_schedule(&self, schedule: CreateScheduleRow) -> Result<Uuid, StoreError>;
async fn get_schedule(&self, id: Uuid) -> Result<ScheduleRow, StoreError>;
async fn list_schedules(&self, filter: ScheduleFilter, pagination: Pagination) -> Result<Vec<ScheduleRow>, StoreError>;
async fn update_schedule(&self, id: Uuid, update: UpdateSchedule) -> Result<(), StoreError>;
async fn delete_schedule(&self, id: Uuid) -> Result<(), StoreError>;

// Scheduler operations
async fn claim_due_schedules(&self, limit: u32) -> Result<Vec<ScheduleRow>, StoreError>;
async fn update_next_trigger(&self, id: Uuid, next: DateTime<Utc>) -> Result<(), StoreError>;
async fn skip_schedule_trigger(&self, id: Uuid) -> Result<(), StoreError>;

// Execution operations
async fn create_schedule_execution(&self, schedule_id: Uuid, scheduled_at: DateTime<Utc>) -> Result<Uuid, StoreError>;
async fn complete_schedule_execution(&self, execution_id: Uuid, target_id: Uuid) -> Result<(), StoreError>;
async fn fail_schedule_execution(&self, execution_id: Uuid, error: &str) -> Result<(), StoreError>;
async fn list_schedule_executions(&self, schedule_id: Uuid, pagination: Pagination) -> Result<Vec<ExecutionRow>, StoreError>;
async fn count_running_executions(&self, schedule_id: Uuid) -> Result<u32, StoreError>;

// Stats
async fn get_schedule_stats(&self, schedule_id: Uuid) -> Result<ScheduleStats, StoreError>;
```

## Database Schema

### Migration

```sql
-- Create schedules table
-- Note: No org_id - multi-tenancy will be added to entire durable engine in a future PR
CREATE TABLE durable_schedules (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    name TEXT NOT NULL UNIQUE,
    description TEXT,
    cron_expression TEXT NOT NULL,
    timezone TEXT NOT NULL DEFAULT 'UTC',
    target_type TEXT NOT NULL CHECK (target_type IN ('workflow', 'activity')),
    target_name TEXT NOT NULL,
    target_input JSONB NOT NULL DEFAULT '{}',
    enabled BOOLEAN NOT NULL DEFAULT true,
    max_concurrent INTEGER,
    catch_up_missed BOOLEAN NOT NULL DEFAULT false,
    max_catch_up INTEGER DEFAULT 1,
    retry_policy JSONB,
    last_triggered_at TIMESTAMPTZ,
    next_trigger_at TIMESTAMPTZ,
    claimed_by TEXT,
    claimed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Index for scheduler polling
CREATE INDEX idx_durable_schedules_polling
ON durable_schedules (next_trigger_at)
WHERE enabled = true;

-- Create executions table
CREATE TABLE durable_schedule_executions (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    schedule_id UUID NOT NULL REFERENCES durable_schedules(id) ON DELETE CASCADE,
    scheduled_at TIMESTAMPTZ NOT NULL,
    started_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at TIMESTAMPTZ,
    status TEXT NOT NULL DEFAULT 'running' CHECK (status IN ('pending', 'running', 'completed', 'failed', 'skipped')),
    workflow_id UUID,
    task_id UUID,
    error TEXT,
    duration_ms INTEGER,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Index for listing executions by schedule
CREATE INDEX idx_durable_schedule_executions_schedule
ON durable_schedule_executions (schedule_id, created_at DESC);

-- Index for counting running executions
CREATE INDEX idx_durable_schedule_executions_running
ON durable_schedule_executions (schedule_id)
WHERE status = 'running';

-- Scheduler instance registration (for horizontal scaling)
CREATE TABLE durable_scheduler_instances (
    instance_id TEXT PRIMARY KEY,
    started_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_heartbeat_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    schedules_processed BIGINT NOT NULL DEFAULT 0,
    hostname TEXT,
    version TEXT
);
```

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

```typescript
// apps/ui/src/components/schedules/
├── schedule-list.tsx        // Table with schedules
├── schedule-card.tsx        // Summary card for dashboard
├── schedule-form.tsx        // Create/edit form
├── schedule-stats.tsx       // Stats display
├── execution-list.tsx       // Execution history table
├── cron-builder.tsx         // Cron expression builder
└── cron-preview.tsx         // Next N runs preview
```

### API Client

```typescript
// apps/ui/src/lib/api/schedules.ts

export interface Schedule {
  id: string;
  name: string;
  description?: string;
  cron_expression: string;
  timezone: string;
  target: ScheduleTarget;
  enabled: boolean;
  max_concurrent?: number;
  catch_up_missed: boolean;
  retry_policy?: RetryPolicy;
  last_triggered_at?: string;
  next_trigger_at?: string;
  stats: ScheduleStats;
  created_at: string;
  updated_at: string;
}

export interface ScheduleTarget {
  type: 'workflow' | 'activity';
  workflow_type?: string;
  activity_type?: string;
  input: Record<string, unknown>;
}

export interface ScheduleStats {
  total_executions: number;
  successful_executions: number;
  failed_executions: number;
  avg_duration_ms?: number;
  last_execution_status?: string;
}

export async function listSchedules(params?: ListSchedulesParams): Promise<ListSchedulesResponse>;
export async function getSchedule(id: string): Promise<Schedule>;
export async function createSchedule(data: CreateScheduleRequest): Promise<Schedule>;
export async function updateSchedule(id: string, data: UpdateScheduleRequest): Promise<Schedule>;
export async function deleteSchedule(id: string): Promise<void>;
export async function triggerSchedule(id: string): Promise<{ execution_id: string }>;
export async function pauseSchedule(id: string): Promise<Schedule>;
export async function resumeSchedule(id: string): Promise<Schedule>;
export async function listExecutions(scheduleId: string, params?: PaginationParams): Promise<ListExecutionsResponse>;
```

### Hooks

```typescript
// apps/ui/src/hooks/use-schedules.ts

export function useSchedules(params?: ListSchedulesParams) {
  return useQuery({
    queryKey: ['durable', 'schedules', params],
    queryFn: () => listSchedules(params),
  });
}

export function useSchedule(id: string) {
  return useQuery({
    queryKey: ['durable', 'schedules', id],
    queryFn: () => getSchedule(id),
  });
}

export function useCreateSchedule() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: createSchedule,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['durable', 'schedules'] });
    },
  });
}

export function useTriggerSchedule() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: triggerSchedule,
    onSuccess: (_, id) => {
      queryClient.invalidateQueries({ queryKey: ['durable', 'schedules', id] });
    },
  });
}

// SSE for real-time updates
export function useSchedulesSSE() {
  // Similar pattern to useDurableSSE
  // Updates query cache on schedule/execution changes
}
```

## SSE Integration

Extend existing durable SSE to include schedule events:

```rust
// New event types
#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DurableSSEEvent {
    // Existing events...

    // Schedule events
    ScheduleCreated { schedule: ScheduleResponse },
    ScheduleUpdated { schedule: ScheduleResponse },
    ScheduleDeleted { schedule_id: Uuid },
    ScheduleTriggered { schedule_id: Uuid, execution_id: Uuid },
    ExecutionCompleted { execution_id: Uuid, status: String, duration_ms: u64 },
}
```

## Testing

### Unit Tests

```rust
#[tokio::test]
async fn test_cron_parsing() {
    let expr = CronExpression::parse("*/30 * * * *").unwrap();
    let next = expr.next_after(Utc::now());
    // Assert next is within 30 minutes
}

#[tokio::test]
async fn test_schedule_creation() {
    let store = InMemoryWorkflowEventStore::new();
    let id = store.create_schedule(CreateScheduleRow {
        name: "test-schedule".into(),
        cron_expression: "*/5 * * * *".into(),
        // ...
    }).await.unwrap();

    let schedule = store.get_schedule(id).await.unwrap();
    assert_eq!(schedule.name, "test-schedule");
    assert!(schedule.next_trigger_at.is_some());
}

#[tokio::test]
async fn test_multi_instance_claiming() {
    let store = PostgresWorkflowEventStore::new(pool).await;

    // Create schedule due now
    let id = store.create_schedule(/* ... */).await.unwrap();

    // Claim from two instances concurrently
    let (claim1, claim2) = tokio::join!(
        store.claim_due_schedules(1),
        store.claim_due_schedules(1),
    );

    // Only one should get the schedule
    let total_claimed = claim1.unwrap().len() + claim2.unwrap().len();
    assert_eq!(total_claimed, 1);
}

#[tokio::test]
async fn test_max_concurrent_enforcement() {
    let store = InMemoryWorkflowEventStore::new();
    let scheduler = DurableScheduler::new(store.clone(), registry);

    // Create schedule with max_concurrent = 1
    let id = store.create_schedule(CreateScheduleRow {
        max_concurrent: Some(1),
        // ...
    }).await.unwrap();

    // Start an execution
    store.create_schedule_execution(id, Utc::now()).await.unwrap();

    // Try to trigger again - should skip
    scheduler.process_due_schedules().await.unwrap();

    let execs = store.list_schedule_executions(id, Pagination::default()).await.unwrap();
    assert_eq!(execs.len(), 1); // Only the first one
}
```

### Integration Tests

```rust
#[tokio::test]
async fn test_schedule_triggers_workflow() {
    let app = TestApp::new().await;

    // Create a schedule
    let resp = app.post("/v1/durable/schedules")
        .json(&json!({
            "name": "test-workflow-schedule",
            "cron_expression": "* * * * *", // Every minute
            "target": {
                "type": "workflow",
                "workflow_type": "test_workflow",
                "input": { "key": "value" }
            }
        }))
        .await;
    assert_eq!(resp.status(), 201);
    let schedule: ScheduleResponse = resp.json().await;

    // Wait for scheduler to trigger
    tokio::time::sleep(Duration::from_secs(65)).await;

    // Check execution was created
    let execs = app.get(&format!("/v1/durable/schedules/{}/executions", schedule.id))
        .await
        .json::<ListExecutionsResponse>()
        .await;
    assert!(!execs.data.is_empty());
    assert!(execs.data[0].workflow_id.is_some());
}
```

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

Add to `.claude/skills/smoke-test/`:

```bash
# smoke-test-schedules.sh

echo "=== Scheduled Tasks Smoke Test ==="

# 1. Create schedule
SCHEDULE=$(curl -s -X POST "$API_URL/v1/durable/schedules" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "smoke-test-schedule",
    "cron_expression": "* * * * *",
    "target": {
      "type": "workflow",
      "workflow_type": "echo",
      "input": {"message": "smoke test"}
    }
  }')
SCHEDULE_ID=$(echo $SCHEDULE | jq -r '.id')
echo "Created schedule: $SCHEDULE_ID"

# 2. Verify schedule in list
SCHEDULES=$(curl -s "$API_URL/v1/durable/schedules")
if ! echo $SCHEDULES | jq -e ".data[] | select(.id == \"$SCHEDULE_ID\")" > /dev/null; then
  echo "FAIL: Schedule not in list"
  exit 1
fi
echo "OK: Schedule in list"

# 3. Manual trigger
TRIGGER=$(curl -s -X POST "$API_URL/v1/durable/schedules/$SCHEDULE_ID/trigger")
EXEC_ID=$(echo $TRIGGER | jq -r '.execution_id')
echo "Triggered execution: $EXEC_ID"

# 4. Wait for execution to complete
for i in {1..30}; do
  EXEC=$(curl -s "$API_URL/v1/durable/executions/$EXEC_ID")
  STATUS=$(echo $EXEC | jq -r '.status')
  if [ "$STATUS" = "completed" ]; then
    echo "OK: Execution completed"
    break
  elif [ "$STATUS" = "failed" ]; then
    echo "FAIL: Execution failed"
    exit 1
  fi
  sleep 1
done

# 5. Pause schedule
curl -s -X POST "$API_URL/v1/durable/schedules/$SCHEDULE_ID/pause" > /dev/null
SCHEDULE=$(curl -s "$API_URL/v1/durable/schedules/$SCHEDULE_ID")
ENABLED=$(echo $SCHEDULE | jq -r '.enabled')
if [ "$ENABLED" != "false" ]; then
  echo "FAIL: Schedule not paused"
  exit 1
fi
echo "OK: Schedule paused"

# 6. Resume schedule
curl -s -X POST "$API_URL/v1/durable/schedules/$SCHEDULE_ID/resume" > /dev/null
SCHEDULE=$(curl -s "$API_URL/v1/durable/schedules/$SCHEDULE_ID")
ENABLED=$(echo $SCHEDULE | jq -r '.enabled')
if [ "$ENABLED" != "true" ]; then
  echo "FAIL: Schedule not resumed"
  exit 1
fi
echo "OK: Schedule resumed"

# 7. Delete schedule
curl -s -X DELETE "$API_URL/v1/durable/schedules/$SCHEDULE_ID" > /dev/null
SCHEDULE=$(curl -s "$API_URL/v1/durable/schedules/$SCHEDULE_ID")
if ! echo $SCHEDULE | jq -e '.error' > /dev/null; then
  echo "FAIL: Schedule not deleted"
  exit 1
fi
echo "OK: Schedule deleted"

echo "=== All smoke tests passed ==="
```

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

### Rust Crates

```toml
[dependencies]
cron = "0.13"           # Cron expression parsing
chrono-tz = "0.10"      # Timezone handling
```

### npm Packages

```json
{
  "dependencies": {
    "cronstrue": "^2.50.0",    // Human-readable cron descriptions
    "cron-parser": "^4.9.0"    // Parse cron for next-run preview
  }
}
```

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

```sql
CREATE TABLE durable_scheduler_instances (
    instance_id TEXT PRIMARY KEY,
    started_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_heartbeat_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    schedules_processed BIGINT NOT NULL DEFAULT 0,
    hostname TEXT,
    version TEXT
);
```

Stale instances (no heartbeat for 60s) are ignored when reclaiming schedules.

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

Prometheus metrics on `/metrics` endpoint - defined for future implementation:

```
# Counters
durable_schedule_triggers_total{status}
durable_schedule_executions_total{status}

# Gauges
durable_schedules_active
durable_schedules_pending_triggers
durable_scheduler_queue_depth

# Histograms
durable_schedule_trigger_latency_seconds
durable_schedule_execution_duration_seconds{activity_type}
```

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

### Example Test

```rust
#[tokio::test]
async fn test_scheduler_recovers_from_db_failure() {
    fail::cfg("postgres_claim_due_schedules_query", "1*return").unwrap();

    let scheduler = DurableScheduler::new(store.clone(), registry);

    // First attempt fails
    let result = scheduler.process_due_schedules().await;
    assert!(result.is_err());

    // Disable fail point
    fail::cfg("postgres_claim_due_schedules_query", "off").unwrap();

    // Second attempt succeeds
    let result = scheduler.process_due_schedules().await;
    assert!(result.is_ok());
}
```

## Benchmarks

Add to `crates/durable/benches/`:

| Benchmark | Target | Description |
|-----------|--------|-------------|
| `scheduler_throughput` | 1000 triggers/second | Sustained trigger rate |
| `scheduler_cold_start` | P50 < 2s | Time from due to trigger start |

### Benchmark Implementation

```rust
// benches/scheduler_throughput.rs
use criterion::{criterion_group, criterion_main, Criterion, Throughput};

fn scheduler_throughput_benchmark(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    let mut group = c.benchmark_group("scheduler");
    group.throughput(Throughput::Elements(1000));

    group.bench_function("trigger_1000_schedules", |b| {
        b.iter(|| {
            rt.block_on(async {
                // Create 1000 due schedules
                // Measure time to process all
            })
        })
    });

    group.finish();
}

criterion_group!(benches, scheduler_throughput_benchmark);
criterion_main!(benches);
```

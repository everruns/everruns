-- Durable Execution Engine Tables (v0.5.0)
-- Squashed migration - PostgreSQL schema for the durable execution engine
-- BREAKING CHANGE: Requires fresh database (drop existing _sqlx_migrations table)
--
-- Provides:
-- - Workflow instances with event sourcing
-- - Task queue with efficient claiming for 1000+ workers
-- - Dead letter queue for failed tasks
-- - Signal queue for external workflow communication
-- - Worker registry for monitoring
-- - Circuit breaker state for reliability
-- - Push-based task notifications via pg_notify
-- - Cron-based scheduled task execution

-- ============================================
-- V001: Workflow Instances
-- ============================================
-- Stores the state of each workflow instance

CREATE TABLE durable_workflow_instances (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    workflow_type TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',  -- pending, running, completed, failed, cancelled
    input JSONB NOT NULL,
    result JSONB,
    error JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    started_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,

    -- FUTURE: Partition key for horizontal sharding
    -- Currently unused - reserved for multi-node deployment scaling
    partition_key INT NOT NULL DEFAULT 0,

    -- Tracing context
    trace_id TEXT,
    span_id TEXT
);

CREATE INDEX idx_durable_workflow_instances_status ON durable_workflow_instances(status);
CREATE INDEX idx_durable_workflow_instances_type ON durable_workflow_instances(workflow_type);
CREATE INDEX idx_durable_workflow_instances_created ON durable_workflow_instances(created_at);

-- ============================================
-- V002: Workflow Events (append-only log)
-- ============================================
-- Event-sourced log of all workflow state changes.
-- Enables replay for recovery and debugging.

CREATE TABLE durable_workflow_events (
    id BIGSERIAL PRIMARY KEY,
    workflow_id UUID NOT NULL REFERENCES durable_workflow_instances(id) ON DELETE CASCADE,
    sequence_num INT NOT NULL,  -- Per-workflow sequence number (0-indexed)
    event_type TEXT NOT NULL,
    event_data JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- Tracing context for this event
    trace_id TEXT,
    span_id TEXT,

    UNIQUE(workflow_id, sequence_num)
);

CREATE INDEX idx_durable_workflow_events_workflow ON durable_workflow_events(workflow_id, sequence_num);

-- ============================================
-- V003: Task Queue (for activity scheduling)
-- ============================================
-- Distributed task queue with efficient claiming using SKIP LOCKED.
-- Optimized for 1000+ concurrent workers polling.

CREATE TABLE durable_task_queue (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    workflow_id UUID NOT NULL REFERENCES durable_workflow_instances(id) ON DELETE CASCADE,
    activity_id TEXT NOT NULL,
    activity_type TEXT NOT NULL,
    input JSONB NOT NULL,
    options JSONB NOT NULL,

    -- Scheduling
    status TEXT NOT NULL DEFAULT 'pending',  -- pending, claimed, completed, failed, dead, cancelled
    priority INT NOT NULL DEFAULT 0,
    scheduled_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    visible_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),  -- For delayed retry

    -- Claiming (partitioned by activity_type for better distribution)
    claimed_by TEXT,  -- Worker ID
    claimed_at TIMESTAMPTZ,
    heartbeat_at TIMESTAMPTZ,

    -- Execution tracking
    attempt INT NOT NULL DEFAULT 0,
    max_attempts INT NOT NULL,
    last_error TEXT,

    -- Timeouts (stored as intervals)
    schedule_to_start_timeout_ms BIGINT NOT NULL,
    start_to_close_timeout_ms BIGINT NOT NULL,
    heartbeat_timeout_ms BIGINT,

    -- Tracing
    trace_id TEXT,
    span_id TEXT,

    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Efficient polling query - CRITICAL for 1000 workers
-- Leads with ORDER BY columns for efficient LIMIT + FOR UPDATE SKIP LOCKED
-- The claim_task query uses:
--   WHERE status = 'pending' AND activity_type = ANY($1) AND visible_at <= NOW()
--   ORDER BY priority DESC, visible_at
--   LIMIT $2
--   FOR UPDATE SKIP LOCKED
CREATE INDEX idx_durable_task_queue_pending
    ON durable_task_queue(priority DESC, visible_at, activity_type)
    WHERE status = 'pending';

-- For heartbeat monitoring (worker lookup)
CREATE INDEX idx_durable_task_queue_claimed
    ON durable_task_queue(claimed_by, heartbeat_at)
    WHERE status = 'claimed';

-- For stale task reclaiming (heartbeat_at range scan)
CREATE INDEX idx_durable_task_queue_stale_reclaim
    ON durable_task_queue(heartbeat_at)
    WHERE status = 'claimed';

-- For workflow-level queries
CREATE INDEX idx_durable_task_queue_workflow ON durable_task_queue(workflow_id);

-- ============================================
-- V004: Dead Letter Queue
-- ============================================
-- Tasks that have exhausted retries or failed permanently.

CREATE TABLE durable_dead_letter_queue (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    original_task_id UUID NOT NULL,
    workflow_id UUID NOT NULL REFERENCES durable_workflow_instances(id) ON DELETE CASCADE,
    activity_id TEXT NOT NULL,
    activity_type TEXT NOT NULL,
    input JSONB NOT NULL,

    -- Failure info
    attempts INT NOT NULL,
    last_error TEXT NOT NULL,
    error_history JSONB NOT NULL,  -- Array of all errors

    -- Metadata
    dead_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    requeued_at TIMESTAMPTZ,
    requeue_count INT NOT NULL DEFAULT 0
);

CREATE INDEX idx_durable_dlq_workflow ON durable_dead_letter_queue(workflow_id);
CREATE INDEX idx_durable_dlq_activity_type ON durable_dead_letter_queue(activity_type);
CREATE INDEX idx_durable_dlq_dead_at ON durable_dead_letter_queue(dead_at);

-- ============================================
-- V005: Circuit Breaker State (shared) - FUTURE FEATURE
-- ============================================
-- Distributed circuit breaker state for external service protection.
-- FUTURE: Not yet integrated. See everruns-durable/reliability/distributed_circuit_breaker.rs
-- TODO: Integrate with LLM provider calls and external tool executions.

CREATE TABLE durable_circuit_breaker_state (
    key TEXT PRIMARY KEY,  -- e.g., "activity:llm_call" or "external:openai"
    state TEXT NOT NULL DEFAULT 'closed',  -- closed, open, half_open
    failure_count INT NOT NULL DEFAULT 0,
    success_count INT NOT NULL DEFAULT 0,
    last_failure_at TIMESTAMPTZ,
    opened_at TIMESTAMPTZ,
    half_open_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- ============================================
-- V006: Worker Registry
-- ============================================
-- Tracks active workers for monitoring and coordination.

CREATE TABLE durable_workers (
    id TEXT PRIMARY KEY,  -- Worker ID (e.g., hostname-pid-uuid)
    worker_group TEXT NOT NULL,  -- Logical grouping (e.g., "default", "high-priority")
    activity_types TEXT[] NOT NULL,  -- Types this worker handles

    -- Capacity and load
    max_concurrency INT NOT NULL,
    current_load INT NOT NULL DEFAULT 0,

    -- Status
    status TEXT NOT NULL DEFAULT 'active',  -- active, draining, stopped
    started_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_heartbeat_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- Backpressure signaling
    accepting_tasks BOOLEAN NOT NULL DEFAULT true,
    backpressure_reason TEXT,

    -- Metadata
    hostname TEXT,
    version TEXT,
    metadata JSONB
);

CREATE INDEX idx_durable_workers_status ON durable_workers(status) WHERE status = 'active';
CREATE INDEX idx_durable_workers_heartbeat ON durable_workers(last_heartbeat_at);
CREATE INDEX idx_durable_workers_group ON durable_workers(worker_group);

-- ============================================
-- V007: Signals Queue - INTERNAL USE
-- ============================================
-- Signals for workflow communication (used internally by WorkflowExecutor).
-- Currently not exposed via gRPC - used for internal workflow state transitions.

CREATE TABLE durable_signals (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    workflow_id UUID NOT NULL REFERENCES durable_workflow_instances(id) ON DELETE CASCADE,
    signal_type TEXT NOT NULL,
    payload JSONB NOT NULL,
    sent_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    processed_at TIMESTAMPTZ,

    -- Ordering within workflow
    sequence_num SERIAL
);

CREATE INDEX idx_durable_signals_pending
    ON durable_signals(workflow_id, sequence_num)
    WHERE processed_at IS NULL;

-- ============================================
-- Trigger for updated_at
-- ============================================

CREATE OR REPLACE FUNCTION update_durable_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trigger_durable_workflow_instances_updated_at
    BEFORE UPDATE ON durable_workflow_instances
    FOR EACH ROW
    EXECUTE FUNCTION update_durable_updated_at();

-- ============================================
-- Push-based Task Notifications (pg_notify)
-- ============================================
-- Enables low-latency task pickup (<10ms) by notifying workers when tasks are enqueued

-- Create notification function for task enqueue
CREATE OR REPLACE FUNCTION notify_task_available()
RETURNS TRIGGER AS $$
BEGIN
    -- Notify with activity_type as payload for filtering
    PERFORM pg_notify('task_available', NEW.activity_type);
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- Trigger on task insert (new tasks) and update to pending (retries)
CREATE TRIGGER task_enqueue_notify
    AFTER INSERT ON durable_task_queue
    FOR EACH ROW
    WHEN (NEW.status = 'pending')
    EXECUTE FUNCTION notify_task_available();

-- Also notify when tasks are set back to pending (retries, reclaims)
CREATE TRIGGER task_pending_notify
    AFTER UPDATE ON durable_task_queue
    FOR EACH ROW
    WHEN (OLD.status != 'pending' AND NEW.status = 'pending')
    EXECUTE FUNCTION notify_task_available();

-- ============================================
-- V008: Schedule Definitions
-- ============================================
-- Cron-based schedule configurations.
-- Design: No org_id yet - multi-tenancy deferred until entire durable engine supports it.
-- SKIP LOCKED for multi-instance scheduler coordination.

CREATE TABLE durable_schedules (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    name TEXT NOT NULL UNIQUE,
    description TEXT,

    -- Cron configuration
    cron_expression TEXT NOT NULL,  -- 7-field cron (sec min hour day month day-of-week year)
    timezone TEXT NOT NULL DEFAULT 'UTC',  -- IANA timezone

    -- Target configuration
    target_type TEXT NOT NULL CHECK (target_type IN ('workflow', 'activity')),
    target_name TEXT NOT NULL,  -- Workflow type or activity type
    target_input JSONB NOT NULL DEFAULT '{}',

    -- Execution control
    enabled BOOLEAN NOT NULL DEFAULT true,
    max_concurrent INTEGER,  -- NULL = unlimited
    catch_up_missed BOOLEAN NOT NULL DEFAULT false,
    max_catch_up INTEGER DEFAULT 1,
    retry_policy JSONB,  -- RetryPolicy configuration

    -- Scheduler coordination
    last_triggered_at TIMESTAMPTZ,
    next_trigger_at TIMESTAMPTZ,
    claimed_by TEXT,  -- Scheduler instance ID
    claimed_at TIMESTAMPTZ,

    -- Timestamps
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Index for scheduler polling
CREATE INDEX idx_durable_schedules_polling
    ON durable_schedules (next_trigger_at)
    WHERE enabled = true;

-- Index for enabled schedules by next trigger time
CREATE INDEX idx_durable_schedules_next_trigger
    ON durable_schedules (next_trigger_at)
    WHERE enabled = true;

-- ============================================
-- V009: Schedule Executions
-- ============================================
-- Tracks each execution of a scheduled task

CREATE TABLE durable_schedule_executions (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    schedule_id UUID NOT NULL REFERENCES durable_schedules(id) ON DELETE CASCADE,

    -- Timing
    scheduled_at TIMESTAMPTZ NOT NULL,  -- When it was supposed to run
    started_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at TIMESTAMPTZ,

    -- Status
    status TEXT NOT NULL DEFAULT 'running' CHECK (status IN ('pending', 'running', 'completed', 'failed', 'skipped')),

    -- Target reference (one of these will be populated)
    workflow_id UUID,  -- If target_type = 'workflow'
    task_id UUID,  -- If target_type = 'activity'

    -- Error tracking
    error TEXT,

    -- Metrics
    duration_ms INTEGER,

    -- Timestamps
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Index for listing executions by schedule
CREATE INDEX idx_durable_schedule_executions_schedule
    ON durable_schedule_executions (schedule_id, created_at DESC);

-- Index for counting running executions (max_concurrent enforcement)
CREATE INDEX idx_durable_schedule_executions_running
    ON durable_schedule_executions (schedule_id)
    WHERE status = 'running';

-- ============================================
-- V010: Scheduler Instance Registration
-- ============================================
-- Tracks active scheduler instances for monitoring and debugging.
-- Not required for coordination (handled by SKIP LOCKED).

CREATE TABLE durable_scheduler_instances (
    instance_id TEXT PRIMARY KEY,  -- e.g., hostname-pid-uuid
    started_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_heartbeat_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    schedules_processed BIGINT NOT NULL DEFAULT 0,

    -- Metadata for debugging
    hostname TEXT,
    version TEXT
);

-- Index for finding stale instances
CREATE INDEX idx_durable_scheduler_instances_heartbeat
    ON durable_scheduler_instances(last_heartbeat_at);

-- ============================================
-- Trigger for schedules updated_at
-- ============================================

CREATE TRIGGER trigger_durable_schedules_updated_at
    BEFORE UPDATE ON durable_schedules
    FOR EACH ROW
    EXECUTE FUNCTION update_durable_updated_at();

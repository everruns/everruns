-- Durable Execution Engine Tables
--
-- PostgreSQL schema for the durable execution engine providing:
-- - Workflow instances with event sourcing
-- - Task queue with efficient claiming for 1000+ workers
-- - Dead letter queue for failed tasks
-- - Signal queue for external workflow communication
-- - Worker registry for monitoring
-- - Circuit breaker state for reliability

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

    -- Partition key for future sharding
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
-- Partial index on pending status + ordering by priority/visibility
CREATE INDEX idx_durable_task_queue_pending
    ON durable_task_queue(activity_type, priority DESC, visible_at)
    WHERE status = 'pending';

-- For heartbeat monitoring
CREATE INDEX idx_durable_task_queue_claimed
    ON durable_task_queue(claimed_by, heartbeat_at)
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
-- V005: Circuit Breaker State (shared)
-- ============================================
-- Distributed circuit breaker state for external service protection.

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
-- V007: Signals Queue
-- ============================================
-- External signals sent to running workflows.

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

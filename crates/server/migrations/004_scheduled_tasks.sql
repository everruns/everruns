-- Scheduled Tasks Tables (v0.5.0)
-- PostgreSQL schema for cron-based scheduled task execution
--
-- Provides:
-- - Schedule definitions with cron expressions
-- - Execution history tracking
-- - Rate limiting per organization
-- - Scheduler instance registration for horizontal scaling
-- - Multi-tenant isolation via org_id
--
-- Design decisions:
-- - org_id on all tables for multi-tenant isolation
-- - Round-robin fair scheduling across organizations
-- - Database-backed rate limits for horizontal scalability
-- - SKIP LOCKED for multi-instance scheduler coordination

-- ============================================
-- V001: Schedule Definitions
-- ============================================
-- Stores cron-based schedule configurations

CREATE TABLE durable_schedules (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    org_id BIGINT NOT NULL REFERENCES organizations(org_id),
    name TEXT NOT NULL,
    description TEXT,

    -- Cron configuration
    cron_expression TEXT NOT NULL,  -- Standard 5 or 6 field cron
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
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- Unique name within organization
    CONSTRAINT uq_durable_schedules_org_name UNIQUE (org_id, name)
);

-- Index for organization queries
CREATE INDEX idx_durable_schedules_org ON durable_schedules(org_id);

-- Index for scheduler polling with fair distribution across orgs
-- Query: SELECT DISTINCT ON (org_id) ... WHERE enabled AND next_trigger_at <= NOW()
CREATE INDEX idx_durable_schedules_polling
    ON durable_schedules (org_id, next_trigger_at)
    WHERE enabled = true;

-- Index for enabled schedules by next trigger time
CREATE INDEX idx_durable_schedules_next_trigger
    ON durable_schedules (next_trigger_at)
    WHERE enabled = true;

-- ============================================
-- V002: Schedule Executions
-- ============================================
-- Tracks each execution of a scheduled task

CREATE TABLE durable_schedule_executions (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    org_id BIGINT NOT NULL REFERENCES organizations(org_id),
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

-- Index for organization queries
CREATE INDEX idx_durable_schedule_executions_org
    ON durable_schedule_executions(org_id);

-- Index for listing executions by schedule (within org)
CREATE INDEX idx_durable_schedule_executions_schedule
    ON durable_schedule_executions (org_id, schedule_id, created_at DESC);

-- Index for counting running executions (max_concurrent enforcement)
CREATE INDEX idx_durable_schedule_executions_running
    ON durable_schedule_executions (schedule_id)
    WHERE status = 'running';

-- ============================================
-- V003: Rate Limit Tracking
-- ============================================
-- Tracks execution counts per organization for rate limiting
-- Uses hourly windows with atomic increment via upsert

CREATE TABLE durable_schedule_rate_limits (
    org_id BIGINT NOT NULL REFERENCES organizations(org_id),
    window_start TIMESTAMPTZ NOT NULL,  -- Truncated to hour
    execution_count INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (org_id, window_start)
);

-- Index for cleanup of old windows
CREATE INDEX idx_durable_schedule_rate_limits_window
    ON durable_schedule_rate_limits(window_start);

-- ============================================
-- V004: Scheduler Instance Registration
-- ============================================
-- Tracks active scheduler instances for monitoring and debugging
-- Not required for coordination (handled by SKIP LOCKED)

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
-- Trigger for updated_at
-- ============================================

CREATE TRIGGER trigger_durable_schedules_updated_at
    BEFORE UPDATE ON durable_schedules
    FOR EACH ROW
    EXECUTE FUNCTION update_durable_updated_at();

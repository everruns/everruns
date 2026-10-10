-- everruns-durable PostgreSQL schema.
--
-- Applied by `PostgresWorkflowEventStore::migrate`, which runs this whole file in
-- one transaction under a transaction-scoped advisory lock. Every statement is
-- idempotent, so it is safe to run on every start-up, concurrently from several
-- processes, and against a database whose durable tables were already created
-- by the Everruns server migrations (crates/server/migrations), in which case it
-- changes nothing.
--
-- Objects are created unqualified, in the first schema of the connection's
-- `search_path`. Only built-in PostgreSQL functions are used (no extensions).
--
-- This file mirrors the final state the server migrations produce for the
-- durable tables. `tests/schema_drift_test.rs` fails when the two diverge, so a
-- server migration that changes a durable table must change this file too. When
-- evolving it, keep every statement idempotent: add columns with
-- `ALTER TABLE ... ADD COLUMN IF NOT EXISTS` and indexes with
-- `CREATE INDEX IF NOT EXISTS`, appended after the CREATE TABLE statements, so
-- databases created by an older version of this file are brought forward.
--
-- `durable_tool_results` is not part of this schema: it belongs to the server's
-- tool-call idempotency storage, and nothing in this crate reads or writes it.

-- UUIDv7 primary-key default. PostgreSQL 18+ ships `uuidv7()`; on older servers,
-- and when no visible schema already provides one, define a fallback built from
-- `gen_random_uuid()` (built in since PostgreSQL 13).
DO $$
BEGIN
    IF to_regprocedure('uuidv7()') IS NULL THEN
        CREATE FUNCTION uuidv7() RETURNS uuid AS $fn$
            SELECT encode(
                set_bit(
                    set_bit(
                        overlay(
                            uuid_send(gen_random_uuid())
                            PLACING substring(
                                int8send(floor(extract(epoch FROM clock_timestamp()) * 1000)::bigint)
                                FROM 3
                            )
                            FROM 1 FOR 6
                        ),
                        52, 1
                    ),
                    53, 1
                ),
                'hex'
            )::uuid;
        $fn$ LANGUAGE sql VOLATILE;
    END IF;
END
$$;

-- ============================================
-- Workflows and their event log
-- ============================================

CREATE TABLE IF NOT EXISTS durable_workflow_instances (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    workflow_type TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    input JSONB NOT NULL,
    result JSONB,
    error JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    started_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    partition_key INTEGER NOT NULL DEFAULT 0,
    trace_id TEXT,
    span_id TEXT,
    continued_as_new_id UUID REFERENCES durable_workflow_instances(id)
);

CREATE INDEX IF NOT EXISTS idx_durable_workflow_instances_status
    ON durable_workflow_instances (status);
CREATE INDEX IF NOT EXISTS idx_durable_workflow_instances_type
    ON durable_workflow_instances (workflow_type);
CREATE INDEX IF NOT EXISTS idx_durable_workflow_instances_created
    ON durable_workflow_instances (created_at);
CREATE INDEX IF NOT EXISTS idx_durable_workflow_instances_continued_as_new_id
    ON durable_workflow_instances (continued_as_new_id)
    WHERE continued_as_new_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS durable_workflow_events (
    id BIGSERIAL PRIMARY KEY,
    workflow_id UUID NOT NULL REFERENCES durable_workflow_instances(id) ON DELETE CASCADE,
    sequence_num INTEGER NOT NULL,
    event_type TEXT NOT NULL,
    event_data JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    trace_id TEXT,
    span_id TEXT,
    UNIQUE (workflow_id, sequence_num)
);

CREATE INDEX IF NOT EXISTS idx_durable_workflow_events_workflow
    ON durable_workflow_events (workflow_id, sequence_num);

CREATE TABLE IF NOT EXISTS durable_workflow_snapshots (
    id BIGSERIAL PRIMARY KEY,
    workflow_id UUID NOT NULL REFERENCES durable_workflow_instances(id) ON DELETE CASCADE,
    sequence_num INTEGER NOT NULL,
    snapshot_data BYTEA NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (workflow_id, sequence_num)
);

CREATE INDEX IF NOT EXISTS idx_durable_workflow_snapshots_latest
    ON durable_workflow_snapshots (workflow_id, sequence_num DESC);

CREATE TABLE IF NOT EXISTS durable_signals (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    workflow_id UUID NOT NULL REFERENCES durable_workflow_instances(id) ON DELETE CASCADE,
    signal_type TEXT NOT NULL,
    payload JSONB NOT NULL,
    sent_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    processed_at TIMESTAMPTZ,
    sequence_num SERIAL
);

CREATE INDEX IF NOT EXISTS idx_durable_signals_pending
    ON durable_signals (workflow_id, sequence_num)
    WHERE processed_at IS NULL;

-- ============================================
-- Task queue and dead letters
-- ============================================

CREATE TABLE IF NOT EXISTS durable_task_queue (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    workflow_id UUID REFERENCES durable_workflow_instances(id) ON DELETE CASCADE,
    activity_id TEXT NOT NULL,
    activity_type TEXT NOT NULL,
    input JSONB NOT NULL,
    options JSONB NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    priority INTEGER NOT NULL DEFAULT 0,
    scheduled_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    visible_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    claimed_by TEXT,
    claimed_at TIMESTAMPTZ,
    heartbeat_at TIMESTAMPTZ,
    attempt INTEGER NOT NULL DEFAULT 0,
    max_attempts INTEGER NOT NULL,
    last_error TEXT,
    schedule_to_start_timeout_ms BIGINT NOT NULL,
    start_to_close_timeout_ms BIGINT NOT NULL,
    heartbeat_timeout_ms BIGINT,
    trace_id TEXT,
    span_id TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    progress_token BIGINT,
    no_progress_count INTEGER NOT NULL DEFAULT 0
);
-- Named task queue (`ActivityOptions::queue`); NULL is the default queue.
-- Server migration 178.
ALTER TABLE durable_task_queue ADD COLUMN IF NOT EXISTS queue TEXT;

CREATE INDEX IF NOT EXISTS idx_durable_task_queue_pending
    ON durable_task_queue (priority DESC, visible_at, activity_type)
    WHERE status = 'pending';
CREATE INDEX IF NOT EXISTS idx_durable_task_queue_claimed
    ON durable_task_queue (claimed_by, heartbeat_at)
    WHERE status = 'claimed';
CREATE INDEX IF NOT EXISTS idx_durable_task_queue_stale_reclaim
    ON durable_task_queue (heartbeat_at)
    WHERE status = 'claimed';
CREATE INDEX IF NOT EXISTS idx_durable_task_queue_standalone
    ON durable_task_queue (created_at DESC)
    WHERE workflow_id IS NULL;
CREATE INDEX IF NOT EXISTS idx_durable_task_queue_workflow
    ON durable_task_queue (workflow_id);
-- Task listings order by created_at; worker listings aggregate by claimed_by.
-- Server migrations 208 and 209.
CREATE INDEX IF NOT EXISTS idx_durable_task_queue_created_at
    ON durable_task_queue (created_at);
CREATE INDEX IF NOT EXISTS idx_durable_task_queue_claimed_by
    ON durable_task_queue (claimed_by)
    WHERE claimed_by IS NOT NULL;
-- Backs `ActivityOptions::dedupe_by_activity_id` for the activity ids Everruns
-- enqueues idempotently (server migration 143). The engine's dedupe SQL names no
-- arbiter, so it uses whatever unique index covers a caller's ids; this one is
-- kept for parity with the server migrations (see schema_drift_test).
CREATE UNIQUE INDEX IF NOT EXISTS idx_durable_task_queue_waiting_turn_resolution
    ON durable_task_queue (workflow_id, activity_id)
    WHERE workflow_id IS NOT NULL AND activity_id LIKE 'waiting_turn_resolution_%';

CREATE TABLE IF NOT EXISTS durable_dead_letter_queue (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    original_task_id UUID NOT NULL,
    workflow_id UUID REFERENCES durable_workflow_instances(id) ON DELETE CASCADE,
    activity_id TEXT NOT NULL,
    activity_type TEXT NOT NULL,
    input JSONB NOT NULL,
    attempts INTEGER NOT NULL,
    last_error TEXT NOT NULL,
    error_history JSONB NOT NULL,
    dead_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    requeued_at TIMESTAMPTZ,
    requeue_count INTEGER NOT NULL DEFAULT 0
);

-- The dead task's activity options (server migration 181), so a requeue keeps
-- its queue and retry settings. NULL for rows dead before it.
ALTER TABLE durable_dead_letter_queue ADD COLUMN IF NOT EXISTS options JSONB;

CREATE INDEX IF NOT EXISTS idx_durable_dlq_activity_type
    ON durable_dead_letter_queue (activity_type);
CREATE INDEX IF NOT EXISTS idx_durable_dlq_dead_at
    ON durable_dead_letter_queue (dead_at);
CREATE INDEX IF NOT EXISTS idx_durable_dlq_workflow
    ON durable_dead_letter_queue (workflow_id);

-- ============================================
-- Workers and circuit breakers
-- ============================================

CREATE TABLE IF NOT EXISTS durable_workers (
    id TEXT PRIMARY KEY,
    worker_group TEXT NOT NULL,
    activity_types TEXT[] NOT NULL,
    max_concurrency INTEGER NOT NULL,
    current_load INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'active',
    started_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_heartbeat_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    accepting_tasks BOOLEAN NOT NULL DEFAULT TRUE,
    backpressure_reason TEXT,
    hostname TEXT,
    version TEXT,
    metadata JSONB
);

CREATE INDEX IF NOT EXISTS idx_durable_workers_group
    ON durable_workers (worker_group);
CREATE INDEX IF NOT EXISTS idx_durable_workers_heartbeat
    ON durable_workers (last_heartbeat_at);
CREATE INDEX IF NOT EXISTS idx_durable_workers_status
    ON durable_workers (status)
    WHERE status = 'active';

CREATE TABLE IF NOT EXISTS durable_circuit_breaker_state (
    key TEXT PRIMARY KEY,
    state TEXT NOT NULL DEFAULT 'closed',
    failure_count INTEGER NOT NULL DEFAULT 0,
    success_count INTEGER NOT NULL DEFAULT 0,
    last_failure_at TIMESTAMPTZ,
    opened_at TIMESTAMPTZ,
    half_open_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- ============================================
-- Schedules
-- ============================================

CREATE TABLE IF NOT EXISTS durable_schedules (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    name TEXT NOT NULL UNIQUE,
    description TEXT,
    cron_expression TEXT NOT NULL,
    timezone TEXT NOT NULL DEFAULT 'UTC',
    target_type TEXT NOT NULL CHECK (target_type IN ('workflow', 'activity')),
    target_name TEXT NOT NULL,
    target_input JSONB NOT NULL DEFAULT '{}',
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    max_concurrent INTEGER,
    catch_up_missed BOOLEAN NOT NULL DEFAULT FALSE,
    max_catch_up INTEGER DEFAULT 1,
    retry_policy JSONB,
    last_triggered_at TIMESTAMPTZ,
    next_trigger_at TIMESTAMPTZ,
    claimed_by TEXT,
    claimed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_durable_schedules_polling
    ON durable_schedules (next_trigger_at)
    INCLUDE (claimed_by, claimed_at)
    WHERE enabled = TRUE;

CREATE TABLE IF NOT EXISTS durable_schedule_executions (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    schedule_id UUID NOT NULL REFERENCES durable_schedules(id) ON DELETE CASCADE,
    scheduled_at TIMESTAMPTZ NOT NULL,
    started_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at TIMESTAMPTZ,
    status TEXT NOT NULL DEFAULT 'running'
        CHECK (status IN ('pending', 'running', 'completed', 'failed', 'skipped')),
    workflow_id UUID,
    task_id UUID,
    error TEXT,
    duration_ms INTEGER,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_durable_schedule_executions_schedule
    ON durable_schedule_executions (schedule_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_durable_schedule_executions_running
    ON durable_schedule_executions (schedule_id)
    WHERE status = 'running';

CREATE TABLE IF NOT EXISTS durable_scheduler_instances (
    instance_id TEXT PRIMARY KEY,
    started_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_heartbeat_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    schedules_processed BIGINT NOT NULL DEFAULT 0,
    hostname TEXT,
    version TEXT
);

CREATE INDEX IF NOT EXISTS idx_durable_scheduler_instances_heartbeat
    ON durable_scheduler_instances (last_heartbeat_at);

-- ============================================
-- Triggers
-- ============================================

CREATE OR REPLACE FUNCTION update_durable_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE OR REPLACE TRIGGER trigger_durable_workflow_instances_updated_at
    BEFORE UPDATE ON durable_workflow_instances
    FOR EACH ROW EXECUTE FUNCTION update_durable_updated_at();

CREATE OR REPLACE TRIGGER trigger_durable_schedules_updated_at
    BEFORE UPDATE ON durable_schedules
    FOR EACH ROW EXECUTE FUNCTION update_durable_updated_at();

-- Wake idle workers (LISTEN task_available) when a task becomes claimable.
CREATE OR REPLACE FUNCTION notify_task_available()
RETURNS TRIGGER AS $$
BEGIN
    -- Notify with activity_type as payload for filtering. A named queue's
    -- task wakes no default-queue listener: none of them could claim it.
    IF NEW.queue IS NULL THEN
        PERFORM pg_notify('task_available', NEW.activity_type);
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE OR REPLACE TRIGGER task_enqueue_notify
    AFTER INSERT ON durable_task_queue
    FOR EACH ROW
    WHEN (NEW.status = 'pending')
    EXECUTE FUNCTION notify_task_available();

CREATE OR REPLACE TRIGGER task_pending_notify
    AFTER UPDATE ON durable_task_queue
    FOR EACH ROW
    WHEN (OLD.status <> 'pending' AND NEW.status = 'pending')
    EXECUTE FUNCTION notify_task_available();

-- ============================================
-- Health counters
-- ============================================
-- Cumulative task/workflow totals read by `DurableAdmin::get_system_health`.
-- Statement-level AFTER triggers apply one aggregated delta per statement, so
-- the sum of a counter's rows always equals the COUNT(*) it replaces and reads
-- stay O(1). Each counter has 16 shard rows and a trigger writes the shard
-- picked by `pg_backend_pid() % 16`: one row per counter would be locked by
-- every claim and hand-off until commit, serializing them all. A single shard
-- can go negative, so only the sum is meaningful.

CREATE TABLE IF NOT EXISTS durable_stat_counters (
    name TEXT NOT NULL,
    value BIGINT NOT NULL DEFAULT 0,
    shard SMALLINT NOT NULL DEFAULT 0,
    PRIMARY KEY (name, shard)
);

-- Databases that took the unsharded table: same steps as server migration 182.
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_schema = current_schema()
          AND table_name = 'durable_stat_counters'
          AND column_name = 'shard'
    ) THEN
        ALTER TABLE durable_stat_counters DROP CONSTRAINT durable_stat_counters_pkey;
        ALTER TABLE durable_stat_counters DROP CONSTRAINT IF EXISTS durable_stat_counters_value_check;
        ALTER TABLE durable_stat_counters ADD COLUMN shard SMALLINT NOT NULL DEFAULT 0;
        ALTER TABLE durable_stat_counters ADD PRIMARY KEY (name, shard);
    END IF;
END
$$;

CREATE OR REPLACE FUNCTION durable_task_stat_counters()
RETURNS TRIGGER AS $$
DECLARE
    d_completed BIGINT := 0;
    d_failed BIGINT := 0;
    d_started BIGINT := 0;
    my_shard SMALLINT := pg_backend_pid() % 16;
BEGIN
    IF TG_OP IN ('INSERT', 'UPDATE') THEN
        SELECT
            count(*) FILTER (WHERE status = 'completed'),
            count(*) FILTER (WHERE status IN ('failed', 'dead')),
            count(*) FILTER (WHERE claimed_at IS NOT NULL)
        INTO d_completed, d_failed, d_started
        FROM new_rows;
    END IF;

    IF TG_OP IN ('UPDATE', 'DELETE') THEN
        d_completed := d_completed - (SELECT count(*) FROM old_rows WHERE status = 'completed');
        d_failed := d_failed - (SELECT count(*) FROM old_rows WHERE status IN ('failed', 'dead'));
        d_started := d_started - (SELECT count(*) FROM old_rows WHERE claimed_at IS NOT NULL);
    END IF;

    IF d_completed <> 0 THEN
        UPDATE durable_stat_counters SET value = value + d_completed WHERE name = 'tasks_completed' AND shard = my_shard;
    END IF;
    IF d_failed <> 0 THEN
        UPDATE durable_stat_counters SET value = value + d_failed WHERE name = 'tasks_failed' AND shard = my_shard;
    END IF;
    IF d_started <> 0 THEN
        UPDATE durable_stat_counters SET value = value + d_started WHERE name = 'tasks_started' AND shard = my_shard;
    END IF;

    RETURN NULL; -- AFTER trigger: return value is ignored
END;
$$ LANGUAGE plpgsql;

CREATE OR REPLACE FUNCTION durable_workflow_stat_counters()
RETURNS TRIGGER AS $$
DECLARE
    d_completed BIGINT := 0;
    d_failed BIGINT := 0;
    d_started BIGINT := 0;
    my_shard SMALLINT := pg_backend_pid() % 16;
BEGIN
    IF TG_OP IN ('INSERT', 'UPDATE') THEN
        SELECT
            count(*) FILTER (WHERE status = 'completed'),
            count(*) FILTER (WHERE status IN ('failed', 'cancelled')),
            count(*) FILTER (WHERE started_at IS NOT NULL)
        INTO d_completed, d_failed, d_started
        FROM new_rows;
    END IF;

    IF TG_OP IN ('UPDATE', 'DELETE') THEN
        d_completed := d_completed - (SELECT count(*) FROM old_rows WHERE status = 'completed');
        d_failed := d_failed - (SELECT count(*) FROM old_rows WHERE status IN ('failed', 'cancelled'));
        d_started := d_started - (SELECT count(*) FROM old_rows WHERE started_at IS NOT NULL);
    END IF;

    IF d_completed <> 0 THEN
        UPDATE durable_stat_counters SET value = value + d_completed WHERE name = 'workflows_completed' AND shard = my_shard;
    END IF;
    IF d_failed <> 0 THEN
        UPDATE durable_stat_counters SET value = value + d_failed WHERE name = 'workflows_failed' AND shard = my_shard;
    END IF;
    IF d_started <> 0 THEN
        UPDATE durable_stat_counters SET value = value + d_started WHERE name = 'workflows_started' AND shard = my_shard;
    END IF;

    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE OR REPLACE TRIGGER durable_task_stat_counters_insert
    AFTER INSERT ON durable_task_queue
    REFERENCING NEW TABLE AS new_rows
    FOR EACH STATEMENT EXECUTE FUNCTION durable_task_stat_counters();

CREATE OR REPLACE TRIGGER durable_task_stat_counters_update
    AFTER UPDATE ON durable_task_queue
    REFERENCING OLD TABLE AS old_rows NEW TABLE AS new_rows
    FOR EACH STATEMENT EXECUTE FUNCTION durable_task_stat_counters();

CREATE OR REPLACE TRIGGER durable_task_stat_counters_delete
    AFTER DELETE ON durable_task_queue
    REFERENCING OLD TABLE AS old_rows
    FOR EACH STATEMENT EXECUTE FUNCTION durable_task_stat_counters();

CREATE OR REPLACE TRIGGER durable_workflow_stat_counters_insert
    AFTER INSERT ON durable_workflow_instances
    REFERENCING NEW TABLE AS new_rows
    FOR EACH STATEMENT EXECUTE FUNCTION durable_workflow_stat_counters();

CREATE OR REPLACE TRIGGER durable_workflow_stat_counters_update
    AFTER UPDATE ON durable_workflow_instances
    REFERENCING OLD TABLE AS old_rows NEW TABLE AS new_rows
    FOR EACH STATEMENT EXECUTE FUNCTION durable_workflow_stat_counters();

CREATE OR REPLACE TRIGGER durable_workflow_stat_counters_delete
    AFTER DELETE ON durable_workflow_instances
    REFERENCING OLD TABLE AS old_rows
    FOR EACH STATEMENT EXECUTE FUNCTION durable_workflow_stat_counters();

-- Seed the counters from current rows into shard 0. The trigger statements
-- above hold SHARE ROW EXCLUSIVE locks on both tables until commit, so no write
-- slips between these counts and the triggers. Existing counter rows are kept.
INSERT INTO durable_stat_counters (name, value) VALUES
    ('tasks_completed',     (SELECT COUNT(*) FROM durable_task_queue WHERE status = 'completed')),
    ('tasks_failed',        (SELECT COUNT(*) FROM durable_task_queue WHERE status IN ('failed', 'dead'))),
    ('tasks_started',       (SELECT COUNT(*) FROM durable_task_queue WHERE claimed_at IS NOT NULL)),
    ('workflows_completed', (SELECT COUNT(*) FROM durable_workflow_instances WHERE status = 'completed')),
    ('workflows_failed',    (SELECT COUNT(*) FROM durable_workflow_instances WHERE status IN ('failed', 'cancelled'))),
    ('workflows_started',   (SELECT COUNT(*) FROM durable_workflow_instances WHERE started_at IS NOT NULL))
ON CONFLICT (name, shard) DO NOTHING;

INSERT INTO durable_stat_counters (name, shard, value)
SELECT counters.name, shards.shard, 0
FROM (SELECT DISTINCT name FROM durable_stat_counters) counters
CROSS JOIN generate_series(1, 15) AS shards(shard)
ON CONFLICT (name, shard) DO NOTHING;

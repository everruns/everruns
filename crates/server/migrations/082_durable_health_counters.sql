-- Maintained counters for durable system-health totals (EVE-605).
--
-- `get_system_health` previously recomputed the cumulative task/workflow totals
-- (completed/failed/started) with COUNT(*) over `durable_task_queue` and
-- `durable_workflow_instances` on every call. Those tables grow without bound
-- (completed rows are never pruned), so the metrics sampler (~10s), the durable
-- SSE stream, and `GET /v1/durable/health` each triggered multiple full table
-- scans. On a ~156k-row dev table this produced recurring multi-second SQL and
-- DB pool acquire timeouts.
--
-- Fix: keep a tiny `durable_stat_counters` table (one row per cumulative metric)
-- maintained incrementally by AFTER triggers using delta accounting, so each
-- counter is always exactly equal to the COUNT(*) it replaces — including after
-- deletes — but reads become O(1) point lookups independent of history size.
-- The live gauges (pending/claimed tasks, running/pending workflows, workers,
-- DLQ) stay as direct queries; they are bounded by current work and already
-- backed by partial indexes.
--
-- The triggers are statement-level with transition tables (FOR EACH STATEMENT
-- ... REFERENCING). A batched claim/reclaim/insert that touches N rows applies a
-- single aggregated delta to each counter row instead of N per-row updates, so
-- the hot durable_task_queue write paths stay cheap and the shared counter rows
-- do not become per-row lock hotspots.

CREATE TABLE durable_stat_counters (
    name TEXT PRIMARY KEY,
    value BIGINT NOT NULL DEFAULT 0
);

-- ============================================
-- Task counters
-- ============================================
-- A counter row tracks the current number of task rows matching a predicate:
--   tasks_completed: status = 'completed'
--   tasks_failed:    status IN ('failed', 'dead')
--   tasks_started:   claimed_at IS NOT NULL
-- Production never prunes completed/failed rows, so these are effectively the
-- monotonic lifetime totals exposed as Prometheus-style counters.
--
-- Delta accounting per statement: for each predicate the net change is
-- (#matching rows in the NEW transition table) - (#matching rows in OLD). An
-- UPDATE sees both tables, an INSERT only NEW, a DELETE only OLD.
CREATE OR REPLACE FUNCTION durable_task_stat_counters()
RETURNS TRIGGER AS $$
DECLARE
    d_completed BIGINT := 0;
    d_failed BIGINT := 0;
    d_started BIGINT := 0;
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
        UPDATE durable_stat_counters SET value = value + d_completed WHERE name = 'tasks_completed';
    END IF;
    IF d_failed <> 0 THEN
        UPDATE durable_stat_counters SET value = value + d_failed WHERE name = 'tasks_failed';
    END IF;
    IF d_started <> 0 THEN
        UPDATE durable_stat_counters SET value = value + d_started WHERE name = 'tasks_started';
    END IF;

    RETURN NULL; -- AFTER trigger: return value is ignored
END;
$$ LANGUAGE plpgsql;

-- ============================================
-- Workflow counters
-- ============================================
--   workflows_completed: status = 'completed'
--   workflows_failed:    status IN ('failed', 'cancelled')
--   workflows_started:   started_at IS NOT NULL
CREATE OR REPLACE FUNCTION durable_workflow_stat_counters()
RETURNS TRIGGER AS $$
DECLARE
    d_completed BIGINT := 0;
    d_failed BIGINT := 0;
    d_started BIGINT := 0;
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
        UPDATE durable_stat_counters SET value = value + d_completed WHERE name = 'workflows_completed';
    END IF;
    IF d_failed <> 0 THEN
        UPDATE durable_stat_counters SET value = value + d_failed WHERE name = 'workflows_failed';
    END IF;
    IF d_started <> 0 THEN
        UPDATE durable_stat_counters SET value = value + d_started WHERE name = 'workflows_started';
    END IF;

    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

-- ============================================
-- Triggers (statement-level, transition tables)
-- ============================================
CREATE TRIGGER durable_task_stat_counters_insert
    AFTER INSERT ON durable_task_queue
    REFERENCING NEW TABLE AS new_rows
    FOR EACH STATEMENT EXECUTE FUNCTION durable_task_stat_counters();

CREATE TRIGGER durable_task_stat_counters_update
    AFTER UPDATE ON durable_task_queue
    REFERENCING OLD TABLE AS old_rows NEW TABLE AS new_rows
    FOR EACH STATEMENT EXECUTE FUNCTION durable_task_stat_counters();

CREATE TRIGGER durable_task_stat_counters_delete
    AFTER DELETE ON durable_task_queue
    REFERENCING OLD TABLE AS old_rows
    FOR EACH STATEMENT EXECUTE FUNCTION durable_task_stat_counters();

CREATE TRIGGER durable_workflow_stat_counters_insert
    AFTER INSERT ON durable_workflow_instances
    REFERENCING NEW TABLE AS new_rows
    FOR EACH STATEMENT EXECUTE FUNCTION durable_workflow_stat_counters();

CREATE TRIGGER durable_workflow_stat_counters_update
    AFTER UPDATE ON durable_workflow_instances
    REFERENCING OLD TABLE AS old_rows NEW TABLE AS new_rows
    FOR EACH STATEMENT EXECUTE FUNCTION durable_workflow_stat_counters();

CREATE TRIGGER durable_workflow_stat_counters_delete
    AFTER DELETE ON durable_workflow_instances
    REFERENCING OLD TABLE AS old_rows
    FOR EACH STATEMENT EXECUTE FUNCTION durable_workflow_stat_counters();

-- ============================================
-- Backfill
-- ============================================
-- The CREATE TRIGGER statements above take ACCESS EXCLUSIVE locks on both tables
-- for the remainder of this migration's transaction, so no concurrent writes can
-- slip between these COUNT(*)s and commit. After commit the counters and the
-- triggers stay in lockstep.
INSERT INTO durable_stat_counters (name, value) VALUES
    ('tasks_completed',     (SELECT COUNT(*) FROM durable_task_queue WHERE status = 'completed')),
    ('tasks_failed',        (SELECT COUNT(*) FROM durable_task_queue WHERE status IN ('failed', 'dead'))),
    ('tasks_started',       (SELECT COUNT(*) FROM durable_task_queue WHERE claimed_at IS NOT NULL)),
    ('workflows_completed', (SELECT COUNT(*) FROM durable_workflow_instances WHERE status = 'completed')),
    ('workflows_failed',    (SELECT COUNT(*) FROM durable_workflow_instances WHERE status IN ('failed', 'cancelled'))),
    ('workflows_started',   (SELECT COUNT(*) FROM durable_workflow_instances WHERE started_at IS NOT NULL));

-- Shard the durable health counters.
--
-- Migration 082 kept one `durable_stat_counters` row per metric. Every task
-- claim, completion and workflow start updates one of those rows from a
-- trigger, and the row stays locked until the writing transaction commits, so
-- every claim and hand-off in the system queued on the same few rows. Under a
-- 10-session llmsim load test the `tasks_started` UPDATE averaged 12.7 ms, all
-- of it lock wait.
--
-- Fix: each metric now has 16 shard rows. A trigger writes the shard picked by
-- `pg_backend_pid() % 16`, so concurrent connections mostly touch different
-- rows, and a reader sums the shards. A single shard can go negative (a
-- connection deletes a row another connection counted), so the per-row
-- non-negative CHECK is gone; the sum still equals the COUNT(*) it replaces.

ALTER TABLE durable_stat_counters DROP CONSTRAINT durable_stat_counters_pkey;
ALTER TABLE durable_stat_counters DROP CONSTRAINT durable_stat_counters_value_check;
ALTER TABLE durable_stat_counters ADD COLUMN shard SMALLINT NOT NULL DEFAULT 0;
ALTER TABLE durable_stat_counters ADD PRIMARY KEY (name, shard);

INSERT INTO durable_stat_counters (name, shard, value)
SELECT counters.name, shards.shard, 0
FROM durable_stat_counters counters
CROSS JOIN generate_series(1, 15) AS shards(shard);

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

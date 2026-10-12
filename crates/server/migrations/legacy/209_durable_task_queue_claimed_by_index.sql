-- no-transaction
-- Worker listings aggregate per-worker task stats. They now scope that
-- aggregate to live workers, which needs claimed_by lookups across every
-- status (the existing claimed_by index only covers status = 'claimed').
-- Separate from 208: CREATE INDEX CONCURRENTLY cannot share a file.
CREATE INDEX CONCURRENTLY idx_durable_task_queue_claimed_by
    ON durable_task_queue (claimed_by)
    WHERE claimed_by IS NOT NULL;

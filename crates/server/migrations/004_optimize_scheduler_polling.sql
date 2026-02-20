-- Optimize durable_schedules polling query
--
-- The claim_due_schedules query filters on (claimed_by IS NULL OR claimed_at < stale_threshold)
-- but existing indexes only cover (next_trigger_at) WHERE enabled = true. PostgreSQL must
-- heap-fetch every matching row to evaluate the claim columns. Adding them as INCLUDE columns
-- lets the planner filter directly from the index leaf pages before acquiring FOR UPDATE locks.
--
-- Also removes idx_durable_schedules_next_trigger which is an exact duplicate of
-- idx_durable_schedules_polling.

-- Drop duplicate index
DROP INDEX IF EXISTS idx_durable_schedules_next_trigger;

-- Replace with covering index for the claim query
DROP INDEX IF EXISTS idx_durable_schedules_polling;
CREATE INDEX idx_durable_schedules_polling
    ON durable_schedules (next_trigger_at)
    INCLUDE (claimed_by, claimed_at)
    WHERE enabled = true;

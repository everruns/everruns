-- Optimize task claiming query performance
--
-- The claim_task query uses:
--   WHERE status = 'pending' AND activity_type = ANY($1) AND visible_at <= NOW()
--   ORDER BY priority DESC, visible_at
--   LIMIT $2
--   FOR UPDATE SKIP LOCKED
--
-- The original index (activity_type, priority DESC, visible_at) is suboptimal when
-- activity_type = ANY($1) matches multiple types. PostgreSQL must scan each type
-- separately and merge-sort results.
--
-- This new index leads with ORDER BY columns, allowing PostgreSQL to walk the index
-- in sorted order, check each row against filters, and stop early with LIMIT.
-- This is much more efficient for FOR UPDATE SKIP LOCKED with concurrent workers.

-- Drop the old suboptimal index first
DROP INDEX IF EXISTS idx_durable_task_queue_pending;

-- New optimized index for task claiming
-- Leads with ORDER BY columns for efficient LIMIT + FOR UPDATE SKIP LOCKED
CREATE INDEX idx_durable_task_queue_pending
    ON durable_task_queue(priority DESC, visible_at, activity_type)
    WHERE status = 'pending';

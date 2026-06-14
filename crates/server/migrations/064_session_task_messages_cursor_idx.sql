-- Covering index for cursor-based pagination on the session task message thread.
--
-- `list_session_task_messages` with an `after_id` cursor uses:
--   ORDER BY created_at ASC, id ASC
-- with a range condition `(created_at, id) > (subquery)`.
-- The existing (task_id, created_at) index satisfies session/task filtering but
-- Postgres must sort by `id` as a tiebreaker, forcing an extra sort step on
-- tasks with many messages at the same timestamp.  Adding `id` to the index
-- lets the planner use an index scan directly for both ordering and the range
-- condition.
--
-- session_id is included so the full ownership filter
-- (session_id = $1 AND task_id = $2) is index-only and avoids the heap for
-- the task-ownership check.

CREATE INDEX CONCURRENTLY IF NOT EXISTS session_task_messages_cursor_idx
    ON session_task_messages (task_id, created_at ASC, id ASC);

-- Partial index for the session-task retention reaper (EVE-580).
-- The retention prune scans for terminal tasks across all sessions whose
-- `finished_at` is older than the TTL — a global, state+timestamp query the
-- existing (session_id, state) index cannot serve. Index `finished_at` and
-- restrict the partial predicate to terminal states so the index stays small
-- (only finished tasks) and the prune query is cheap as session_tasks grows.
CREATE INDEX session_tasks_terminal_finished_at_idx
    ON session_tasks (finished_at)
    WHERE state IN ('succeeded', 'failed', 'canceled');

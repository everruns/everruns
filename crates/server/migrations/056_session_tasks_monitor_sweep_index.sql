-- Partial index for the session scheduler's orphaned-monitor sweep
-- (list_monitor_tasks_with_inactive_schedules): the sweep scans for running
-- monitor tasks across all sessions, which the (session_id, state) index
-- cannot serve. Indexing the extracted schedule_id keeps the periodic query
-- cheap as session_tasks grows; the partial predicate keeps the index tiny
-- (only active monitors).
CREATE INDEX session_tasks_running_monitors_idx
    ON session_tasks ((spec->>'schedule_id'))
    WHERE kind = 'monitor' AND state = 'running';

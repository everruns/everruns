-- session_schedule_soft_delete
-- A deleted session schedule stays on the session so the recording can show
-- that it was registered and then removed. It must not fire again.

ALTER TABLE session_schedules
    ADD COLUMN deleted_at timestamp with time zone;

-- Live rows are the ones the poller, the quota, and the agent's list read.
CREATE INDEX idx_session_schedules_live
    ON session_schedules (session_id, created_at DESC)
    WHERE deleted_at IS NULL;

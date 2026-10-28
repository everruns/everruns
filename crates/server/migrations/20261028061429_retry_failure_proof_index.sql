-- no-transaction
-- Bound proof lookup to reason completions, independently of tool/stream volume.
CREATE INDEX CONCURRENTLY idx_events_reason_completed
    ON events (session_id, sequence DESC)
    WHERE event_type = 'reason.completed';

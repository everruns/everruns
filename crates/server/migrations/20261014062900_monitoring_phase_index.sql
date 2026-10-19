-- no-transaction
-- Build the read-side index without blocking live execution writes.
CREATE INDEX CONCURRENTLY idx_events_monitoring_phase ON events (session_id, sequence DESC)
    INCLUDE (event_type, ts)
    WHERE event_type IN ('turn.started', 'reason.started', 'reason.completed',
        'act.started', 'act.completed', 'turn.completed', 'turn.failed',
        'turn.cancelled', 'turn.sealed');

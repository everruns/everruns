-- no-transaction
-- Build the read-side index without blocking live execution writes.
CREATE INDEX CONCURRENTLY idx_events_monitoring_rate ON events (ts, session_id)
    INCLUDE (event_type)
    WHERE event_type IN ('turn.started', 'reason.started', 'act.started',
        'tool.started', 'tool.completed', 'turn.completed', 'turn.failed',
        'turn.cancelled', 'turn.sealed');

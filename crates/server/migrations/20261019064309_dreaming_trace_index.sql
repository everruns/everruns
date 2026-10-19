-- no-transaction
-- Keep the bounded background lane fast without blocking live event writes.
CREATE INDEX CONCURRENTLY idx_events_dreaming ON events(session_id, sequence DESC)
    WHERE event_type = 'dreaming.activity';

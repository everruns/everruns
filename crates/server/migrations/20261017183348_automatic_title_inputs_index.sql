-- no-transaction
-- Bound title admission scans without blocking live event writes during rollout.
CREATE INDEX CONCURRENTLY idx_events_title_inputs ON events(session_id, sequence)
    WHERE event_type = 'input.message';

-- no-transaction
-- Lets the Monitoring Timeline backfill read one day of turn intervals at a
-- time instead of scanning every projected turn.
CREATE INDEX CONCURRENTLY idx_session_trace_turns_started_at ON session_trace_turns (started_at);

-- no-transaction
-- Build the read-side index without blocking live execution writes.
CREATE INDEX CONCURRENTLY idx_sessions_monitoring_live ON sessions (org_id, id)
    WHERE status IN ('active', 'waiting_for_tool_results', 'resolving_tool_results', 'paused');

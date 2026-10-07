-- no-transaction
-- Re-projecting a session's configured capabilities deletes that session's
-- `configured` facts by session. With no index on session_id the delete
-- scanned the whole fact table, so every session projection got slower as
-- reporting history grew (18 ms at 57k rows in the 2026-10-06 load test).
-- Build concurrently so applying this migration does not block fact writes.
CREATE INDEX CONCURRENTLY idx_fact_capability_usage_org_session
    ON fact_capability_usage (org_id, session_id)
    WHERE capability_usage_kind = 'configured';

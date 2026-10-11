-- no-transaction
-- Session listing is scoped to one organization and ordered newest-first.
-- Build concurrently so applying this migration does not block session writes.
CREATE INDEX CONCURRENTLY idx_sessions_org_created_at
    ON sessions (org_id, created_at DESC);

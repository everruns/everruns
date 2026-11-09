-- Deleting a session keeps its record. `deleted_at` marks it (and its subagent
-- sessions) deleted; reads a person or an agent can reach leave marked rows
-- out. Nullable with no default, so adding it does not rewrite the table.
-- Deleting a user or an organization still erases their sessions.
ALTER TABLE sessions ADD COLUMN deleted_at TIMESTAMPTZ;

COMMENT ON COLUMN sessions.deleted_at IS
    'Set when the session was deleted. The row and everything it owns stay; readers filter on deleted_at IS NULL.';

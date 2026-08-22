-- Archive a session without deleting it.
--
-- Chat threads accumulate: a finished conversation is not garbage (its events
-- are the record of what an agent did) but it should stop competing for
-- attention in the thread list. `archived_at` is that "put it away" bit —
-- distinct from `status`/`activity`, which describe execution, and from
-- deletion, which destroys history.
--
-- Default list results hide archived sessions; `include_archived=true` widens
-- them back. Archive is a property of the session, not of the viewer (unlike
-- `pinned_sessions`), so a thread archived by its owner is archived for
-- everyone who can see it.

ALTER TABLE sessions ADD COLUMN archived_at TIMESTAMPTZ;

COMMENT ON COLUMN sessions.archived_at IS
    'When the session was archived; NULL means active. Archived sessions are hidden from default list results.';

-- The default list predicate is "org, not archived, newest activity first".
-- A partial index keeps archived rows out of the index entirely, so archiving
-- shrinks the hot path instead of adding to it.
CREATE INDEX idx_sessions_org_active_updated_at
    ON sessions(org_id, updated_at DESC)
    WHERE archived_at IS NULL;

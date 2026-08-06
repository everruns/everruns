-- Record fork provenance on sessions (knowledge/runtime-resources/forking-sessions.md).
--
-- Forking is a user-initiated "branch from here" relationship and is distinct
-- from subagent nesting (`parent_session_id`, the recursion guard). These two
-- nullable columns capture where a session was forked from:
--
--   forked_from_session_id  - the session this one was forked from.
--   forked_from_sequence    - the parent event sequence the fork was taken at
--                             (high-water mark of copied history; the fork
--                             point for display and future arbitrary-point
--                             forking).
--
-- NULL on both columns is the default for every existing and non-forked
-- session. ON DELETE SET NULL keeps a fork independent once made: deleting the
-- parent clears the back-reference rather than cascade-deleting its forks.
ALTER TABLE sessions
    ADD COLUMN forked_from_session_id UUID REFERENCES sessions(id) ON DELETE SET NULL;
ALTER TABLE sessions
    ADD COLUMN forked_from_sequence INTEGER;

CREATE INDEX idx_sessions_forked_from_session_id
    ON sessions (forked_from_session_id)
    WHERE forked_from_session_id IS NOT NULL;

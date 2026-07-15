-- Denormalize `root_session_id` onto `sessions` and `session_tasks`.
--
-- EVE-680 (governed subagent nesting) makes a delegation tree — coordinator →
-- area owner → worker — a first-class object: the whole tree shares one root
-- session, which (in later PRs) owns the shared budget pool and anchors
-- tree-level observability. This migration lands only the denormalized column
-- and its backfill; depth caps, breadth caps, and the shared budget arrive in
-- follow-up PRs.
--
-- Semantics: a top-level session (`parent_session_id IS NULL`) is its own root;
-- a child (subagent) session inherits its parent's root. Denormalizing the root
-- turns "give me the whole tree" into one indexed lookup instead of a per-row
-- walk up `parent_session_id`. `session_tasks.root_session_id` mirrors the root
-- of the task's owning session so `GET /v1/tasks?root_session_id=…` filters a
-- whole tree's work with a local column and no join.
--
-- The column is a self-referential FK with ON DELETE CASCADE, matching
-- `parent_session_id` (007_v0.8.6): deleting a root already cascades the subtree
-- via the parent chain, so the denormalized pointer never dangles.

ALTER TABLE sessions
    ADD COLUMN root_session_id UUID REFERENCES sessions(id) ON DELETE CASCADE;

ALTER TABLE session_tasks
    ADD COLUMN root_session_id UUID;

-- Backfill sessions: walk each session's parent chain to its top-level ancestor.
-- Anchored at NULL-parent roots (root_session_id = self), then propagated down.
WITH RECURSIVE session_roots AS (
    SELECT id, id AS root_id
    FROM sessions
    WHERE parent_session_id IS NULL
  UNION ALL
    SELECT s.id, r.root_id
    FROM sessions s
    JOIN session_roots r ON s.parent_session_id = r.id
)
UPDATE sessions s
SET root_session_id = session_roots.root_id
FROM session_roots
WHERE session_roots.id = s.id;

-- Any session not reachable from a NULL-parent root (e.g. a parent row already
-- gone) falls back to being its own root, so the column is never NULL after
-- backfill for existing rows.
UPDATE sessions
SET root_session_id = id
WHERE root_session_id IS NULL;

-- Backfill tasks from their owning session's (now-populated) root.
UPDATE session_tasks t
SET root_session_id = s.root_session_id
FROM sessions s
WHERE s.id = t.session_id;

CREATE INDEX sessions_root_session_id_idx ON sessions (root_session_id);
CREATE INDEX session_tasks_root_session_id_idx ON session_tasks (root_session_id);

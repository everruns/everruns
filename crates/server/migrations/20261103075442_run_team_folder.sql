-- A run's team folder: one Memory owned by the run's first session, mounted
-- at /team in every session of the run
-- (knowledge/runtime-resources/agents-using-agents.md). Memories are never
-- purged, so there is no foreign key to the session.
ALTER TABLE memories ADD COLUMN owner_session_id uuid;

ALTER TABLE memories DROP CONSTRAINT memories_scope_check;
ALTER TABLE memories ADD CONSTRAINT memories_scope_check
    CHECK (scope IN ('org', 'agent', 'user', 'run'));

ALTER TABLE memories DROP CONSTRAINT memories_scope_owner_shape;
ALTER TABLE memories ADD CONSTRAINT memories_scope_owner_shape CHECK (
    (scope = 'org' AND owner_agent_id IS NULL AND owner_user_id IS NULL AND owner_session_id IS NULL)
    OR (scope = 'agent' AND owner_agent_id IS NOT NULL AND owner_user_id IS NULL AND owner_session_id IS NULL)
    OR (scope = 'user' AND owner_agent_id IS NULL AND owner_user_id IS NOT NULL AND owner_session_id IS NULL)
    OR (scope = 'run' AND owner_agent_id IS NULL AND owner_user_id IS NULL AND owner_session_id IS NOT NULL)
);

CREATE UNIQUE INDEX idx_memories_org_run_scope_owner_active
    ON memories (org_id, owner_session_id)
    WHERE scope = 'run' AND status <> 'deleted';

-- Memory scopes for agent/user-owned durable memory (EVE-699).
-- Existing Memories remain org-scoped and keep their current behavior.

BEGIN;

ALTER TABLE memories
    ADD COLUMN scope VARCHAR(16) NOT NULL DEFAULT 'org',
    ADD COLUMN owner_agent_id UUID REFERENCES agents(id),
    ADD COLUMN owner_user_id UUID REFERENCES users(id);

ALTER TABLE memories
    ADD CONSTRAINT memories_scope_check
        CHECK (scope IN ('org', 'agent', 'user')),
    ADD CONSTRAINT memories_scope_owner_shape CHECK (
        (scope = 'org' AND owner_agent_id IS NULL AND owner_user_id IS NULL)
        OR (scope = 'agent' AND owner_agent_id IS NOT NULL AND owner_user_id IS NULL)
        OR (scope = 'user' AND owner_agent_id IS NULL AND owner_user_id IS NOT NULL)
    ),
    ADD CONSTRAINT memories_scoped_manual_source CHECK (
        scope = 'org'
        OR (source_type = 'manual' AND source_config = '{}'::jsonb AND is_readonly = FALSE)
    );

CREATE UNIQUE INDEX idx_memories_org_agent_scope_owner_active
    ON memories(org_id, owner_agent_id)
    WHERE scope = 'agent' AND status != 'deleted';

CREATE UNIQUE INDEX idx_memories_org_user_scope_owner_active
    ON memories(org_id, owner_user_id)
    WHERE scope = 'user' AND status != 'deleted';

CREATE INDEX idx_memories_org_scope_status
    ON memories(org_id, scope, status);

COMMENT ON COLUMN memories.scope IS 'Memory scope: org, agent, or user. See knowledge/runtime-resources/memory.md.';
COMMENT ON COLUMN memories.owner_agent_id IS 'Owning agent for scope=agent memories.';
COMMENT ON COLUMN memories.owner_user_id IS 'Owning user for scope=user memories.';

COMMIT;

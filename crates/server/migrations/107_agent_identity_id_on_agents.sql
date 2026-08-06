-- Agent-owned identity: each agent gets a lazily-created agent_identity
-- principal on its first unattended action (e.g. an agent trigger fire), so
-- unattended sessions are owned by the agent acting as itself rather than the
-- org system-owner. See knowledge/runtime-resources/agent-triggers.md and knowledge/runtime-resources/agent-identities.md.
--
-- Schema-only: nullable column plus a partial index. No data backfill and no
-- retroactive rewrite of historical sessions.owner_principal_id — the identity
-- is created on demand the next time an agent fires.

ALTER TABLE agents
    ADD COLUMN agent_identity_id UUID REFERENCES agent_identities(id);

CREATE INDEX idx_agents_agent_identity_id
    ON agents(agent_identity_id)
    WHERE agent_identity_id IS NOT NULL;

-- An agent created as part of another agent ("Created here" on the Agents tab).
-- It is a real agent: it runs, exports and costs like any other. Owning only
-- groups it under its owner on Agents home and bundles it into the owner's
-- export. Deleting the owner keeps the agent and clears the link.
ALTER TABLE agents
    ADD COLUMN owner_agent_id UUID REFERENCES agents(id) ON DELETE SET NULL;

CREATE INDEX idx_agents_owner_agent_id ON agents (owner_agent_id)
    WHERE owner_agent_id IS NOT NULL;

COMMENT ON COLUMN agents.owner_agent_id IS
    'Agent this agent was created as part of. Groups it on Agents home and bundles it into the owner''s export.';

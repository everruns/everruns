-- Agent scripts ("saved scripts"): org-scoped shell scripts an agent owns.
--
-- A saved script is a name, a one-line description, an optional JSON Schema
-- for its input, and the body. Same lifecycle as agent_triggers (104):
-- delete archives, the name of an archived script is free again.
-- See knowledge/runtime-resources/agent-scripts.md.

CREATE TABLE agent_scripts (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) DEFAULT 1,
    agent_id UUID NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    name VARCHAR(64) NOT NULL,
    description TEXT NOT NULL,
    input_schema JSONB,
    body TEXT NOT NULL,
    status VARCHAR(50) NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'archived', 'deleted')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    archived_at TIMESTAMPTZ,
    deleted_at TIMESTAMPTZ
);

CREATE INDEX idx_agent_scripts_org_id ON agent_scripts(org_id);
CREATE INDEX idx_agent_scripts_agent_id ON agent_scripts(agent_id);

-- One active script per (agent, name).
CREATE UNIQUE INDEX agent_scripts_agent_name_active_idx
    ON agent_scripts (agent_id, name)
    WHERE status = 'active';

CREATE TRIGGER update_agent_scripts_updated_at BEFORE UPDATE ON agent_scripts
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

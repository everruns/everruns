CREATE TABLE agent_mcp_secret_bindings (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    org_id BIGINT NOT NULL REFERENCES organizations(org_id),
    agent_id UUID NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    mcp_server_name VARCHAR(255) NOT NULL,
    mcp_server_url TEXT NOT NULL,
    tool_name VARCHAR(255) NOT NULL,
    parameter_name VARCHAR(255) NOT NULL,
    label VARCHAR(255) NOT NULL,
    description TEXT,
    value_encrypted BYTEA,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (agent_id, mcp_server_name, tool_name, parameter_name)
);

CREATE INDEX idx_agent_mcp_secret_bindings_agent
    ON agent_mcp_secret_bindings(org_id, agent_id);

CREATE TRIGGER update_agent_mcp_secret_bindings_updated_at
    BEFORE UPDATE ON agent_mcp_secret_bindings
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

COMMENT ON TABLE agent_mcp_secret_bindings IS
    'Encrypted Agent-scoped MCP tool-argument credentials; values are write-only and injected after model tool-call persistence.';

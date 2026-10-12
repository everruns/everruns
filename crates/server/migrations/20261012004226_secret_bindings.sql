-- secret_bindings
-- Bound secrets (knowledge/security/bound-secrets.md): a secret stored with the
-- URL and auth form it may be sent with. Bound values are write-only; the
-- shell gets a placeholder and Bashkit swaps the value into request headers
-- only for the bound URL.

ALTER TABLE session_secrets
    ADD COLUMN binding JSONB;

COMMENT ON COLUMN session_secrets.binding IS
    'SecretBinding {url, auth}. NULL: an ordinary secret the model may read. Set: write-only, injected only into requests to url.';

CREATE TABLE agent_secrets (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    org_id BIGINT NOT NULL REFERENCES organizations(org_id),
    agent_id UUID NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    name VARCHAR(255) NOT NULL,
    binding JSONB NOT NULL,
    value_encrypted BYTEA NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (agent_id, name)
);

CREATE INDEX idx_agent_secrets_org_agent ON agent_secrets(org_id, agent_id);

CREATE TRIGGER update_agent_secrets_updated_at
    BEFORE UPDATE ON agent_secrets
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

COMMENT ON TABLE agent_secrets IS
    'Agent-scoped bound secrets: write-only values injected into HTTP requests to the bound URL for every session of the agent.';

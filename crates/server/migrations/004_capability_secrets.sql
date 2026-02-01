-- Agent Capability Secrets Table
-- Stores encrypted secrets for agent capability configurations (e.g., API keys for web search providers)
-- Secrets are per-agent, per-capability, enabling each agent to have its own credentials

CREATE TABLE IF NOT EXISTS agent_capability_secrets (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    agent_id UUID NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    capability_id VARCHAR(100) NOT NULL,
    secret_name VARCHAR(100) NOT NULL,
    value_encrypted BYTEA NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- Each agent can have one secret per (capability_id, secret_name) combination
    UNIQUE(agent_id, capability_id, secret_name)
);

-- Index for efficient lookup by agent
CREATE INDEX IF NOT EXISTS idx_agent_capability_secrets_agent_id ON agent_capability_secrets(agent_id);

-- Index for org-level queries (e.g., listing all secrets for an org)
CREATE INDEX IF NOT EXISTS idx_agent_capability_secrets_org_id ON agent_capability_secrets(org_id);

-- Combined index for the common lookup pattern
CREATE INDEX IF NOT EXISTS idx_agent_capability_secrets_lookup
    ON agent_capability_secrets(agent_id, capability_id);

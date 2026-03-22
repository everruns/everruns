-- Migration 009: Agent identity connections
-- Mirrors user_connections but scoped to agent_identity_id instead of user_id.
-- Allows agent identities to have their own connection credentials for
-- unattended execution without borrowing from the session creator.

----------------------------------------------------------------------
-- 001: Agent identity connections
----------------------------------------------------------------------

CREATE TABLE agent_identity_connections (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    agent_identity_id UUID NOT NULL REFERENCES agent_identities(id) ON DELETE CASCADE,
    -- Provider identifier: 'github', 'daytona', etc.
    provider VARCHAR(50) NOT NULL,
    -- Connection type: 'oauth' or 'api_key'
    connection_type VARCHAR(20) NOT NULL DEFAULT 'api_key',
    -- Provider-side user identity
    provider_user_id TEXT,
    provider_username TEXT,
    -- Encrypted credentials (AES-256-GCM envelope encryption)
    access_token_encrypted BYTEA,
    refresh_token_encrypted BYTEA,
    -- GitHub App: installation_id for minting short-lived tokens
    installation_id BIGINT,
    -- What scopes were granted
    scopes TEXT,
    -- NULL = no expiry
    expires_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    -- One connection per identity per provider
    UNIQUE (agent_identity_id, provider)
);

CREATE INDEX idx_agent_identity_connections_identity
    ON agent_identity_connections(agent_identity_id);

CREATE TRIGGER update_agent_identity_connections_updated_at
    BEFORE UPDATE ON agent_identity_connections
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

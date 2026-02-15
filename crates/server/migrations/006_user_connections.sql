-- User Connections: external service accounts linked to user identity
-- Decision: User-scoped (not org-scoped) — token represents user's identity on external service
-- Decision: No unique constraint on (user_id, provider) — enforced in app code for future flexibility
-- Decision: Same envelope encryption as session_secrets and mcp_servers.api_key_encrypted

CREATE TABLE user_connections (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    -- Provider identifier: 'github', 'gitlab', 'bitbucket'
    provider VARCHAR(50) NOT NULL,
    -- Provider-side user identity
    provider_user_id TEXT,
    provider_username TEXT,
    -- Encrypted credentials (AES-256-GCM envelope encryption)
    access_token_encrypted BYTEA NOT NULL,
    refresh_token_encrypted BYTEA,
    -- What scopes were granted (e.g., 'repo,read:user')
    scopes TEXT,
    -- NULL = no expiry (GitHub OAuth App tokens don't expire)
    expires_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_user_connections_user_id ON user_connections(user_id);
CREATE INDEX idx_user_connections_provider ON user_connections(user_id, provider);

CREATE TRIGGER update_user_connections_updated_at
    BEFORE UPDATE ON user_connections
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

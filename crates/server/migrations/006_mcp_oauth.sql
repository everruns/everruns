-- Migration: Add MCP OAuth support
-- This migration adds OAuth 2.1 authentication support for MCP servers

-- ============================================
-- MCP Servers OAuth columns
-- ============================================

-- Auth type: 'none' (default), 'api_key' (uses existing api_key_encrypted), 'oauth'
ALTER TABLE mcp_servers ADD COLUMN auth_type VARCHAR(50) NOT NULL DEFAULT 'none'
    CHECK (auth_type IN ('none', 'api_key', 'oauth'));

-- OAuth configuration fields
ALTER TABLE mcp_servers ADD COLUMN oauth_authorization_url TEXT;
ALTER TABLE mcp_servers ADD COLUMN oauth_token_url TEXT;
ALTER TABLE mcp_servers ADD COLUMN oauth_client_id TEXT;
ALTER TABLE mcp_servers ADD COLUMN oauth_client_secret_encrypted BYTEA;
ALTER TABLE mcp_servers ADD COLUMN oauth_scopes JSONB;
ALTER TABLE mcp_servers ADD COLUMN oauth_resource_metadata_url TEXT;

-- Migrate existing api_key_set servers to auth_type = 'api_key'
UPDATE mcp_servers SET auth_type = 'api_key' WHERE api_key_set = TRUE;

COMMENT ON COLUMN mcp_servers.auth_type IS 'Authentication type: none, api_key, or oauth';
COMMENT ON COLUMN mcp_servers.oauth_authorization_url IS 'OAuth authorization endpoint URL';
COMMENT ON COLUMN mcp_servers.oauth_token_url IS 'OAuth token endpoint URL';
COMMENT ON COLUMN mcp_servers.oauth_client_id IS 'OAuth client ID (public)';
COMMENT ON COLUMN mcp_servers.oauth_client_secret_encrypted IS 'Encrypted OAuth client secret';
COMMENT ON COLUMN mcp_servers.oauth_scopes IS 'Required OAuth scopes as JSON array';
COMMENT ON COLUMN mcp_servers.oauth_resource_metadata_url IS 'RFC 9728 Protected Resource Metadata URL';

-- ============================================
-- MCP User Tokens (per-user OAuth tokens)
-- ============================================

CREATE TABLE mcp_user_tokens (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    mcp_server_id UUID NOT NULL REFERENCES mcp_servers(id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    access_token_encrypted BYTEA NOT NULL,
    refresh_token_encrypted BYTEA,
    token_type TEXT NOT NULL DEFAULT 'Bearer',
    scope TEXT,
    expires_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(mcp_server_id, user_id)
);

CREATE INDEX idx_mcp_user_tokens_user_id ON mcp_user_tokens(user_id);
CREATE INDEX idx_mcp_user_tokens_mcp_server_id ON mcp_user_tokens(mcp_server_id);
CREATE INDEX idx_mcp_user_tokens_expires_at ON mcp_user_tokens(expires_at);

CREATE TRIGGER update_mcp_user_tokens_updated_at BEFORE UPDATE ON mcp_user_tokens
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

COMMENT ON TABLE mcp_user_tokens IS 'Per-user OAuth tokens for MCP servers';
COMMENT ON COLUMN mcp_user_tokens.access_token_encrypted IS 'Encrypted OAuth access token';
COMMENT ON COLUMN mcp_user_tokens.refresh_token_encrypted IS 'Encrypted OAuth refresh token';
COMMENT ON COLUMN mcp_user_tokens.expires_at IS 'Access token expiration time';

-- ============================================
-- MCP OAuth States (PKCE and CSRF protection)
-- ============================================

CREATE TABLE mcp_oauth_states (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    mcp_server_id UUID NOT NULL REFERENCES mcp_servers(id) ON DELETE CASCADE,
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) DEFAULT 1,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    code_verifier TEXT NOT NULL,
    redirect_uri TEXT NOT NULL,
    return_url TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX idx_mcp_oauth_states_expires_at ON mcp_oauth_states(expires_at);
CREATE INDEX idx_mcp_oauth_states_user_id ON mcp_oauth_states(user_id);

COMMENT ON TABLE mcp_oauth_states IS 'Temporary OAuth state storage for PKCE and CSRF protection';
COMMENT ON COLUMN mcp_oauth_states.code_verifier IS 'PKCE code verifier';
COMMENT ON COLUMN mcp_oauth_states.return_url IS 'URL to redirect to after OAuth completes';
COMMENT ON COLUMN mcp_oauth_states.expires_at IS 'State expiration (typically 10 minutes)';

-- ============================================
-- Sessions: Add user_id for OAuth token lookup
-- ============================================

-- Add user_id to sessions for per-user OAuth token lookup
ALTER TABLE sessions ADD COLUMN user_id UUID REFERENCES users(id) ON DELETE SET NULL;

CREATE INDEX idx_sessions_user_id ON sessions(user_id);

COMMENT ON COLUMN sessions.user_id IS 'User who created this session (for per-user OAuth tokens)';

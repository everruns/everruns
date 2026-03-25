-- MCP OAuth: Dynamic client registration, authorization codes, refresh tokens
-- Supports OAuth 2.1 + PKCE for MCP client authentication

CREATE TABLE oauth_clients (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    client_id TEXT NOT NULL UNIQUE,
    client_secret_hash TEXT NOT NULL,
    client_name TEXT NOT NULL,
    redirect_uris JSONB NOT NULL DEFAULT '[]'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_oauth_clients_client_id ON oauth_clients(client_id);

CREATE TABLE oauth_authorization_codes (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    code_hash TEXT NOT NULL UNIQUE,
    client_id TEXT NOT NULL,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    org_id BIGINT NOT NULL,
    redirect_uri TEXT NOT NULL,
    code_challenge TEXT NOT NULL,
    code_challenge_method TEXT NOT NULL DEFAULT 'S256',
    scope TEXT NOT NULL DEFAULT 'mcp',
    consumed BOOLEAN NOT NULL DEFAULT FALSE,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_oauth_authorization_codes_code_hash ON oauth_authorization_codes(code_hash);
CREATE INDEX idx_oauth_authorization_codes_expires_at ON oauth_authorization_codes(expires_at);

CREATE TABLE oauth_refresh_tokens (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    token_hash TEXT NOT NULL UNIQUE,
    client_id TEXT NOT NULL,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    org_id BIGINT NOT NULL,
    scope TEXT NOT NULL DEFAULT 'mcp',
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_oauth_refresh_tokens_token_hash ON oauth_refresh_tokens(token_hash);
CREATE INDEX idx_oauth_refresh_tokens_expires_at ON oauth_refresh_tokens(expires_at);

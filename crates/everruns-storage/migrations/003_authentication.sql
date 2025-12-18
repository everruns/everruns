-- Authentication tables
-- Decision: Support multiple auth methods - password, OAuth, API keys
-- Decision: Store password hashes using Argon2id
-- Decision: API keys are hashed with SHA-256 for lookup, full key shown only once at creation

-- Alter users table to add authentication fields
ALTER TABLE users ADD COLUMN IF NOT EXISTS password_hash TEXT;
ALTER TABLE users ADD COLUMN IF NOT EXISTS email_verified BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE users ADD COLUMN IF NOT EXISTS auth_provider TEXT; -- 'local', 'google', 'github'
ALTER TABLE users ADD COLUMN IF NOT EXISTS auth_provider_id TEXT; -- External provider user ID

-- Index for OAuth provider lookup
CREATE INDEX IF NOT EXISTS idx_users_auth_provider ON users(auth_provider, auth_provider_id)
    WHERE auth_provider IS NOT NULL;

-- API Keys table
CREATE TABLE IF NOT EXISTS api_keys (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    -- SHA-256 hash of the key (for lookup)
    key_hash TEXT NOT NULL,
    -- Prefix of the key for display (e.g., "evr_abc...")
    key_prefix TEXT NOT NULL,
    scopes JSONB NOT NULL DEFAULT '["*"]'::jsonb,
    expires_at TIMESTAMPTZ,
    last_used_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE UNIQUE INDEX idx_api_keys_hash ON api_keys(key_hash);
CREATE INDEX idx_api_keys_user_id ON api_keys(user_id);
CREATE INDEX idx_api_keys_expires_at ON api_keys(expires_at) WHERE expires_at IS NOT NULL;

-- Refresh tokens table (for JWT refresh flow)
CREATE TABLE IF NOT EXISTS refresh_tokens (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash TEXT NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE UNIQUE INDEX idx_refresh_tokens_hash ON refresh_tokens(token_hash);
CREATE INDEX idx_refresh_tokens_user_id ON refresh_tokens(user_id);
CREATE INDEX idx_refresh_tokens_expires_at ON refresh_tokens(expires_at);

-- Drop old sessions table if it exists (replaced by refresh_tokens + JWT)
-- Note: We keep sessions table for backwards compatibility but it's unused

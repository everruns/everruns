-- CLI authentication: metadata on API keys + CLI auth sessions table

-- Add metadata JSONB column to api_keys for creation context
ALTER TABLE api_keys ADD COLUMN IF NOT EXISTS metadata JSONB NOT NULL DEFAULT '{}';

-- CLI auth sessions (short-lived, for the OAuth exchange flow)
CREATE TABLE cli_auth_sessions (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    state TEXT NOT NULL UNIQUE,
    exchange_code TEXT NOT NULL UNIQUE,
    user_id UUID REFERENCES users(id) ON DELETE CASCADE,
    redirect_port INT NOT NULL,
    completed BOOLEAN NOT NULL DEFAULT FALSE,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_cli_auth_sessions_state ON cli_auth_sessions(state);
CREATE INDEX idx_cli_auth_sessions_exchange_code ON cli_auth_sessions(exchange_code);
CREATE INDEX idx_cli_auth_sessions_expires_at ON cli_auth_sessions(expires_at);

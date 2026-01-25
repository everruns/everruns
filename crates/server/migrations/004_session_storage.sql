-- Migration: Session Key/Value and Secret Storage
-- Adds session-scoped storage for key/value pairs and encrypted secrets.

-- ============================================
-- Session Key/Value Storage
-- ============================================

-- Simple key/value storage scoped to sessions.
-- Use case: Agents can persist data across turns within a session.
CREATE TABLE session_key_values (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    session_id UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,

    -- Key name (unique per session)
    key VARCHAR(255) NOT NULL,

    -- Value (stored as text, can be JSON)
    value TEXT NOT NULL,

    -- Timestamps
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- Unique key per session
    CONSTRAINT session_key_values_unique_key UNIQUE (session_id, key)
);

-- Index for listing all keys in a session
CREATE INDEX idx_session_key_values_session_id ON session_key_values(session_id);

-- Index for key lookup
CREATE INDEX idx_session_key_values_key ON session_key_values(session_id, key);

-- Auto-update updated_at
CREATE TRIGGER update_session_key_values_updated_at
    BEFORE UPDATE ON session_key_values
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

-- ============================================
-- Session Secret Storage (Encrypted)
-- ============================================

-- Encrypted secret storage scoped to sessions.
-- Use case: Agents can securely store sensitive data like API keys, tokens, etc.
-- Secrets are encrypted using envelope encryption (AES-256-GCM).
CREATE TABLE session_secrets (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    session_id UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,

    -- Secret name (unique per session)
    name VARCHAR(255) NOT NULL,

    -- Encrypted value (JSON-encoded EncryptedPayload from encryption.rs)
    -- Contains: version, alg, key_id, dek_wrapped, nonce, ciphertext
    value_encrypted BYTEA NOT NULL,

    -- Timestamps
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- Unique name per session
    CONSTRAINT session_secrets_unique_name UNIQUE (session_id, name)
);

-- Index for listing all secrets in a session
CREATE INDEX idx_session_secrets_session_id ON session_secrets(session_id);

-- Index for secret name lookup
CREATE INDEX idx_session_secrets_name ON session_secrets(session_id, name);

-- Auto-update updated_at
CREATE TRIGGER update_session_secrets_updated_at
    BEFORE UPDATE ON session_secrets
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

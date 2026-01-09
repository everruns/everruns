-- Everruns Base Schema (v0.3.0)
-- Squashed migration - represents the final state of all base migrations
--
-- Key design decisions:
-- - UUID v7 for time-ordered, sortable IDs (better for DB performance)
-- - Messages stored as events (single source of truth)
-- - API keys encrypted with AES-256-GCM, key from SECRETS_ENCRYPTION_KEY env var
-- - PostgreSQL 17 with custom uuidv7() function (native support requires PostgreSQL 18+)
-- - Capabilities validated at application layer via CapabilityRegistry (no DB constraint)
-- - Session status: started/active/idle lifecycle

-- ============================================
-- UUID v7 Function (conditional for PG < 18)
-- ============================================
-- UUID v7: time-ordered UUIDs with millisecond timestamp prefix
-- Format: 48-bit timestamp | 4-bit version (7) | 12-bit random | 2-bit variant | 62-bit random
-- PostgreSQL 18+ has native uuidv7(); this creates a fallback for older versions.

-- pgcrypto provides gen_random_bytes() for PG < 17
CREATE EXTENSION IF NOT EXISTS pgcrypto;

-- Only create uuidv7() if it doesn't already exist (preserves native PG18+ function)
DO $$
BEGIN
    -- Check if uuidv7() function exists
    IF NOT EXISTS (
        SELECT 1 FROM pg_proc p
        JOIN pg_namespace n ON p.pronamespace = n.oid
        WHERE p.proname = 'uuidv7'
        AND n.nspname = 'pg_catalog'
    ) AND NOT EXISTS (
        SELECT 1 FROM pg_proc p
        JOIN pg_namespace n ON p.pronamespace = n.oid
        WHERE p.proname = 'uuidv7'
        AND n.nspname = 'public'
    ) THEN
        -- Create custom uuidv7() for PostgreSQL < 18
        CREATE FUNCTION uuidv7() RETURNS uuid AS $func$
        DECLARE
            timestamp_ms BIGINT;
            uuid_bytes BYTEA;
        BEGIN
            timestamp_ms := (EXTRACT(EPOCH FROM clock_timestamp()) * 1000)::BIGINT;
            uuid_bytes := gen_random_bytes(16);
            uuid_bytes := set_byte(uuid_bytes, 0, ((timestamp_ms >> 40) & 255)::INT);
            uuid_bytes := set_byte(uuid_bytes, 1, ((timestamp_ms >> 32) & 255)::INT);
            uuid_bytes := set_byte(uuid_bytes, 2, ((timestamp_ms >> 24) & 255)::INT);
            uuid_bytes := set_byte(uuid_bytes, 3, ((timestamp_ms >> 16) & 255)::INT);
            uuid_bytes := set_byte(uuid_bytes, 4, ((timestamp_ms >> 8) & 255)::INT);
            uuid_bytes := set_byte(uuid_bytes, 5, (timestamp_ms & 255)::INT);
            uuid_bytes := set_byte(uuid_bytes, 6, (get_byte(uuid_bytes, 6) & 15) | 112);
            uuid_bytes := set_byte(uuid_bytes, 8, (get_byte(uuid_bytes, 8) & 63) | 128);
            RETURN encode(uuid_bytes, 'hex')::uuid;
        END;
        $func$ LANGUAGE plpgsql;
    END IF;
END;
$$;

-- ============================================
-- Utility Functions
-- ============================================

-- Updated_at trigger function
CREATE OR REPLACE FUNCTION update_updated_at_column()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- ============================================
-- Users
-- ============================================

CREATE TABLE users (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    email TEXT NOT NULL,
    name TEXT NOT NULL,
    avatar_url TEXT,
    roles JSONB NOT NULL DEFAULT '[]'::jsonb,
    -- Authentication fields
    password_hash TEXT,
    email_verified BOOLEAN NOT NULL DEFAULT FALSE,
    auth_provider TEXT, -- 'local', 'google', 'github'
    auth_provider_id TEXT, -- External provider user ID
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE UNIQUE INDEX idx_users_email ON users(email);
CREATE INDEX idx_users_auth_provider ON users(auth_provider, auth_provider_id)
    WHERE auth_provider IS NOT NULL;

CREATE TRIGGER update_users_updated_at BEFORE UPDATE ON users
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

-- ============================================
-- API Keys
-- ============================================

CREATE TABLE api_keys (
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

-- ============================================
-- Refresh Tokens (for JWT refresh flow)
-- ============================================

CREATE TABLE refresh_tokens (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash TEXT NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE UNIQUE INDEX idx_refresh_tokens_hash ON refresh_tokens(token_hash);
CREATE INDEX idx_refresh_tokens_user_id ON refresh_tokens(user_id);
CREATE INDEX idx_refresh_tokens_expires_at ON refresh_tokens(expires_at);

-- ============================================
-- LLM Providers
-- ============================================

CREATE TABLE llm_providers (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    name TEXT NOT NULL,
    provider_type TEXT NOT NULL CHECK (provider_type IN ('openai', 'anthropic', 'azure_openai', 'llmsim')),
    base_url TEXT,
    -- Encrypted API key (AES-256-GCM): 12-byte nonce || ciphertext || 16-byte tag
    api_key_encrypted BYTEA,
    api_key_set BOOLEAN NOT NULL DEFAULT FALSE,
    -- Provider-specific settings (e.g., Azure deployment_name, api_version)
    settings JSONB NOT NULL DEFAULT '{}'::jsonb,
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'disabled')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_llm_providers_status ON llm_providers(status);
CREATE INDEX idx_llm_providers_provider_type ON llm_providers(provider_type);

CREATE TRIGGER update_llm_providers_updated_at BEFORE UPDATE ON llm_providers
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

COMMENT ON COLUMN llm_providers.settings IS 'Provider-specific settings as JSON. E.g., Azure deployment_name, api_version, etc.';

-- ============================================
-- LLM Models
-- ============================================

CREATE TABLE llm_models (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    provider_id UUID NOT NULL REFERENCES llm_providers(id) ON DELETE CASCADE,
    model_id TEXT NOT NULL,
    display_name TEXT NOT NULL,
    capabilities JSONB NOT NULL DEFAULT '[]'::jsonb,
    is_default BOOLEAN NOT NULL DEFAULT FALSE,
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'disabled')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_llm_models_provider_id ON llm_models(provider_id);
CREATE INDEX idx_llm_models_status ON llm_models(status);
-- Unique model_id per provider
CREATE UNIQUE INDEX idx_llm_models_provider_model ON llm_models(provider_id, model_id);
-- Only one default model globally
CREATE UNIQUE INDEX idx_llm_models_default ON llm_models(is_default) WHERE is_default = TRUE;

CREATE TRIGGER update_llm_models_updated_at BEFORE UPDATE ON llm_models
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

-- ============================================
-- Agents (configuration for agentic loop)
-- ============================================

CREATE TABLE agents (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    name VARCHAR(255) NOT NULL,
    description TEXT,
    system_prompt TEXT NOT NULL,
    default_model_id UUID REFERENCES llm_models(id),
    tags TEXT[] NOT NULL DEFAULT '{}',
    status VARCHAR(50) NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'archived')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_agents_status ON agents(status);
CREATE INDEX idx_agents_tags ON agents USING GIN(tags);

CREATE TRIGGER update_agents_updated_at BEFORE UPDATE ON agents
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

-- ============================================
-- Agent Capabilities (junction table)
-- ============================================

CREATE TABLE agent_capabilities (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    agent_id UUID NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    -- Capability ID validated at application layer via CapabilityRegistry
    capability_id VARCHAR(50) NOT NULL,
    -- Position determines the order in the capability chain (lower = earlier)
    position INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    -- Each agent can have each capability only once
    UNIQUE(agent_id, capability_id)
);

CREATE INDEX idx_agent_capabilities_agent_id ON agent_capabilities(agent_id);
CREATE INDEX idx_agent_capabilities_position ON agent_capabilities(agent_id, position);

-- ============================================
-- Sessions (instance of agentic loop execution)
-- ============================================

CREATE TABLE sessions (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    agent_id UUID NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    title VARCHAR(255),
    tags TEXT[] NOT NULL DEFAULT '{}',
    model_id UUID REFERENCES llm_models(id),
    status VARCHAR(50) NOT NULL DEFAULT 'started' CHECK (status IN ('started', 'active', 'idle')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    started_at TIMESTAMPTZ,
    finished_at TIMESTAMPTZ
);

CREATE INDEX idx_sessions_agent_id ON sessions(agent_id);
CREATE INDEX idx_sessions_status ON sessions(status);
CREATE INDEX idx_sessions_created_at ON sessions(created_at DESC);
CREATE INDEX idx_sessions_tags ON sessions USING GIN(tags);

-- ============================================
-- Events (SSE notification stream + message storage)
-- ============================================
-- Messages are stored as events with type "message.*"
-- This table is the sole source of truth for conversation data

CREATE TABLE events (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    session_id UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    sequence INTEGER NOT NULL,
    event_type VARCHAR(100) NOT NULL,
    data JSONB NOT NULL DEFAULT '{}',
    ts TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    context JSONB NOT NULL DEFAULT '{}',
    metadata JSONB,
    tags TEXT[],
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(session_id, sequence)
);

CREATE INDEX idx_events_session_id ON events(session_id);
CREATE INDEX idx_events_session_sequence ON events(session_id, sequence);
CREATE INDEX idx_events_event_type ON events(event_type);
CREATE INDEX idx_events_tags ON events USING GIN(tags);

-- Partial index for message events (user and agent messages only)
CREATE INDEX idx_events_messages ON events(session_id, sequence)
WHERE event_type IN ('message.user', 'message.agent');

COMMENT ON INDEX idx_events_messages IS 'Partial index for message events (message.user, message.agent)';

-- Partial index for turn lifecycle events
CREATE INDEX idx_events_turns ON events(session_id, sequence)
WHERE event_type IN ('turn.started', 'turn.completed', 'turn.failed');

COMMENT ON INDEX idx_events_turns IS 'Partial index for turn lifecycle events';

-- Partial index for tool execution events (includes results)
CREATE INDEX idx_events_tool_calls ON events(session_id, sequence)
WHERE event_type IN ('tool.call_started', 'tool.call_completed');

COMMENT ON INDEX idx_events_tool_calls IS 'Partial index for tool execution events with results';

COMMENT ON COLUMN events.ts IS 'Event timestamp (when the event occurred)';
COMMENT ON COLUMN events.context IS 'Event correlation context (turn_id, input_message_id, exec_id)';
COMMENT ON COLUMN events.metadata IS 'Arbitrary metadata for the event';
COMMENT ON COLUMN events.tags IS 'Tags for filtering and categorization';

-- ============================================
-- Session Files (virtual filesystem)
-- ============================================

CREATE TABLE session_files (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    session_id UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,

    -- File path (normalized: starts with /, no trailing slash, forward slashes only)
    path TEXT NOT NULL,

    -- Content (NULL for directories)
    content BYTEA,

    -- File type
    is_directory BOOLEAN NOT NULL DEFAULT FALSE,

    -- Metadata
    is_readonly BOOLEAN NOT NULL DEFAULT FALSE,
    size_bytes BIGINT NOT NULL DEFAULT 0,

    -- Timestamps
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- Constraints
    CONSTRAINT session_files_path_check CHECK (
        path ~ '^/([^/\0]+(/[^/\0]+)*)?$' -- Valid path format
    ),
    CONSTRAINT session_files_directory_no_content CHECK (
        NOT is_directory OR content IS NULL -- Directories cannot have content
    )
);

-- Unique path per session
CREATE UNIQUE INDEX idx_session_files_path ON session_files(session_id, path);

-- For listing directory contents (parent path lookup)
CREATE INDEX idx_session_files_parent ON session_files(session_id, (substring(path from '^(.*)/[^/]+$')));

-- For session cleanup
CREATE INDEX idx_session_files_session_id ON session_files(session_id);

-- For searching by name pattern
CREATE INDEX idx_session_files_name ON session_files(session_id, (substring(path from '[^/]+$')));

-- Auto-update updated_at
CREATE TRIGGER update_session_files_updated_at
    BEFORE UPDATE ON session_files
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

-- ============================================
-- Helper Functions
-- ============================================

-- Function to get next event sequence for a session
CREATE OR REPLACE FUNCTION next_event_sequence(p_session_id UUID) RETURNS INTEGER AS $$
DECLARE
    next_seq INTEGER;
BEGIN
    SELECT COALESCE(MAX(sequence), 0) + 1 INTO next_seq
    FROM events WHERE session_id = p_session_id;
    RETURN next_seq;
END;
$$ LANGUAGE plpgsql;

-- Get parent directory path for session files
CREATE OR REPLACE FUNCTION session_files_parent_path(file_path TEXT)
RETURNS TEXT AS $$
BEGIN
    IF file_path = '/' THEN
        RETURN NULL;
    END IF;
    RETURN COALESCE(substring(file_path from '^(.*)/[^/]+$'), '/');
END;
$$ LANGUAGE plpgsql IMMUTABLE;

-- Get file name from path for session files
CREATE OR REPLACE FUNCTION session_files_name(file_path TEXT)
RETURNS TEXT AS $$
BEGIN
    IF file_path = '/' THEN
        RETURN '/';
    END IF;
    RETURN substring(file_path from '[^/]+$');
END;
$$ LANGUAGE plpgsql IMMUTABLE;

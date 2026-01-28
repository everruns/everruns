-- Everruns Base Schema (v0.4.0)
-- Squashed migration - represents final state of all base migrations
--
-- Key design decisions:
-- - UUID v7 for time-ordered, sortable IDs (better for DB performance)
-- - Messages stored as events (single source of truth)
-- - API keys encrypted with AES-256-GCM, key from SECRETS_ENCRYPTION_KEY env var
-- - PostgreSQL 17 with custom uuidv7() function (native support requires PostgreSQL 18+)
-- - Capabilities validated at application layer via CapabilityRegistry (no DB constraint)
-- - Session status: started/active/idle lifecycle
-- - Organization-based multitenancy with default organization

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
-- Organizations (multitenancy)
-- ============================================

CREATE TABLE organizations (
    org_id BIGSERIAL PRIMARY KEY,
    public_id TEXT UNIQUE NOT NULL,
    name TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT organizations_public_id_format
        CHECK (public_id ~ '^org_[0-9a-f]{32}$')
);

CREATE INDEX idx_organizations_public_id ON organizations(public_id);

CREATE TRIGGER update_organizations_updated_at BEFORE UPDATE ON organizations
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

-- Seed default organization (required for FKs)
INSERT INTO organizations (org_id, public_id, name)
VALUES (1, 'org_00000000000000000000000000000001', 'Default Organization');

-- Reset sequence to start after seeded org
SELECT setval('organizations_org_id_seq', 1, true);

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
-- Organization Members
-- ============================================

CREATE TABLE organization_members (
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (org_id, user_id)
);

CREATE INDEX idx_organization_members_user_id ON organization_members(user_id);

-- ============================================
-- API Keys
-- ============================================

CREATE TABLE api_keys (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) DEFAULT 1,
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
CREATE INDEX idx_api_keys_org_id ON api_keys(org_id);
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
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) DEFAULT 1,
    name TEXT NOT NULL,
    provider_type TEXT NOT NULL CHECK (provider_type IN ('openai', 'openai_completions', 'anthropic', 'llmsim')),
    base_url TEXT,
    -- Encrypted API key (AES-256-GCM): 12-byte nonce || ciphertext || 16-byte tag
    api_key_encrypted BYTEA,
    api_key_set BOOLEAN NOT NULL DEFAULT FALSE,
    -- Provider-specific settings
    settings JSONB NOT NULL DEFAULT '{}'::jsonb,
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'disabled')),
    -- Model discovery sync tracking
    last_synced_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_llm_providers_status ON llm_providers(status);
CREATE INDEX idx_llm_providers_provider_type ON llm_providers(provider_type);
CREATE INDEX idx_llm_providers_org_id ON llm_providers(org_id);

CREATE TRIGGER update_llm_providers_updated_at BEFORE UPDATE ON llm_providers
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

COMMENT ON COLUMN llm_providers.settings IS 'Provider-specific settings as JSON.';
COMMENT ON COLUMN llm_providers.last_synced_at IS 'Last time models were synced from provider API';

-- ============================================
-- LLM Models
-- ============================================

CREATE TABLE llm_models (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) DEFAULT 1,
    provider_id UUID NOT NULL REFERENCES llm_providers(id) ON DELETE CASCADE,
    model_id TEXT NOT NULL,
    display_name TEXT NOT NULL,
    capabilities JSONB NOT NULL DEFAULT '[]'::jsonb,
    is_default BOOLEAN NOT NULL DEFAULT FALSE,
    is_favorite BOOLEAN NOT NULL DEFAULT FALSE,
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'disabled')),
    -- Model discovery fields
    source TEXT NOT NULL DEFAULT 'manual' CHECK (source IN ('manual', 'discovered', 'predefined')),
    last_seen_at TIMESTAMPTZ,
    provider_metadata JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_llm_models_provider_id ON llm_models(provider_id);
CREATE INDEX idx_llm_models_status ON llm_models(status);
CREATE INDEX idx_llm_models_org_id ON llm_models(org_id);
-- Unique model_id per provider
CREATE UNIQUE INDEX idx_llm_models_provider_model ON llm_models(provider_id, model_id);
-- Only one default model globally
CREATE UNIQUE INDEX idx_llm_models_default ON llm_models(is_default) WHERE is_default = TRUE;
-- Index for favorite model queries
CREATE INDEX idx_llm_models_favorite ON llm_models(is_favorite) WHERE is_favorite = TRUE;
-- Index for model discovery
CREATE INDEX idx_llm_models_source ON llm_models(source);
CREATE INDEX idx_llm_models_last_seen ON llm_models(last_seen_at) WHERE last_seen_at IS NOT NULL;

CREATE TRIGGER update_llm_models_updated_at BEFORE UPDATE ON llm_models
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

COMMENT ON COLUMN llm_models.is_favorite IS 'Whether this model is marked as a favorite for quick access';
COMMENT ON COLUMN llm_models.source IS 'How the model was added: manual, discovered, or predefined';
COMMENT ON COLUMN llm_models.last_seen_at IS 'Last time model was seen in provider API response';
COMMENT ON COLUMN llm_models.provider_metadata IS 'Raw metadata from provider API response';

-- ============================================
-- Agents (configuration for agentic loop)
-- ============================================

CREATE TABLE agents (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) DEFAULT 1,
    name VARCHAR(255) NOT NULL,
    description TEXT,
    system_prompt TEXT NOT NULL,
    default_model_id UUID REFERENCES llm_models(id),
    tags TEXT[] NOT NULL DEFAULT '{}',
    status VARCHAR(50) NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'archived')),
    -- Denormalized usage totals
    total_input_tokens BIGINT NOT NULL DEFAULT 0,
    total_output_tokens BIGINT NOT NULL DEFAULT 0,
    total_cache_read_tokens BIGINT NOT NULL DEFAULT 0,
    total_cache_creation_tokens BIGINT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_agents_status ON agents(status);
CREATE INDEX idx_agents_tags ON agents USING GIN(tags);
CREATE INDEX idx_agents_org_id ON agents(org_id);

CREATE TRIGGER update_agents_updated_at BEFORE UPDATE ON agents
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

COMMENT ON COLUMN agents.total_input_tokens IS 'Denormalized: sum of input_tokens across all sessions';
COMMENT ON COLUMN agents.total_output_tokens IS 'Denormalized: sum of output_tokens across all sessions';
COMMENT ON COLUMN agents.total_cache_read_tokens IS 'Denormalized: sum of cache_read_tokens across all sessions';
COMMENT ON COLUMN agents.total_cache_creation_tokens IS 'Denormalized: sum of cache_creation_tokens across all sessions';

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
    -- Per-agent capability configuration
    config JSONB NOT NULL DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    -- Each agent can have each capability only once
    UNIQUE(agent_id, capability_id)
);

CREATE INDEX idx_agent_capabilities_agent_id ON agent_capabilities(agent_id);
CREATE INDEX idx_agent_capabilities_position ON agent_capabilities(agent_id, position);

COMMENT ON COLUMN agent_capabilities.config IS 'Per-agent capability configuration (JSON object)';

-- ============================================
-- Sessions (instance of agentic loop execution)
-- ============================================

CREATE TABLE sessions (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    agent_id UUID NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) DEFAULT 1,
    title VARCHAR(255),
    tags TEXT[] NOT NULL DEFAULT '{}',
    model_id UUID REFERENCES llm_models(id),
    status VARCHAR(50) NOT NULL DEFAULT 'started' CHECK (status IN ('started', 'active', 'idle')),
    -- Denormalized usage totals
    total_input_tokens BIGINT NOT NULL DEFAULT 0,
    total_output_tokens BIGINT NOT NULL DEFAULT 0,
    total_cache_read_tokens BIGINT NOT NULL DEFAULT 0,
    total_cache_creation_tokens BIGINT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    started_at TIMESTAMPTZ,
    finished_at TIMESTAMPTZ
);

CREATE INDEX idx_sessions_agent_id ON sessions(agent_id);
CREATE INDEX idx_sessions_org_id ON sessions(org_id);
CREATE INDEX idx_sessions_status ON sessions(status);
CREATE INDEX idx_sessions_created_at ON sessions(created_at DESC);
CREATE INDEX idx_sessions_tags ON sessions USING GIN(tags);

CREATE TRIGGER update_sessions_updated_at BEFORE UPDATE ON sessions
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

COMMENT ON COLUMN sessions.total_input_tokens IS 'Denormalized: sum of input_tokens from llm_generations';
COMMENT ON COLUMN sessions.total_output_tokens IS 'Denormalized: sum of output_tokens from llm_generations';
COMMENT ON COLUMN sessions.total_cache_read_tokens IS 'Denormalized: sum of cache_read_tokens from llm_generations';
COMMENT ON COLUMN sessions.total_cache_creation_tokens IS 'Denormalized: sum of cache_creation_tokens from llm_generations';

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

-- Partial index for message events (input and output messages only)
CREATE INDEX idx_events_messages ON events(session_id, sequence)
WHERE event_type IN ('input.message', 'output.message.completed');

COMMENT ON INDEX idx_events_messages IS 'Partial index for message events (input.message, output.message.completed)';

-- Partial index for turn lifecycle events
CREATE INDEX idx_events_turns ON events(session_id, sequence)
WHERE event_type IN ('turn.started', 'turn.completed', 'turn.failed');

COMMENT ON INDEX idx_events_turns IS 'Partial index for turn lifecycle events';

-- Partial index for tool execution events (includes results)
CREATE INDEX idx_events_tool_calls ON events(session_id, sequence)
WHERE event_type IN ('tool.started', 'tool.completed');

COMMENT ON INDEX idx_events_tool_calls IS 'Partial index for tool execution events with results';

COMMENT ON COLUMN events.ts IS 'Event timestamp (when the event occurred)';
COMMENT ON COLUMN events.context IS 'Event correlation context (turn_id, input_message_id, exec_id)';
COMMENT ON COLUMN events.metadata IS 'Arbitrary metadata for the event';
COMMENT ON COLUMN events.tags IS 'Tags for filtering and categorization';

-- Append-only triggers for events table
CREATE OR REPLACE FUNCTION prevent_event_mutation()
RETURNS TRIGGER AS $$
BEGIN
    RAISE EXCEPTION 'events are append-only: % operations are not allowed', TG_OP;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER events_append_only_update
    BEFORE UPDATE ON events
    FOR EACH ROW EXECUTE FUNCTION prevent_event_mutation();

CREATE TRIGGER events_append_only_delete
    BEFORE DELETE ON events
    FOR EACH ROW EXECUTE FUNCTION prevent_event_mutation();

COMMENT ON FUNCTION prevent_event_mutation() IS 'Enforces append-only semantics on the events table';

-- ============================================
-- Event Sequences (atomic per-session allocation)
-- ============================================

CREATE TABLE event_sequences (
    session_id UUID PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
    next_sequence INTEGER NOT NULL DEFAULT 1,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

COMMENT ON TABLE event_sequences IS 'Per-session sequence counter for atomic event sequence allocation';
COMMENT ON COLUMN event_sequences.next_sequence IS 'Next sequence number to allocate (1-indexed)';

-- Function to atomically allocate the next sequence number
CREATE OR REPLACE FUNCTION allocate_event_sequence(p_session_id UUID)
RETURNS INTEGER AS $$
DECLARE
    allocated_seq INTEGER;
BEGIN
    INSERT INTO event_sequences (session_id, next_sequence, updated_at)
    VALUES (p_session_id, 2, NOW())
    ON CONFLICT (session_id) DO UPDATE
    SET next_sequence = event_sequences.next_sequence + 1,
        updated_at = NOW()
    RETURNING next_sequence - 1 INTO allocated_seq;

    RETURN allocated_seq;
END;
$$ LANGUAGE plpgsql;

COMMENT ON FUNCTION allocate_event_sequence(UUID) IS 'Atomically allocate the next event sequence number for a session';

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

-- ============================================
-- MCP Servers
-- ============================================

CREATE TABLE mcp_servers (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) DEFAULT 1,
    name VARCHAR(255) NOT NULL,
    description TEXT,
    -- URL of the MCP server endpoint (e.g., https://mcp.atlassian.com/v1/mcp)
    url TEXT NOT NULL,
    -- Transport type: currently only 'http' is supported
    transport_type VARCHAR(50) NOT NULL DEFAULT 'http' CHECK (transport_type IN ('http')),
    -- Status for lifecycle management
    status VARCHAR(50) NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'disabled')),
    -- Optional API key for authentication (encrypted)
    api_key_encrypted BYTEA,
    api_key_set BOOLEAN NOT NULL DEFAULT FALSE,
    -- Additional headers as JSON (e.g., for custom authentication)
    headers JSONB NOT NULL DEFAULT '{}'::jsonb,
    -- MCP server-specific settings
    settings JSONB NOT NULL DEFAULT '{}'::jsonb,
    -- Cached tool definitions
    cached_tools JSONB NOT NULL DEFAULT '[]'::jsonb,
    tools_cached_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Unique constraint on name to prevent duplicates
CREATE UNIQUE INDEX idx_mcp_servers_name ON mcp_servers(name);
CREATE INDEX idx_mcp_servers_status ON mcp_servers(status);
CREATE INDEX idx_mcp_servers_org_id ON mcp_servers(org_id);

CREATE TRIGGER update_mcp_servers_updated_at BEFORE UPDATE ON mcp_servers
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

COMMENT ON COLUMN mcp_servers.cached_tools IS 'Cached tool definitions from MCP server tools/list endpoint';
COMMENT ON COLUMN mcp_servers.tools_cached_at IS 'Timestamp when tools were last fetched from the MCP server';

-- ============================================
-- LLM Generations (usage tracking)
-- ============================================

CREATE TABLE llm_generations (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) DEFAULT 1,
    session_id UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    turn_id UUID,  -- Optional: links to turn for grouping
    event_id UUID, -- Optional: links to the original event

    -- Model info
    model TEXT,
    provider TEXT,

    -- Token usage (source of truth)
    input_tokens BIGINT NOT NULL DEFAULT 0,
    output_tokens BIGINT NOT NULL DEFAULT 0,
    cache_read_tokens BIGINT NOT NULL DEFAULT 0,
    cache_creation_tokens BIGINT NOT NULL DEFAULT 0,

    -- Metadata
    duration_ms INTEGER,
    finish_reason TEXT,

    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Indexes for efficient querying
CREATE INDEX idx_llm_generations_session_id ON llm_generations(session_id);
CREATE INDEX idx_llm_generations_turn_id ON llm_generations(turn_id) WHERE turn_id IS NOT NULL;
CREATE INDEX idx_llm_generations_created_at ON llm_generations(created_at);
CREATE UNIQUE INDEX idx_llm_generations_event_id ON llm_generations(event_id) WHERE event_id IS NOT NULL;
CREATE INDEX idx_llm_generations_org_id ON llm_generations(org_id);
CREATE INDEX idx_llm_generations_org_created ON llm_generations(org_id, created_at);

COMMENT ON TABLE llm_generations IS 'Source of truth for all LLM generation calls and their token usage';
COMMENT ON COLUMN llm_generations.event_id IS 'Reference to original event (for traceability)';
COMMENT ON COLUMN llm_generations.turn_id IS 'Groups generations within a turn';

-- ============================================
-- Images
-- ============================================

CREATE TABLE images (
    id UUID PRIMARY KEY DEFAULT uuidv7(),

    -- Original file information
    filename TEXT NOT NULL,
    content_type TEXT NOT NULL,
    size_bytes BIGINT NOT NULL,

    -- Binary data (original and thumbnail)
    data BYTEA NOT NULL,
    thumbnail_data BYTEA,
    thumbnail_content_type TEXT,

    -- Metadata for tracking (includes session_id if uploaded from a session)
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,

    -- Timestamps
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Index for listing images by creation date
CREATE INDEX idx_images_created_at ON images(created_at DESC);

-- Index for filtering by session_id in metadata
CREATE INDEX idx_images_session_id ON images((metadata->>'session_id'))
    WHERE metadata->>'session_id' IS NOT NULL;

COMMENT ON TABLE images IS 'Global image storage for message attachments';
COMMENT ON COLUMN images.content_type IS 'MIME type: image/png, image/jpeg, image/gif, image/webp';
COMMENT ON COLUMN images.metadata IS 'JSON metadata including optional session_id for tracking';
COMMENT ON COLUMN images.thumbnail_data IS 'Thumbnail image for efficient display (max 200x200)';

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

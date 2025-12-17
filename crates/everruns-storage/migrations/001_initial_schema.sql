-- Everruns M2 Schema (v0.2.0)
-- Decision: Use UUID v7 for time-ordered, sortable IDs (better for DB performance)
-- Decision: Messages are PRIMARY data store, Events are SSE notifications only
-- Decision: API keys encrypted with AES-256-GCM, key from SECRETS_ENCRYPTION_KEY env var
-- PostgreSQL 18+ has native UUIDv7 support via uuidv7()

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
-- Users (for future auth implementation)
-- ============================================

CREATE TABLE users (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    email TEXT NOT NULL,
    name TEXT NOT NULL,
    avatar_url TEXT,
    roles JSONB NOT NULL DEFAULT '[]'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE UNIQUE INDEX idx_users_email ON users(email);

CREATE TRIGGER update_users_updated_at BEFORE UPDATE ON users
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

-- ============================================
-- LLM Providers
-- ============================================

CREATE TABLE llm_providers (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    name TEXT NOT NULL,
    provider_type TEXT NOT NULL CHECK (provider_type IN ('openai', 'anthropic', 'azure_openai', 'ollama', 'custom')),
    base_url TEXT,
    -- Encrypted API key (AES-256-GCM): 12-byte nonce || ciphertext || 16-byte tag
    api_key_encrypted BYTEA,
    api_key_set BOOLEAN NOT NULL DEFAULT FALSE,
    is_default BOOLEAN NOT NULL DEFAULT FALSE,
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'disabled')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_llm_providers_status ON llm_providers(status);
CREATE INDEX idx_llm_providers_provider_type ON llm_providers(provider_type);
-- Ensure only one default provider
CREATE UNIQUE INDEX idx_llm_providers_default ON llm_providers(is_default) WHERE is_default = TRUE;

CREATE TRIGGER update_llm_providers_updated_at BEFORE UPDATE ON llm_providers
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

-- ============================================
-- LLM Models
-- ============================================

CREATE TABLE llm_models (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    provider_id UUID NOT NULL REFERENCES llm_providers(id) ON DELETE CASCADE,
    model_id TEXT NOT NULL,
    display_name TEXT NOT NULL,
    capabilities JSONB NOT NULL DEFAULT '[]'::jsonb,
    context_window INTEGER,
    is_default BOOLEAN NOT NULL DEFAULT FALSE,
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'disabled')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_llm_models_provider_id ON llm_models(provider_id);
CREATE INDEX idx_llm_models_status ON llm_models(status);
-- Unique model_id per provider
CREATE UNIQUE INDEX idx_llm_models_provider_model ON llm_models(provider_id, model_id);
-- Ensure only one default model per provider
CREATE UNIQUE INDEX idx_llm_models_provider_default ON llm_models(provider_id, is_default) WHERE is_default = TRUE;

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
-- Sessions (instance of agentic loop execution)
-- ============================================

CREATE TABLE sessions (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    agent_id UUID NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    title VARCHAR(255),
    tags TEXT[] NOT NULL DEFAULT '{}',
    model_id UUID REFERENCES llm_models(id),
    status VARCHAR(50) NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'running', 'completed', 'failed')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    started_at TIMESTAMPTZ,
    finished_at TIMESTAMPTZ
);

CREATE INDEX idx_sessions_agent_id ON sessions(agent_id);
CREATE INDEX idx_sessions_status ON sessions(status);
CREATE INDEX idx_sessions_created_at ON sessions(created_at DESC);
CREATE INDEX idx_sessions_tags ON sessions USING GIN(tags);

-- ============================================
-- Messages (PRIMARY conversation data)
-- ============================================

CREATE TABLE messages (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    session_id UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    sequence INTEGER NOT NULL,
    role VARCHAR(50) NOT NULL CHECK (role IN ('user', 'assistant', 'tool_call', 'tool_result', 'system')),
    content JSONB NOT NULL,
    tool_call_id VARCHAR(255), -- For tool_result, references the tool_call id
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(session_id, sequence)
);

CREATE INDEX idx_messages_session_id ON messages(session_id);
CREATE INDEX idx_messages_session_sequence ON messages(session_id, sequence);
CREATE INDEX idx_messages_role ON messages(role);

-- ============================================
-- Events (SSE notification stream for UI)
-- ============================================

CREATE TABLE events (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    session_id UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    sequence INTEGER NOT NULL,
    event_type VARCHAR(100) NOT NULL,
    data JSONB NOT NULL DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(session_id, sequence)
);

CREATE INDEX idx_events_session_id ON events(session_id);
CREATE INDEX idx_events_session_sequence ON events(session_id, sequence);
CREATE INDEX idx_events_event_type ON events(event_type);

-- ============================================
-- Sequence functions for auto-increment within session
-- ============================================

-- Function to get next message sequence for a session
CREATE OR REPLACE FUNCTION next_message_sequence(p_session_id UUID) RETURNS INTEGER AS $$
DECLARE
    next_seq INTEGER;
BEGIN
    SELECT COALESCE(MAX(sequence), 0) + 1 INTO next_seq
    FROM messages WHERE session_id = p_session_id;
    RETURN next_seq;
END;
$$ LANGUAGE plpgsql;

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

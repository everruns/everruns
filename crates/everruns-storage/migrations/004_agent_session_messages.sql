-- M2 Revised: Agent/Session/Messages/Events Model
-- This migration corrects the data model:
--   - Messages are the PRIMARY data store (user, assistant, tool_call, tool_result)
--   - Events are SSE notifications for real-time UI updates (NOT primary data)

-- ============================================
-- Drop M2 tables (from migration 003)
-- ============================================
DROP TABLE IF EXISTS session_actions CASCADE;
DROP TABLE IF EXISTS session_events CASCADE;
DROP TABLE IF EXISTS agent_sessions CASCADE;
DROP TABLE IF EXISTS harnesses CASCADE;

-- ============================================
-- New tables for Agent/Session/Messages/Events model
-- ============================================

-- Agents table (configuration for agentic loop)
CREATE TABLE agents (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    slug VARCHAR(255) NOT NULL UNIQUE,
    name VARCHAR(255) NOT NULL,
    description TEXT,
    system_prompt TEXT NOT NULL,
    default_model_id UUID REFERENCES llm_models(id),
    temperature REAL,
    max_tokens INTEGER,
    tools JSONB NOT NULL DEFAULT '[]',
    tags TEXT[] NOT NULL DEFAULT '{}',
    status VARCHAR(50) NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'archived')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_agents_slug ON agents(slug);
CREATE INDEX idx_agents_status ON agents(status);
CREATE INDEX idx_agents_tags ON agents USING GIN(tags);

-- Apply updated_at trigger
CREATE TRIGGER update_agents_updated_at BEFORE UPDATE ON agents
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

-- Sessions table (instance of agentic loop execution)
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

-- Messages table (PRIMARY conversation data)
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

-- Events table (SSE notification stream for UI)
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

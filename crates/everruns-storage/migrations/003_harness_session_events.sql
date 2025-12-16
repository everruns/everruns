-- M2: Harness/Session/Events Model Refactoring
-- This migration introduces a new data model that better represents agentic loop concepts:
--   - Harness (replaces Agent): Setup for agentic loop (system prompt, slug, display name, model, tools, budgets)
--   - Session (replaces Thread + Run): Instance of agentic loop execution
--   - Event (replaces Messages + RunEvents): All operations in a session

-- ============================================
-- New tables for Harness/Session/Events model
-- ============================================

-- Harnesses table (replaces agents)
CREATE TABLE harnesses (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    slug VARCHAR(255) NOT NULL UNIQUE,
    display_name VARCHAR(255) NOT NULL,
    description TEXT,
    system_prompt TEXT NOT NULL,
    default_model_id UUID REFERENCES llm_models(id),
    temperature REAL,
    max_tokens INTEGER,
    tags TEXT[] NOT NULL DEFAULT '{}',
    status VARCHAR(50) NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'archived')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_harnesses_slug ON harnesses(slug);
CREATE INDEX idx_harnesses_status ON harnesses(status);
CREATE INDEX idx_harnesses_tags ON harnesses USING GIN(tags);

-- Sessions table (replaces threads + runs)
CREATE TABLE agent_sessions (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    harness_id UUID NOT NULL REFERENCES harnesses(id) ON DELETE CASCADE,
    title VARCHAR(255),
    tags TEXT[] NOT NULL DEFAULT '{}',
    model_id UUID REFERENCES llm_models(id),
    -- Temporal workflow tracking (if using Temporal runner)
    temporal_workflow_id VARCHAR(255),
    temporal_run_id VARCHAR(255),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    started_at TIMESTAMPTZ,
    finished_at TIMESTAMPTZ
);

CREATE INDEX idx_agent_sessions_harness_id ON agent_sessions(harness_id);
CREATE INDEX idx_agent_sessions_created_at ON agent_sessions(created_at DESC);
CREATE INDEX idx_agent_sessions_tags ON agent_sessions USING GIN(tags);
CREATE UNIQUE INDEX idx_agent_sessions_temporal_workflow_id
    ON agent_sessions(temporal_workflow_id) WHERE temporal_workflow_id IS NOT NULL;

-- Events table (replaces messages + run_events)
CREATE TABLE session_events (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    session_id UUID NOT NULL REFERENCES agent_sessions(id) ON DELETE CASCADE,
    sequence INTEGER NOT NULL,
    event_type VARCHAR(100) NOT NULL,
    data JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(session_id, sequence)
);

CREATE INDEX idx_session_events_session_id ON session_events(session_id);
CREATE INDEX idx_session_events_session_sequence ON session_events(session_id, sequence);
CREATE INDEX idx_session_events_event_type ON session_events(event_type);

-- Session actions table (replaces actions)
CREATE TABLE session_actions (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    session_id UUID NOT NULL REFERENCES agent_sessions(id) ON DELETE CASCADE,
    kind VARCHAR(50) NOT NULL,
    payload JSONB NOT NULL DEFAULT '{}',
    by_user_id UUID,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_session_actions_session_id ON session_actions(session_id);

-- Apply updated_at trigger to harnesses
CREATE TRIGGER update_harnesses_updated_at BEFORE UPDATE ON harnesses
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

-- ============================================
-- Drop legacy tables
-- ============================================
DROP TABLE IF EXISTS actions CASCADE;
DROP TABLE IF EXISTS run_events CASCADE;
DROP TABLE IF EXISTS runs CASCADE;
DROP TABLE IF EXISTS messages CASCADE;
DROP TABLE IF EXISTS threads CASCADE;
DROP TABLE IF EXISTS agents CASCADE;

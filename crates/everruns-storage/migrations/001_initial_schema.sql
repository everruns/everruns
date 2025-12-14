-- Decision: Use UUID v7 for time-ordered, sortable IDs (better for DB performance)
-- Decision: Multi-tenancy removed for MVP - will be added in future milestone
-- PostgreSQL 18+ has native UUIDv7 support via uuidv7()

-- Users table (for future auth implementation)
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

-- Sessions table (for future auth implementation)
CREATE TABLE sessions (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token TEXT NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE UNIQUE INDEX idx_sessions_token ON sessions(token);
CREATE INDEX idx_sessions_user_id ON sessions(user_id);
CREATE INDEX idx_sessions_expires_at ON sessions(expires_at);

-- Agents table
CREATE TABLE agents (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    name TEXT NOT NULL,
    description TEXT,
    default_model_id TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'disabled')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_agents_status ON agents(status);

-- Agent versions table (immutable)
CREATE TABLE agent_versions (
    agent_id UUID NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    version INTEGER NOT NULL,
    definition JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (agent_id, version)
);

CREATE INDEX idx_agent_versions_created_at ON agent_versions(created_at);

-- Threads table
CREATE TABLE threads (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_threads_created_at ON threads(created_at);

-- Messages table
CREATE TABLE messages (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    thread_id UUID NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    role TEXT NOT NULL CHECK (role IN ('user', 'assistant', 'system')),
    content TEXT NOT NULL,
    metadata JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_messages_thread_id ON messages(thread_id);
CREATE INDEX idx_messages_created_at ON messages(created_at);

-- Runs table
CREATE TABLE runs (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    agent_id UUID NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    agent_version INTEGER NOT NULL,
    thread_id UUID NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'running', 'completed', 'failed', 'cancelled')),
    -- Internal Temporal workflow ID (never exposed in API)
    temporal_workflow_id TEXT,
    temporal_run_id TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    started_at TIMESTAMPTZ,
    finished_at TIMESTAMPTZ,
    FOREIGN KEY (agent_id, agent_version) REFERENCES agent_versions(agent_id, version)
);

CREATE INDEX idx_runs_agent_id ON runs(agent_id);
CREATE INDEX idx_runs_thread_id ON runs(thread_id);
CREATE INDEX idx_runs_status ON runs(status);
CREATE INDEX idx_runs_created_at ON runs(created_at);
CREATE UNIQUE INDEX idx_runs_temporal_workflow_id ON runs(temporal_workflow_id) WHERE temporal_workflow_id IS NOT NULL;

-- Actions table (HITL and control)
CREATE TABLE actions (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    run_id UUID NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('approve', 'deny', 'resume', 'cancel')),
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    by_user_id UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_actions_run_id ON actions(run_id);
CREATE INDEX idx_actions_created_at ON actions(created_at);

-- Run events table (AG-UI event stream)
CREATE TABLE run_events (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    run_id UUID NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    sequence_number BIGSERIAL,
    event_type TEXT NOT NULL,
    event_data JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_run_events_run_id ON run_events(run_id);
CREATE INDEX idx_run_events_sequence_number ON run_events(sequence_number);
CREATE UNIQUE INDEX idx_run_events_run_sequence ON run_events(run_id, sequence_number);

-- Updated_at trigger function
CREATE OR REPLACE FUNCTION update_updated_at_column()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- Apply updated_at triggers
CREATE TRIGGER update_users_updated_at BEFORE UPDATE ON users
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

CREATE TRIGGER update_agents_updated_at BEFORE UPDATE ON agents
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

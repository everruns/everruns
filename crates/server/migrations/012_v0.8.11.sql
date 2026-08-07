-- v0.8.11: Session resource registry
-- Squashed from: 012_session_resources.sql

-- ============================================
-- 012_session_resources: Session resource registry
-- Generic, session-scoped registry of active resources (sandboxes, subagents,
-- browser sessions, etc.). See specs/session-resources.md.
-- ============================================

CREATE TABLE session_resources (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    session_id UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    resource_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    display_name TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'active'
        CHECK (status IN ('active', 'completed', 'failed', 'released')),
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(session_id, resource_id)
);

CREATE INDEX idx_session_resources_session_status
    ON session_resources (session_id, status);

-- Auto-update updated_at
CREATE TRIGGER update_session_resources_updated_at
    BEFORE UPDATE ON session_resources
    FOR EACH ROW
    EXECUTE FUNCTION update_updated_at_column();

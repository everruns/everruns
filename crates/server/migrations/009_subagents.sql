-- Everruns: Subagent support
--
-- Adds parent/child relationship to sessions for subagent delegation.
-- A subagent is a child session spawned by a parent session's tool call.
--
-- Design decisions:
-- - Subagent metadata lives on the sessions table (not a separate table)
--   because a subagent IS a session with extra context.
-- - subagent_results is a separate table for clean querying of results
--   without scanning all sessions.
-- - Names are human-readable ("Test Runner"), unique per parent session.

-- ============================================
-- Subagent columns on sessions
-- ============================================

ALTER TABLE sessions ADD COLUMN parent_session_id UUID REFERENCES sessions(id) ON DELETE CASCADE;
ALTER TABLE sessions ADD COLUMN subagent_name TEXT;
ALTER TABLE sessions ADD COLUMN subagent_task TEXT;
ALTER TABLE sessions ADD COLUMN subagent_config JSONB;
ALTER TABLE sessions ADD COLUMN subagent_status VARCHAR(50)
    CHECK (subagent_status IS NULL OR subagent_status IN (
        'spawning', 'running', 'completed', 'failed', 'cancelled', 'max_iterations_reached'
    ));

-- Unique name within parent session (only for subagent sessions)
CREATE UNIQUE INDEX idx_subagent_name_per_parent
    ON sessions (parent_session_id, subagent_name)
    WHERE parent_session_id IS NOT NULL;

-- Fast lookup: all subagents of a parent
CREATE INDEX idx_sessions_parent_session_id
    ON sessions (parent_session_id)
    WHERE parent_session_id IS NOT NULL;

COMMENT ON COLUMN sessions.parent_session_id IS 'Parent session that spawned this subagent (NULL for top-level sessions)';
COMMENT ON COLUMN sessions.subagent_name IS 'Human-readable subagent name, unique per parent ("Test Runner")';
COMMENT ON COLUMN sessions.subagent_task IS 'Original task description given to the subagent';
COMMENT ON COLUMN sessions.subagent_config IS 'Inline config: allowed_tools, model, filesystem_mode, etc.';
COMMENT ON COLUMN sessions.subagent_status IS 'Subagent lifecycle: spawning→running→completed/failed/cancelled';

-- ============================================
-- Subagent results (denormalized for querying)
-- ============================================

CREATE TABLE subagent_results (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    parent_session_id UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    subagent_session_id UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    task TEXT NOT NULL,
    status VARCHAR(50) NOT NULL,
    result TEXT,
    iterations INTEGER DEFAULT 0,
    tool_calls_count INTEGER DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at TIMESTAMPTZ
);

CREATE INDEX idx_subagent_results_parent ON subagent_results(parent_session_id);
CREATE INDEX idx_subagent_results_status ON subagent_results(parent_session_id, status);

COMMENT ON TABLE subagent_results IS 'Denormalized subagent completion records for fast parent queries';

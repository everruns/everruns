-- Session schedules: agent-scheduled future tasks within a session
--
-- Allows agents to schedule work at a specific time. When the time arrives,
-- an "app" role message is injected into the session and triggers a new turn.

CREATE TABLE session_schedules (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    session_id UUID NOT NULL,
    organization_id TEXT NOT NULL,
    harness_id TEXT NOT NULL,
    agent_id TEXT,
    description TEXT NOT NULL,
    scheduled_at TIMESTAMPTZ NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'triggered', 'cancelled', 'failed')),
    triggered_at TIMESTAMPTZ,
    message_id UUID,
    error TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Index for poller: find pending schedules that are due
CREATE INDEX idx_session_schedules_pending
    ON session_schedules (scheduled_at)
    WHERE status = 'pending';

-- Index for listing schedules by session
CREATE INDEX idx_session_schedules_session
    ON session_schedules (session_id, created_at DESC);

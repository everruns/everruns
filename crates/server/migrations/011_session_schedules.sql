-- Session-scoped schedules
-- Allows agents to schedule future work within a session.
-- When a schedule fires, a message is injected into the session triggering a turn.

CREATE TABLE session_schedules (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    public_id TEXT NOT NULL,
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    session_id UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    description TEXT NOT NULL,
    cron_expression TEXT,
    scheduled_at TIMESTAMPTZ,
    timezone TEXT NOT NULL DEFAULT 'UTC',
    enabled BOOLEAN NOT NULL DEFAULT true,
    next_trigger_at TIMESTAMPTZ,
    last_triggered_at TIMESTAMPTZ,
    trigger_count INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT session_schedules_has_schedule CHECK (
        cron_expression IS NOT NULL OR scheduled_at IS NOT NULL
    ),
    CONSTRAINT session_schedules_public_id_format CHECK (
        public_id ~ '^sched_[0-9a-f]{32}$'
    )
);

-- Unique public_id per org
CREATE UNIQUE INDEX idx_session_schedules_org_public_id
    ON session_schedules(org_id, public_id);

-- Scheduler polling: find due schedules efficiently
CREATE INDEX idx_session_schedules_polling
    ON session_schedules (next_trigger_at)
    WHERE enabled = true AND next_trigger_at IS NOT NULL;

-- List schedules for a session
CREATE INDEX idx_session_schedules_session
    ON session_schedules (session_id, created_at DESC);

-- Count active schedules per session (for max-5 enforcement)
CREATE INDEX idx_session_schedules_active_count
    ON session_schedules (session_id)
    WHERE enabled = true;

-- Session tasks — unified registry of background work owned by a session.
-- See specs/session-tasks.md. Replaces the work half of session_resources
-- (subagents, agent runs, background runs); leased infrastructure stays in
-- session_resources.

CREATE TABLE session_tasks (
    id                  TEXT        PRIMARY KEY,
    session_id          UUID        NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    kind                TEXT        NOT NULL,
    display_name        TEXT        NOT NULL DEFAULT '',
    spec                JSONB       NOT NULL DEFAULT '{}',
    state               TEXT        NOT NULL DEFAULT 'queued'
                        CHECK (state IN ('queued', 'running', 'awaiting_input',
                                         'succeeded', 'failed', 'canceled')),
    state_detail        TEXT,
    progress            JSONB,
    input_request       JSONB,
    cancel_requested_at TIMESTAMPTZ,
    summary             TEXT,
    result_path         TEXT,
    artifacts           JSONB       NOT NULL DEFAULT '[]',
    error               JSONB,
    attempt             INTEGER     NOT NULL DEFAULT 1,
    worker_id           TEXT,
    heartbeat_at        TIMESTAMPTZ,
    links               JSONB       NOT NULL DEFAULT '{}',
    wake_policy         TEXT        NOT NULL DEFAULT 'silent',
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    started_at          TIMESTAMPTZ,
    finished_at         TIMESTAMPTZ,
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

COMMENT ON TABLE session_tasks IS
    'Background work owned by a session (subagents, external agents, background tools). '
    'One uniform lifecycle per specs/session-tasks.md.';

CREATE INDEX session_tasks_session_state_idx ON session_tasks (session_id, state);

-- Bidirectional persisted message channel between a session and its tasks.
CREATE TABLE session_task_messages (
    id          TEXT        PRIMARY KEY,
    task_id     TEXT        NOT NULL REFERENCES session_tasks(id) ON DELETE CASCADE,
    session_id  UUID        NOT NULL,
    direction   TEXT        NOT NULL CHECK (direction IN ('inbound', 'outbound')),
    content     JSONB       NOT NULL DEFAULT '[]',
    in_reply_to TEXT,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX session_task_messages_task_idx ON session_task_messages (task_id, created_at);

-- Backfill: work-shaped session_resources rows become tasks. The resource
-- registry narrows back to leased infrastructure (sandboxes, browser
-- sessions, ...). Task IDs derive from the existing row UUID so re-running
-- against a restored dump stays deterministic.
INSERT INTO session_tasks (
    id, session_id, kind, display_name, spec, state,
    created_at, started_at, finished_at, updated_at
)
SELECT
    'task_' || replace(sr.id::text, '-', ''),
    sr.session_id,
    CASE sr.kind
        WHEN 'agent_run' THEN 'external_agent'
        WHEN 'background_run' THEN 'background_tool'
        WHEN 'agent_handoff' THEN 'subagent'
        ELSE 'subagent'
    END,
    sr.display_name,
    sr.metadata,
    CASE sr.status
        WHEN 'active' THEN 'running'
        WHEN 'completed' THEN 'succeeded'
        WHEN 'failed' THEN 'failed'
        WHEN 'released' THEN 'canceled'
        ELSE 'running'
    END,
    sr.created_at,
    sr.created_at,
    CASE WHEN sr.status IN ('completed', 'failed', 'released') THEN sr.updated_at END,
    sr.updated_at
FROM session_resources sr
WHERE sr.kind IN ('subagent', 'agent_run', 'background_run', 'agent_handoff');

DELETE FROM session_resources
WHERE kind IN ('subagent', 'agent_run', 'background_run', 'agent_handoff');

-- Denormalized subagent completion records fold into session_tasks.
DROP TABLE IF EXISTS subagent_results;

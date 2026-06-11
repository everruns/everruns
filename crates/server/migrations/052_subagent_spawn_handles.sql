-- Durable spawn handles link a parent tool call to a child session (EVE-535).
--
-- Keyed by (parent_session_id, tool_call_id) — one row per unique spawn.
-- The UNIQUE constraint is the CAS primitive: first INSERT wins (claim),
-- subsequent ones conflict and reveal the existing child_session_id so
-- callers can reattach instead of spawning a duplicate.
--
-- Lifecycle:
--   pending  — claimed; child session not yet created (register_child_session pending)
--   running  — child session exists; waiting for completion
--   settled  — child finished; terminal_result and terminal_status are populated
--
-- child_session_id is nullable and has NO FK: the claim INSERT happens before
-- the child session row exists. register_child_session sets it after creation.
CREATE TABLE subagent_spawn_handles (
    id                 UUID        PRIMARY KEY DEFAULT uuidv7(),
    parent_session_id  UUID        NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    tool_call_id       TEXT        NOT NULL,
    child_session_id   UUID,
    status             TEXT        NOT NULL DEFAULT 'pending'
                       CHECK (status IN ('pending', 'running', 'settled')),
    -- Last assistant message from child, stored as a JSON-encoded string.
    terminal_result    TEXT,
    -- wait_for_idle return value ("idle", "error", "timeout", etc.)
    terminal_status    TEXT,
    subagent_name      TEXT        NOT NULL,
    subagent_task      TEXT        NOT NULL,
    -- Opaque token generated at claim time; settle CAS verifies this matches.
    claim_token        UUID        NOT NULL DEFAULT uuidv7(),
    created_at         TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    settled_at         TIMESTAMPTZ,
    CONSTRAINT subagent_spawn_handles_parent_call_uq
        UNIQUE (parent_session_id, tool_call_id)
);

COMMENT ON TABLE subagent_spawn_handles IS
    'Durable spawn handles linking parent tool calls to child sessions (EVE-535). '
    'Prevents duplicate subagent spawning on parent worker reclaim/replay.';

CREATE INDEX idx_spawn_handles_parent
    ON subagent_spawn_handles (parent_session_id);

CREATE INDEX idx_spawn_handles_child
    ON subagent_spawn_handles (child_session_id)
    WHERE child_session_id IS NOT NULL;

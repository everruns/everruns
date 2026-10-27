-- Tool calls a worker took off their turn (knowledge/execution/detached-tool-calls.md).
-- A live row keeps its session's actor from being leased; its holder renews
-- it, and an expired row makes the actor runnable again.
CREATE TABLE detached_tool_calls (
    session_id UUID NOT NULL REFERENCES session_actors (session_id) ON DELETE CASCADE,
    tool_call_id TEXT NOT NULL,
    org_id BIGINT NOT NULL,
    turn_id UUID NOT NULL,
    holder TEXT NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (session_id, tool_call_id)
);

CREATE INDEX idx_detached_tool_calls_expires_at ON detached_tool_calls (expires_at);
CREATE INDEX idx_detached_tool_calls_holder ON detached_tool_calls (holder);

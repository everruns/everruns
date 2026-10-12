-- Session trace index: one row per turn and one per step, projected from events.
--
-- Decision: the Trace view reads these small rows instead of deriving turns and
-- steps from raw events, whose payloads (full prompts on llm.generation, full
-- results on tool.completed) make a per-turn read cost megabytes. Every row is
-- rebuildable from the session's events; session_trace_state records how far
-- the projection has caught up. See knowledge/ui/session-trace.md.

CREATE TABLE session_trace_state (
    session_id UUID PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
    -- Highest event sequence the projection has applied.
    projected_sequence INTEGER NOT NULL DEFAULT 0,
    turn_count INTEGER NOT NULL DEFAULT 0,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE session_trace_turns (
    session_id UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    -- Per-session ordinal, 1-based, in turn start order.
    turn_no INTEGER NOT NULL,
    turn_id UUID NOT NULL,
    start_sequence INTEGER NOT NULL,
    end_sequence INTEGER,
    started_at TIMESTAMPTZ NOT NULL,
    ended_at TIMESTAMPTZ,
    status TEXT NOT NULL,
    prompt_preview TEXT,
    error TEXT,
    step_count INTEGER NOT NULL DEFAULT 0,
    model_calls INTEGER NOT NULL DEFAULT 0,
    tool_calls INTEGER NOT NULL DEFAULT 0,
    subagent_calls INTEGER NOT NULL DEFAULT 0,
    error_count INTEGER NOT NULL DEFAULT 0,
    input_tokens BIGINT NOT NULL DEFAULT 0,
    output_tokens BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY (session_id, turn_no)
);

CREATE UNIQUE INDEX idx_session_trace_turns_turn_id
    ON session_trace_turns (session_id, turn_id);
-- Maps an event (search hit, deep link) to the turn that contains it.
CREATE INDEX idx_session_trace_turns_start
    ON session_trace_turns (session_id, start_sequence);
CREATE INDEX idx_session_trace_turns_errors
    ON session_trace_turns (session_id, turn_no) WHERE error_count > 0;

CREATE TABLE session_trace_steps (
    session_id UUID NOT NULL,
    turn_no INTEGER NOT NULL,
    -- Per-turn ordinal, 1-based, in start order.
    step_no INTEGER NOT NULL,
    kind TEXT NOT NULL,
    status TEXT NOT NULL,
    start_sequence INTEGER NOT NULL,
    end_sequence INTEGER,
    started_at TIMESTAMPTZ NOT NULL,
    duration_ms BIGINT,
    name TEXT,
    target TEXT,
    result TEXT,
    narration TEXT,
    tool_call_id TEXT,
    model TEXT,
    input_tokens INTEGER,
    output_tokens INTEGER,
    message_count INTEGER,
    requested_tool_call_ids TEXT[] NOT NULL DEFAULT '{}',
    PRIMARY KEY (session_id, turn_no, step_no),
    FOREIGN KEY (session_id, turn_no)
        REFERENCES session_trace_turns (session_id, turn_no) ON DELETE CASCADE
);

-- Pairs tool.completed with the step tool.started opened.
CREATE INDEX idx_session_trace_steps_tool_call
    ON session_trace_steps (session_id, tool_call_id) WHERE tool_call_id IS NOT NULL;
CREATE INDEX idx_session_trace_steps_errors
    ON session_trace_steps (session_id, turn_no, step_no) WHERE status = 'error';

-- Migration: Add LLM generation tracking with usage statistics
--
-- Architecture:
-- 1. llm_generations table - SOURCE OF TRUTH for all LLM calls
-- 2. Denormalized totals on sessions/agents - for fast queries
--
-- The llm_generations table stores every LLM call, allowing totals to be
-- recalculated if needed.

-- ============================================================================
-- 1. Create llm_generations table (SOURCE OF TRUTH)
-- ============================================================================

CREATE TABLE IF NOT EXISTS llm_generations (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    session_id UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    turn_id UUID,  -- Optional: links to turn for grouping
    event_id UUID, -- Optional: links to the original event

    -- Model info
    model TEXT,
    provider TEXT,

    -- Token usage (source of truth)
    input_tokens BIGINT NOT NULL DEFAULT 0,
    output_tokens BIGINT NOT NULL DEFAULT 0,
    cache_read_tokens BIGINT NOT NULL DEFAULT 0,
    cache_creation_tokens BIGINT NOT NULL DEFAULT 0,

    -- Metadata
    duration_ms INTEGER,
    finish_reason TEXT,

    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Indexes for efficient querying
CREATE INDEX IF NOT EXISTS idx_llm_generations_session_id ON llm_generations(session_id);
CREATE INDEX IF NOT EXISTS idx_llm_generations_turn_id ON llm_generations(turn_id) WHERE turn_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_llm_generations_created_at ON llm_generations(created_at);

-- ============================================================================
-- 2. Add denormalized usage columns to sessions and agents (for fast queries)
-- ============================================================================

ALTER TABLE sessions
ADD COLUMN IF NOT EXISTS total_input_tokens BIGINT DEFAULT 0,
ADD COLUMN IF NOT EXISTS total_output_tokens BIGINT DEFAULT 0,
ADD COLUMN IF NOT EXISTS total_cache_read_tokens BIGINT DEFAULT 0,
ADD COLUMN IF NOT EXISTS total_cache_creation_tokens BIGINT DEFAULT 0;

ALTER TABLE agents
ADD COLUMN IF NOT EXISTS total_input_tokens BIGINT DEFAULT 0,
ADD COLUMN IF NOT EXISTS total_output_tokens BIGINT DEFAULT 0,
ADD COLUMN IF NOT EXISTS total_cache_read_tokens BIGINT DEFAULT 0,
ADD COLUMN IF NOT EXISTS total_cache_creation_tokens BIGINT DEFAULT 0;

-- ============================================================================
-- 3. Trigger: Insert llm_generation AND update denormalized totals
-- ============================================================================

CREATE OR REPLACE FUNCTION process_llm_generation_event()
RETURNS TRIGGER AS $$
DECLARE
    v_input_tokens BIGINT;
    v_output_tokens BIGINT;
    v_cache_read_tokens BIGINT;
    v_cache_creation_tokens BIGINT;
    v_model TEXT;
    v_provider TEXT;
    v_duration_ms INTEGER;
    v_finish_reason TEXT;
    v_turn_id UUID;
    v_agent_id UUID;
BEGIN
    -- Only process llm.generation events
    IF NEW.event_type != 'llm.generation' THEN
        RETURN NEW;
    END IF;

    -- Extract usage data from event
    v_input_tokens := COALESCE((NEW.data->'metadata'->'usage'->>'input_tokens')::BIGINT, 0);
    v_output_tokens := COALESCE((NEW.data->'metadata'->'usage'->>'output_tokens')::BIGINT, 0);
    v_cache_read_tokens := COALESCE((NEW.data->'metadata'->'usage'->>'cache_read_tokens')::BIGINT, 0);
    v_cache_creation_tokens := COALESCE((NEW.data->'metadata'->'usage'->>'cache_creation_tokens')::BIGINT, 0);
    v_model := NEW.data->'metadata'->>'model';
    v_provider := NEW.data->'metadata'->>'provider';
    v_duration_ms := (NEW.data->'metadata'->>'duration_ms')::INTEGER;
    v_turn_id := (NEW.data->'context'->>'turn_id')::UUID;
    v_finish_reason := NEW.data->'metadata'->'finish_reasons'->>0;

    -- 1. Insert into llm_generations (SOURCE OF TRUTH)
    INSERT INTO llm_generations (
        session_id, turn_id, event_id, model, provider,
        input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
        duration_ms, finish_reason, created_at
    ) VALUES (
        NEW.session_id, v_turn_id, NEW.id, v_model, v_provider,
        v_input_tokens, v_output_tokens, v_cache_read_tokens, v_cache_creation_tokens,
        v_duration_ms, v_finish_reason, NEW.created_at
    );

    -- 2. Update session denormalized totals
    UPDATE sessions
    SET
        total_input_tokens = total_input_tokens + v_input_tokens,
        total_output_tokens = total_output_tokens + v_output_tokens,
        total_cache_read_tokens = total_cache_read_tokens + v_cache_read_tokens,
        total_cache_creation_tokens = total_cache_creation_tokens + v_cache_creation_tokens
    WHERE id = NEW.session_id;

    -- 3. Update agent denormalized totals
    SELECT agent_id INTO v_agent_id FROM sessions WHERE id = NEW.session_id;
    IF v_agent_id IS NOT NULL THEN
        UPDATE agents
        SET
            total_input_tokens = total_input_tokens + v_input_tokens,
            total_output_tokens = total_output_tokens + v_output_tokens,
            total_cache_read_tokens = total_cache_read_tokens + v_cache_read_tokens,
            total_cache_creation_tokens = total_cache_creation_tokens + v_cache_creation_tokens
        WHERE id = v_agent_id;
    END IF;

    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trigger_process_llm_generation ON events;
CREATE TRIGGER trigger_process_llm_generation
    AFTER INSERT ON events
    FOR EACH ROW
    EXECUTE FUNCTION process_llm_generation_event();

-- ============================================================================
-- 4. Backfill existing llm.generation events
-- ============================================================================

DO $$
DECLARE
    v_event RECORD;
BEGIN
    FOR v_event IN
        SELECT id, session_id, data, created_at
        FROM events
        WHERE event_type = 'llm.generation'
        ORDER BY created_at
    LOOP
        INSERT INTO llm_generations (
            session_id, turn_id, event_id, model, provider,
            input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
            duration_ms, finish_reason, created_at
        ) VALUES (
            v_event.session_id,
            (v_event.data->'context'->>'turn_id')::UUID,
            v_event.id,
            v_event.data->'metadata'->>'model',
            v_event.data->'metadata'->>'provider',
            COALESCE((v_event.data->'metadata'->'usage'->>'input_tokens')::BIGINT, 0),
            COALESCE((v_event.data->'metadata'->'usage'->>'output_tokens')::BIGINT, 0),
            COALESCE((v_event.data->'metadata'->'usage'->>'cache_read_tokens')::BIGINT, 0),
            COALESCE((v_event.data->'metadata'->'usage'->>'cache_creation_tokens')::BIGINT, 0),
            (v_event.data->'metadata'->>'duration_ms')::INTEGER,
            v_event.data->'metadata'->'finish_reasons'->>0,
            v_event.created_at
        ) ON CONFLICT DO NOTHING;
    END LOOP;

    -- Update session totals from llm_generations
    UPDATE sessions s
    SET
        total_input_tokens = COALESCE(g.input_tokens, 0),
        total_output_tokens = COALESCE(g.output_tokens, 0),
        total_cache_read_tokens = COALESCE(g.cache_read_tokens, 0),
        total_cache_creation_tokens = COALESCE(g.cache_creation_tokens, 0)
    FROM (
        SELECT session_id,
            SUM(input_tokens) as input_tokens,
            SUM(output_tokens) as output_tokens,
            SUM(cache_read_tokens) as cache_read_tokens,
            SUM(cache_creation_tokens) as cache_creation_tokens
        FROM llm_generations
        GROUP BY session_id
    ) g
    WHERE s.id = g.session_id;

    -- Update agent totals from sessions
    UPDATE agents a
    SET
        total_input_tokens = COALESCE(g.input_tokens, 0),
        total_output_tokens = COALESCE(g.output_tokens, 0),
        total_cache_read_tokens = COALESCE(g.cache_read_tokens, 0),
        total_cache_creation_tokens = COALESCE(g.cache_creation_tokens, 0)
    FROM (
        SELECT agent_id,
            SUM(total_input_tokens) as input_tokens,
            SUM(total_output_tokens) as output_tokens,
            SUM(total_cache_read_tokens) as cache_read_tokens,
            SUM(total_cache_creation_tokens) as cache_creation_tokens
        FROM sessions
        GROUP BY agent_id
    ) g
    WHERE a.id = g.agent_id;
END $$;

-- ============================================================================
-- 5. Comments
-- ============================================================================

COMMENT ON TABLE llm_generations IS 'Source of truth for all LLM generation calls and their token usage';
COMMENT ON COLUMN llm_generations.event_id IS 'Reference to original event (for traceability)';
COMMENT ON COLUMN llm_generations.turn_id IS 'Groups generations within a turn';

COMMENT ON COLUMN sessions.total_input_tokens IS 'Denormalized: sum of input_tokens from llm_generations';
COMMENT ON COLUMN sessions.total_output_tokens IS 'Denormalized: sum of output_tokens from llm_generations';
COMMENT ON COLUMN sessions.total_cache_read_tokens IS 'Denormalized: sum of cache_read_tokens from llm_generations';
COMMENT ON COLUMN sessions.total_cache_creation_tokens IS 'Denormalized: sum of cache_creation_tokens from llm_generations';

COMMENT ON COLUMN agents.total_input_tokens IS 'Denormalized: sum of input_tokens across all sessions';
COMMENT ON COLUMN agents.total_output_tokens IS 'Denormalized: sum of output_tokens across all sessions';
COMMENT ON COLUMN agents.total_cache_read_tokens IS 'Denormalized: sum of cache_read_tokens across all sessions';
COMMENT ON COLUMN agents.total_cache_creation_tokens IS 'Denormalized: sum of cache_creation_tokens across all sessions';

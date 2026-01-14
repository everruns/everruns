-- Migration: Add usage tracking columns to sessions and agents
--
-- This migration adds token usage tracking at session and agent levels.
-- Usage is aggregated from llm.generation events stored in the events table.
-- The cached columns are updated incrementally when new events are inserted.

-- Add usage columns to sessions table
-- Stores aggregated token usage for all LLM calls in the session
ALTER TABLE sessions
ADD COLUMN IF NOT EXISTS total_input_tokens BIGINT DEFAULT 0,
ADD COLUMN IF NOT EXISTS total_output_tokens BIGINT DEFAULT 0,
ADD COLUMN IF NOT EXISTS total_cache_read_tokens BIGINT DEFAULT 0,
ADD COLUMN IF NOT EXISTS total_cache_creation_tokens BIGINT DEFAULT 0;

-- Add usage columns to agents table
-- Stores aggregated token usage across all sessions for the agent
ALTER TABLE agents
ADD COLUMN IF NOT EXISTS total_input_tokens BIGINT DEFAULT 0,
ADD COLUMN IF NOT EXISTS total_output_tokens BIGINT DEFAULT 0,
ADD COLUMN IF NOT EXISTS total_cache_read_tokens BIGINT DEFAULT 0,
ADD COLUMN IF NOT EXISTS total_cache_creation_tokens BIGINT DEFAULT 0;

-- Create index on events for efficient usage aggregation queries
-- This index helps when querying llm.generation events for a session
CREATE INDEX IF NOT EXISTS idx_events_session_llm_generation
ON events (session_id, event_type)
WHERE event_type = 'llm.generation';

-- Create a function to extract usage from llm.generation event data
-- Returns the usage as a record with (input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens)
CREATE OR REPLACE FUNCTION extract_llm_usage(event_data JSONB)
RETURNS TABLE(
    input_tokens BIGINT,
    output_tokens BIGINT,
    cache_read_tokens BIGINT,
    cache_creation_tokens BIGINT
) AS $$
BEGIN
    RETURN QUERY SELECT
        COALESCE((event_data->'metadata'->'usage'->>'input_tokens')::BIGINT, 0),
        COALESCE((event_data->'metadata'->'usage'->>'output_tokens')::BIGINT, 0),
        COALESCE((event_data->'metadata'->'usage'->>'cache_read_tokens')::BIGINT, 0),
        COALESCE((event_data->'metadata'->'usage'->>'cache_creation_tokens')::BIGINT, 0);
END;
$$ LANGUAGE plpgsql IMMUTABLE;

-- Create a function to update session usage when an llm.generation event is inserted
CREATE OR REPLACE FUNCTION update_session_usage()
RETURNS TRIGGER AS $$
DECLARE
    usage_data RECORD;
BEGIN
    -- Only process llm.generation events
    IF NEW.event_type = 'llm.generation' THEN
        SELECT * INTO usage_data FROM extract_llm_usage(NEW.data);

        UPDATE sessions
        SET
            total_input_tokens = total_input_tokens + usage_data.input_tokens,
            total_output_tokens = total_output_tokens + usage_data.output_tokens,
            total_cache_read_tokens = total_cache_read_tokens + usage_data.cache_read_tokens,
            total_cache_creation_tokens = total_cache_creation_tokens + usage_data.cache_creation_tokens
        WHERE id = NEW.session_id;

        -- Also update the agent's usage
        UPDATE agents
        SET
            total_input_tokens = total_input_tokens + usage_data.input_tokens,
            total_output_tokens = total_output_tokens + usage_data.output_tokens,
            total_cache_read_tokens = total_cache_read_tokens + usage_data.cache_read_tokens,
            total_cache_creation_tokens = total_cache_creation_tokens + usage_data.cache_creation_tokens
        WHERE id = (SELECT agent_id FROM sessions WHERE id = NEW.session_id);
    END IF;

    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- Create trigger to automatically update usage on event insert
DROP TRIGGER IF EXISTS trigger_update_session_usage ON events;
CREATE TRIGGER trigger_update_session_usage
    AFTER INSERT ON events
    FOR EACH ROW
    EXECUTE FUNCTION update_session_usage();

-- Backfill existing usage data from events
-- This runs once during migration to populate initial values
DO $$
DECLARE
    sess RECORD;
    total_usage RECORD;
BEGIN
    FOR sess IN SELECT id, agent_id FROM sessions LOOP
        SELECT
            COALESCE(SUM((data->'metadata'->'usage'->>'input_tokens')::BIGINT), 0) as input_tokens,
            COALESCE(SUM((data->'metadata'->'usage'->>'output_tokens')::BIGINT), 0) as output_tokens,
            COALESCE(SUM((data->'metadata'->'usage'->>'cache_read_tokens')::BIGINT), 0) as cache_read_tokens,
            COALESCE(SUM((data->'metadata'->'usage'->>'cache_creation_tokens')::BIGINT), 0) as cache_creation_tokens
        INTO total_usage
        FROM events
        WHERE session_id = sess.id AND event_type = 'llm.generation';

        UPDATE sessions
        SET
            total_input_tokens = total_usage.input_tokens,
            total_output_tokens = total_usage.output_tokens,
            total_cache_read_tokens = total_usage.cache_read_tokens,
            total_cache_creation_tokens = total_usage.cache_creation_tokens
        WHERE id = sess.id;
    END LOOP;
END $$;

-- Recalculate agent totals from sessions
UPDATE agents a
SET
    total_input_tokens = COALESCE(session_totals.input_tokens, 0),
    total_output_tokens = COALESCE(session_totals.output_tokens, 0),
    total_cache_read_tokens = COALESCE(session_totals.cache_read_tokens, 0),
    total_cache_creation_tokens = COALESCE(session_totals.cache_creation_tokens, 0)
FROM (
    SELECT
        agent_id,
        SUM(total_input_tokens) as input_tokens,
        SUM(total_output_tokens) as output_tokens,
        SUM(total_cache_read_tokens) as cache_read_tokens,
        SUM(total_cache_creation_tokens) as cache_creation_tokens
    FROM sessions
    GROUP BY agent_id
) session_totals
WHERE a.id = session_totals.agent_id;

-- Add comment explaining the usage tracking
COMMENT ON COLUMN sessions.total_input_tokens IS 'Cumulative input/prompt tokens for all LLM calls in this session';
COMMENT ON COLUMN sessions.total_output_tokens IS 'Cumulative output/completion tokens for all LLM calls in this session';
COMMENT ON COLUMN sessions.total_cache_read_tokens IS 'Cumulative cache read tokens for all LLM calls in this session';
COMMENT ON COLUMN sessions.total_cache_creation_tokens IS 'Cumulative cache creation tokens for all LLM calls in this session';
COMMENT ON COLUMN agents.total_input_tokens IS 'Cumulative input/prompt tokens across all sessions for this agent';
COMMENT ON COLUMN agents.total_output_tokens IS 'Cumulative output/completion tokens across all sessions for this agent';
COMMENT ON COLUMN agents.total_cache_read_tokens IS 'Cumulative cache read tokens across all sessions for this agent';
COMMENT ON COLUMN agents.total_cache_creation_tokens IS 'Cumulative cache creation tokens across all sessions for this agent';

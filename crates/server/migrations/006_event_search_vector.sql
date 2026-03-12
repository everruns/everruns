-- Full-text search on events (EVE-87)
--
-- Replaces `data::text ILIKE '%query%'` (sequential scan) with a tsvector
-- generated column + GIN index for indexed full-text search.
--
-- The search_vector is built from data->>'content' which is the main text
-- field in message-type event JSONB payloads. A partial GIN index covers
-- only message-type events (the only types searched in practice).

-- Generated tsvector column from the content field in JSONB data
ALTER TABLE events
    ADD COLUMN search_vector tsvector
    GENERATED ALWAYS AS (
        to_tsvector('english', COALESCE(data->>'content', ''))
    ) STORED;

-- GIN index on search_vector, scoped to message-type events
-- These are the event types used in list_message_events_filtered
CREATE INDEX idx_events_search_vector
    ON events USING GIN(search_vector)
    WHERE event_type IN (
        'input.message',
        'output.message.completed',
        'output.message.delta',
        'tool.completed'
    );

COMMENT ON COLUMN events.search_vector IS 'Generated tsvector for full-text search on data.content (EVE-87)';

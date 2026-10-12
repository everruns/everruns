-- Everruns v0.8.5
-- Squashed migration for changes between v0.8.4 and v0.8.5
-- BREAKING CHANGE: Requires fresh database (drop existing _sqlx_migrations table)
--
-- Includes:
-- - Full-text search on events (EVE-87)
-- - Workflow snapshot checkpointing (EVE-86)

-- ============================================
-- Event full-text search (EVE-87)
-- ============================================
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

-- ============================================
-- Workflow snapshot checkpointing (EVE-86)
-- ============================================
-- Periodic snapshots of serialized workflow state to avoid replaying
-- all events from the beginning. On replay, the engine loads the latest
-- snapshot + events since that snapshot's sequence number.
--
-- Reduces replay cost from O(total_events) to O(checkpoint_interval).

CREATE TABLE IF NOT EXISTS durable_workflow_snapshots (
    id BIGSERIAL PRIMARY KEY,
    workflow_id UUID NOT NULL REFERENCES durable_workflow_instances(id) ON DELETE CASCADE,
    sequence_num INT NOT NULL,         -- Event sequence at which snapshot was taken
    snapshot_data BYTEA NOT NULL,      -- Serialized workflow state (JSON)
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- Only one snapshot per (workflow, sequence) pair
    UNIQUE(workflow_id, sequence_num)
);

-- Fast lookup: latest snapshot for a workflow
CREATE INDEX IF NOT EXISTS idx_durable_workflow_snapshots_latest
    ON durable_workflow_snapshots(workflow_id, sequence_num DESC);

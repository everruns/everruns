-- Event retention: archived_events table and archival support
--
-- Phase 1 of EVE-9: configurable EVENT_RETENTION_DAYS with background archival.
-- Old events are moved from `events` to `archived_events`, then deleted.
-- The delete trigger is bypassed by a session variable set by the archival process.
--
-- Usage: SET LOCAL app.archival_bypass = 'true'; DELETE FROM events WHERE ...;

-- Archived events table (same schema as events, no foreign keys)
CREATE TABLE archived_events (
    id UUID PRIMARY KEY,
    session_id UUID NOT NULL,
    sequence INTEGER NOT NULL,
    event_type VARCHAR(100) NOT NULL,
    data JSONB NOT NULL DEFAULT '{}',
    ts TIMESTAMPTZ NOT NULL,
    context JSONB NOT NULL DEFAULT '{}',
    metadata JSONB,
    tags TEXT[],
    created_at TIMESTAMPTZ NOT NULL,
    archived_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_archived_events_session ON archived_events(session_id);
CREATE INDEX idx_archived_events_ts ON archived_events(ts);

COMMENT ON TABLE archived_events IS 'Cold storage for events past retention period';

-- Replace the delete trigger to allow archival bypass
CREATE OR REPLACE FUNCTION prevent_event_mutation()
RETURNS TRIGGER AS $$
BEGIN
    -- Allow deletes when archival bypass flag is set
    IF TG_OP = 'DELETE' AND current_setting('app.archival_bypass', true) = 'true' THEN
        RETURN OLD;
    END IF;
    RAISE EXCEPTION 'events are append-only: % operations are not allowed', TG_OP;
END;
$$ LANGUAGE plpgsql;

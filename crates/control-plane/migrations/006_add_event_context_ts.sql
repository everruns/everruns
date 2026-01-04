-- Add context and ts columns to events table to match Event struct
--
-- This aligns the database schema with the core Event struct:
-- - ts: Event timestamp (when the event occurred, may differ from created_at)
-- - context: EventContext with turn_id, input_message_id, exec_id

-- Add ts column for event timestamp (defaults to created_at for existing rows)
ALTER TABLE events ADD COLUMN ts TIMESTAMPTZ;
UPDATE events SET ts = created_at WHERE ts IS NULL;
ALTER TABLE events ALTER COLUMN ts SET NOT NULL;
ALTER TABLE events ALTER COLUMN ts SET DEFAULT NOW();

COMMENT ON COLUMN events.ts IS 'Event timestamp (when the event occurred)';

-- Add context column for EventContext
ALTER TABLE events ADD COLUMN context JSONB NOT NULL DEFAULT '{}';

COMMENT ON COLUMN events.context IS 'Event correlation context (turn_id, input_message_id, exec_id)';

-- Add metadata column for arbitrary event metadata
ALTER TABLE events ADD COLUMN metadata JSONB;

COMMENT ON COLUMN events.metadata IS 'Arbitrary metadata for the event';

-- Add tags column for categorization
ALTER TABLE events ADD COLUMN tags TEXT[];

COMMENT ON COLUMN events.tags IS 'Tags for filtering and categorization';

-- Index on tags for efficient filtering
CREATE INDEX idx_events_tags ON events USING GIN(tags);

-- Add context and ts columns to events table to match Event struct
--
-- This aligns the database schema with the core Event struct:
-- - ts: Event timestamp (when the event occurred, may differ from created_at)
-- - context: EventContext with turn_id, input_message_id, exec_id
--
-- Note: metadata and tags were added in migration 004

-- Add ts column for event timestamp (defaults to created_at for existing rows)
ALTER TABLE events ADD COLUMN ts TIMESTAMPTZ;
UPDATE events SET ts = created_at WHERE ts IS NULL;
ALTER TABLE events ALTER COLUMN ts SET NOT NULL;
ALTER TABLE events ALTER COLUMN ts SET DEFAULT NOW();

COMMENT ON COLUMN events.ts IS 'Event timestamp (when the event occurred)';

-- Add context column for EventContext
ALTER TABLE events ADD COLUMN context JSONB NOT NULL DEFAULT '{}';

COMMENT ON COLUMN events.context IS 'Event correlation context (turn_id, input_message_id, exec_id)';

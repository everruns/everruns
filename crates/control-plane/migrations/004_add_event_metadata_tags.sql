-- Add metadata and tags columns to events table
--
-- These columns provide additional filtering and categorization capabilities:
-- - metadata: Arbitrary JSONB data for event-specific context
-- - tags: Array of strings for categorization and filtering

-- Add metadata column for arbitrary event context
ALTER TABLE events ADD COLUMN metadata JSONB;

COMMENT ON COLUMN events.metadata IS 'Arbitrary metadata for the event';

-- Add tags column for categorization
ALTER TABLE events ADD COLUMN tags TEXT[];

COMMENT ON COLUMN events.tags IS 'Tags for filtering and categorization';

-- Index on tags for efficient filtering
CREATE INDEX idx_events_tags ON events USING GIN(tags);

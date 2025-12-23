-- Add metadata column to messages table
-- This supports the new message contract with message-level metadata

ALTER TABLE messages
    ADD COLUMN metadata JSONB;

-- Add tags column to messages table (for request-level tags)
ALTER TABLE messages
    ADD COLUMN tags TEXT[] NOT NULL DEFAULT '{}';

-- Create index for metadata queries (GIN for JSONB)
CREATE INDEX idx_messages_metadata ON messages USING GIN(metadata) WHERE metadata IS NOT NULL;

-- Create index for tags
CREATE INDEX idx_messages_tags ON messages USING GIN(tags);

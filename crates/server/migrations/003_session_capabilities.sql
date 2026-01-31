-- Add capabilities column to sessions table
-- Allows setting session-level capabilities that are additive to agent capabilities

ALTER TABLE sessions ADD COLUMN capabilities JSONB NOT NULL DEFAULT '[]'::jsonb;

-- Add comment for documentation
COMMENT ON COLUMN sessions.capabilities IS 'Session-level capabilities (additive to agent capabilities)';

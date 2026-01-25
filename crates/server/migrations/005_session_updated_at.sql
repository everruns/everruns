-- Add updated_at column to sessions table
--
-- Sessions need updated_at to track when they were last modified.
-- Uses the existing update_updated_at_column() trigger function.

ALTER TABLE sessions ADD COLUMN updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW();

-- Backfill: Set updated_at to created_at for existing sessions
UPDATE sessions SET updated_at = created_at;

-- Create trigger to auto-update updated_at on row updates
CREATE TRIGGER update_sessions_updated_at BEFORE UPDATE ON sessions
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

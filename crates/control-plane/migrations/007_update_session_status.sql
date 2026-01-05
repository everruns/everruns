-- Update session status values: pending/running/completed/failed -> started/active/idle
-- This migration updates the status constraint and migrates existing data

-- First, migrate existing data to new status values
UPDATE sessions SET status = CASE
    WHEN status = 'pending' THEN 'started'
    WHEN status = 'running' THEN 'active'
    WHEN status IN ('completed', 'failed') THEN 'idle'
    ELSE 'started'
END;

-- Drop the old constraint
ALTER TABLE sessions DROP CONSTRAINT IF EXISTS sessions_status_check;

-- Add the new constraint with new status values
ALTER TABLE sessions ADD CONSTRAINT sessions_status_check
    CHECK (status IN ('started', 'active', 'idle'));

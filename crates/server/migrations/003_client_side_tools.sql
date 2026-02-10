-- Migration 003: Client-side tools support
--
-- Adds tools JSONB column to agents and sessions tables for client-side tool definitions.
-- Updates sessions status constraint to include 'waiting_for_tool_results'.

-- Add tools column to agents
ALTER TABLE agents ADD COLUMN IF NOT EXISTS tools JSONB NOT NULL DEFAULT '[]'::jsonb;

-- Add tools column to sessions
ALTER TABLE sessions ADD COLUMN IF NOT EXISTS tools JSONB NOT NULL DEFAULT '[]'::jsonb;

-- Update sessions status constraint to include waiting_for_tool_results
ALTER TABLE sessions DROP CONSTRAINT IF EXISTS sessions_status_check;
ALTER TABLE sessions ADD CONSTRAINT sessions_status_check
    CHECK (status IN ('started', 'active', 'idle', 'waiting_for_tool_results'));

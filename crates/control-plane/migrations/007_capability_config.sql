-- Migration: Add config column to agent_capabilities table
-- Purpose: Support per-agent capability configuration

-- Add config column with default empty object
ALTER TABLE agent_capabilities
ADD COLUMN config JSONB NOT NULL DEFAULT '{}';

-- Add comment for documentation
COMMENT ON COLUMN agent_capabilities.config IS 'Per-agent capability configuration (JSON object)';

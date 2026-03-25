-- Add session-level system_prompt and initial_files overrides
-- These layer on top of agent defaults (same additive pattern as capabilities/tools).

ALTER TABLE sessions ADD COLUMN IF NOT EXISTS system_prompt TEXT;
ALTER TABLE sessions ADD COLUMN IF NOT EXISTS initial_files JSONB NOT NULL DEFAULT '[]'::jsonb;

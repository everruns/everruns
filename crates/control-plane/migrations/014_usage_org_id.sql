-- Migration: Add org_id to llm_generations for per-organization usage tracking
--
-- This enables efficient queries for organization-level usage aggregation.
-- The org_id is derived from the session's agent's org_id when inserting.

-- Add org_id column to llm_generations (nullable for backfill)
ALTER TABLE llm_generations
ADD COLUMN IF NOT EXISTS org_id BIGINT REFERENCES organizations(org_id);

-- Backfill org_id from session -> agent relationship
UPDATE llm_generations g
SET org_id = a.org_id
FROM sessions s
JOIN agents a ON s.agent_id = a.id
WHERE g.session_id = s.id
  AND g.org_id IS NULL;

-- After backfill, make it NOT NULL with default for safety
ALTER TABLE llm_generations
ALTER COLUMN org_id SET DEFAULT 1;

ALTER TABLE llm_generations
ALTER COLUMN org_id SET NOT NULL;

-- Add index for org-level queries
CREATE INDEX IF NOT EXISTS idx_llm_generations_org_id ON llm_generations(org_id);
CREATE INDEX IF NOT EXISTS idx_llm_generations_org_created ON llm_generations(org_id, created_at);

-- Add comment
COMMENT ON COLUMN llm_generations.org_id IS 'Organization ID for efficient per-org usage aggregation';

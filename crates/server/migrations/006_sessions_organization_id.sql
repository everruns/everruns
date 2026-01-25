-- Add org_id to sessions table
-- Sessions become direct children of organizations instead of agents

-- Step 1: Add column (nullable for backfill)
ALTER TABLE sessions ADD COLUMN org_id BIGINT REFERENCES organizations(org_id);

-- Step 2: Backfill from agent's organization
UPDATE sessions s SET org_id = a.org_id FROM agents a WHERE s.agent_id = a.id;

-- Step 3: Make NOT NULL after backfill
ALTER TABLE sessions ALTER COLUMN org_id SET NOT NULL;

-- Step 4: Add index for org-level queries
CREATE INDEX idx_sessions_org_id ON sessions(org_id);

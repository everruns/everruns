-- Agent Blueprints: blueprint-backed sessions
-- When set, reason_activity and act_activity build RuntimeAgent from the
-- blueprint definition instead of from harness_id/agent_id.

ALTER TABLE sessions ADD COLUMN IF NOT EXISTS blueprint_id TEXT;
ALTER TABLE sessions ADD COLUMN IF NOT EXISTS blueprint_config JSONB;

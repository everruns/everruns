-- agent_completion_check
-- When a turn about to finish gets a completion check and, if its work is
-- unfinished, a bounded automatic continuation: 'off', 'unattended' (sessions
-- started by triggers and schedules) or 'always'.

ALTER TABLE agents
    ADD COLUMN completion_check TEXT NOT NULL DEFAULT 'unattended',
    ADD CONSTRAINT agents_completion_check_valid CHECK (completion_check IN ('off', 'unattended', 'always'));

COMMENT ON COLUMN agents.completion_check IS
    'Completion check of finishing turns: off, unattended (trigger and schedule sessions) or always.';

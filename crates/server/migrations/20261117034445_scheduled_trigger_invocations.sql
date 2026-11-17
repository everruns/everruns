-- A receipt lives as long as its retriable durable task. Receipt, session and
-- input commit together; a lost worker acknowledgement only replays the result.
CREATE TABLE scheduled_trigger_invocations (
    task_id UUID PRIMARY KEY REFERENCES durable_task_queue(id) ON DELETE CASCADE,
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    trigger_id UUID NOT NULL REFERENCES agent_triggers(id) ON DELETE CASCADE,
    result JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

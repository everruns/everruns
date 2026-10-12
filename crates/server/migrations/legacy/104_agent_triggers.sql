-- Agent triggers: org-scoped, agent-owned autonomous invocation triggers.
--
-- Mirrors the agent_identities CRUD shape (see 008_v0.8.7.sql). The concrete
-- per-type configuration lives in `config` (JSONB); `durable_schedule_id`
-- optionally binds a schedule trigger to a durable_schedules row, matching the
-- app_channels durable-schedule binding (see 021).

CREATE TABLE agent_triggers (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) DEFAULT 1,
    agent_id UUID NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    trigger_type VARCHAR(50) NOT NULL DEFAULT 'schedule',
    config JSONB NOT NULL DEFAULT '{}'::jsonb,
    enabled BOOLEAN NOT NULL DEFAULT true,
    durable_schedule_id UUID REFERENCES durable_schedules(id) ON DELETE SET NULL,
    status VARCHAR(50) NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'archived', 'deleted')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    archived_at TIMESTAMPTZ,
    deleted_at TIMESTAMPTZ
);

CREATE INDEX idx_agent_triggers_org_id ON agent_triggers(org_id);
CREATE INDEX idx_agent_triggers_agent_id ON agent_triggers(agent_id);

-- At most one trigger may bind a given durable schedule (cf. app_channels, 021).
CREATE UNIQUE INDEX agent_triggers_durable_schedule_id_idx
    ON agent_triggers (durable_schedule_id)
    WHERE durable_schedule_id IS NOT NULL;

CREATE TRIGGER update_agent_triggers_updated_at BEFORE UPDATE ON agent_triggers
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

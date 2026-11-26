-- Preserve what opened an episode. Existing episodes deliberately stay unknown:
-- their original rolling-window calculation cannot be reconstructed honestly.
CREATE TABLE health_issue_observations (
    issue_id UUID NOT NULL REFERENCES health_issues(id) ON DELETE CASCADE,
    episode_id UUID NOT NULL,
    org_id BIGINT NOT NULL,
    observed_at TIMESTAMPTZ NOT NULL,
    evidence JSONB,
    PRIMARY KEY (issue_id, episode_id)
);
CREATE FUNCTION capture_health_issue_opening() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.status IN ('open', 'needs_check') AND
       (TG_OP = 'INSERT' OR NEW.episode_id IS DISTINCT FROM OLD.episode_id) THEN
        INSERT INTO health_issue_observations (issue_id, episode_id, org_id, observed_at, evidence)
        VALUES (NEW.id, NEW.episode_id, NEW.org_id, NEW.last_checked_at, NEW.evidence)
        ON CONFLICT DO NOTHING;
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER health_issue_opening AFTER INSERT OR UPDATE ON health_issues
    FOR EACH ROW EXECUTE FUNCTION capture_health_issue_opening();

-- Caller identity is a fact of authenticated ingress, never event payload data.
ALTER TABLE agent_trigger_deliveries ADD COLUMN requested_by UUID, ADD COLUMN input_message_id UUID;
CREATE TABLE agent_trigger_run_evidence (
    execution_id UUID PRIMARY KEY REFERENCES durable_schedule_executions(id) ON DELETE CASCADE,
    org_id BIGINT NOT NULL,
    trigger_id UUID NOT NULL REFERENCES agent_triggers(id) ON DELETE CASCADE,
    requested_by UUID,
    source TEXT NOT NULL,
    input_message_id UUID,
    configuration JSONB NOT NULL,
    trigger_revision TIMESTAMPTZ NOT NULL
);
CREATE INDEX agent_trigger_run_evidence_trigger ON agent_trigger_run_evidence (org_id, trigger_id);

-- Match the time/keyset ordering used by the agent-scoped run history.
CREATE INDEX durable_agent_trigger_run_history
    ON durable_schedule_executions (schedule_id, scheduled_at DESC, id DESC);

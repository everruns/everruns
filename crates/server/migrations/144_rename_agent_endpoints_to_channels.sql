-- Rename the Agent "endpoint" concept to "channel".
--
-- An endpoint read like a URL, but the row is the transport an agent is
-- reachable through (Slack, AG-UI, A2A, FCP, Public Chat). "Channel" is the
-- product term; see knowledge/integrations/agent-exposure.md.
--
-- Rolling-deploy compatibility for one release: pods still running the
-- previous build keep reading and writing `agent_endpoints` and
-- `sessions.endpoint_id`. Both shims are removed by a follow-up migration.

-- ---------------------------------------------------------------------------
-- Table
-- ---------------------------------------------------------------------------

ALTER TABLE agent_endpoints RENAME TO agent_channels;

ALTER TABLE agent_channels
    RENAME CONSTRAINT agent_endpoints_public_id_format TO agent_channels_public_id_format;
ALTER INDEX idx_agent_endpoints_agent_id RENAME TO idx_agent_channels_agent_id;
ALTER INDEX idx_agent_endpoints_app_id RENAME TO idx_agent_channels_app_id;
ALTER INDEX idx_agent_endpoints_status RENAME TO idx_agent_channels_status;
ALTER INDEX agent_endpoints_durable_schedule_id_idx RENAME TO agent_channels_durable_schedule_id_idx;
ALTER INDEX idx_agent_endpoints_legacy_app_channel_type
    RENAME TO idx_agent_channels_legacy_app_channel_type;
ALTER TRIGGER update_agent_endpoints_updated_at ON agent_channels
    RENAME TO update_agent_channels_updated_at;

-- A single-table view is automatically updatable, and inserts through it pick
-- up the base table's column defaults, so the previous build keeps working.
CREATE VIEW agent_endpoints AS SELECT * FROM agent_channels;

-- ---------------------------------------------------------------------------
-- Session attribution
-- ---------------------------------------------------------------------------

-- A column cannot be aliased, and the previous build names `endpoint_id` in
-- every session projection, so add `channel_id` and keep both in step until
-- the follow-up drops `endpoint_id`.
ALTER TABLE sessions
    ADD COLUMN channel_id UUID REFERENCES agent_channels(id) ON DELETE SET NULL;

COMMENT ON COLUMN sessions.channel_id IS
    'Channel whose ingress created this session. NULL for user, API, and '
    'platform-created sessions.';

UPDATE sessions SET channel_id = endpoint_id WHERE endpoint_id IS NOT NULL;

CREATE INDEX idx_sessions_channel_id ON sessions(channel_id)
    WHERE channel_id IS NOT NULL;

CREATE FUNCTION sync_session_channel_id() RETURNS trigger AS $$
BEGIN
    IF TG_OP = 'INSERT' THEN
        NEW.channel_id := COALESCE(NEW.channel_id, NEW.endpoint_id);
        NEW.endpoint_id := COALESCE(NEW.endpoint_id, NEW.channel_id);
    ELSIF NEW.channel_id IS DISTINCT FROM OLD.channel_id THEN
        NEW.endpoint_id := NEW.channel_id;
    ELSIF NEW.endpoint_id IS DISTINCT FROM OLD.endpoint_id THEN
        NEW.channel_id := NEW.endpoint_id;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER sync_session_channel_id
    BEFORE INSERT OR UPDATE OF channel_id, endpoint_id ON sessions
    FOR EACH ROW EXECUTE FUNCTION sync_session_channel_id();

-- ---------------------------------------------------------------------------
-- Budgets
-- ---------------------------------------------------------------------------

-- `agent_endpoint` stays accepted so the previous build can still write it
-- during the rollout; new code reads it as `agent_channel`.
ALTER TABLE budgets
    DROP CONSTRAINT IF EXISTS budgets_subject_type_check;

ALTER TABLE budgets
    ADD CONSTRAINT budgets_subject_type_check
    CHECK (subject_type IN (
        'session', 'agent', 'user', 'org', 'app', 'app_channel', 'agent_endpoint', 'agent_channel'
    ));

UPDATE budgets
SET subject_type = 'agent_channel',
    updated_at = NOW()
WHERE subject_type = 'agent_endpoint';

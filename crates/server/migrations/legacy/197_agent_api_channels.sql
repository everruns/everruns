-- Agent Execution API: the `api` channel type and the org-owned agent keys
-- that call it.
--
-- An `api` channel is an agent's base URL for code: its card, sessions,
-- messages and events. Agent keys (`evr_ak_`) are managed credentials granted
-- to one api channel. Only the SHA-256 hash and a display prefix are kept.
-- A rotation keeps the key's id, so sessions tagged with it survive, and the
-- previous secret stays valid until `previous_valid_until`.
-- See knowledge/integrations/agent-execution-api.md.

ALTER TABLE apps
    DROP CONSTRAINT IF EXISTS apps_channel_type_check;

ALTER TABLE apps
    ADD CONSTRAINT apps_channel_type_check
    CHECK (channel_type IN ('slack', 'ag_ui', 'schedule', 'webhook', 'a2a', 'fcp', 'api_endpoint', 'public_chat', 'voice', 'api'));

ALTER TABLE agent_channels
    DROP CONSTRAINT IF EXISTS agent_channels_channel_type_check;

ALTER TABLE agent_channels
    ADD CONSTRAINT agent_channels_channel_type_check
    CHECK (channel_type IN ('slack', 'ag_ui', 'schedule', 'webhook', 'a2a', 'fcp', 'api_endpoint', 'public_chat', 'voice', 'api'));

CREATE TABLE agent_keys (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) DEFAULT 1,
    channel_id UUID NOT NULL REFERENCES agent_channels(id) ON DELETE CASCADE,
    name VARCHAR(128) NOT NULL,
    token_hash TEXT NOT NULL,
    token_prefix TEXT NOT NULL,
    previous_token_hash TEXT,
    previous_valid_until TIMESTAMPTZ,
    permissions TEXT[] NOT NULL DEFAULT ARRAY['sessions']::TEXT[],
    expires_at TIMESTAMPTZ,
    last_used_at TIMESTAMPTZ,
    -- Audit only: the key belongs to the org and outlives its creator.
    created_by_user_id UUID REFERENCES users(id) ON DELETE SET NULL,
    revoked_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE UNIQUE INDEX agent_keys_token_hash_idx ON agent_keys(token_hash);
CREATE UNIQUE INDEX agent_keys_previous_token_hash_idx
    ON agent_keys(previous_token_hash)
    WHERE previous_token_hash IS NOT NULL;
CREATE INDEX idx_agent_keys_org_channel ON agent_keys(org_id, channel_id);

CREATE TRIGGER update_agent_keys_updated_at BEFORE UPDATE ON agent_keys
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

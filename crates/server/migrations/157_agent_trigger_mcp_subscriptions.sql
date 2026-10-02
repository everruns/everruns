-- Inbound MCP Events subscriptions held by agent triggers (EVE-1121).
-- See knowledge/integrations/mcp-events.md.
--
-- One row per `mcp_event` trigger: the subscription Everruns holds, as the
-- agent, on the agent's MCP server. `secret_encrypted` is the `whsec_` signing
-- secret Everruns generated and handed to the server, encrypted at rest; the
-- callback verifies every delivery against it. `remote_subscription_id` and
-- `refresh_before` come from the server's `events/subscribe` result, and the
-- refresher re-subscribes rows whose `refresh_before` is near. The trigger's
-- own config stays in `agent_triggers.config`; this row is mutable state only.
CREATE TABLE agent_trigger_mcp_subscriptions (
    trigger_id UUID PRIMARY KEY REFERENCES agent_triggers(id) ON DELETE CASCADE,
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    secret_encrypted BYTEA NOT NULL,
    remote_subscription_id TEXT,
    refresh_before TIMESTAMPTZ,
    cursor TEXT,
    status VARCHAR(20) NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'active', 'failed')),
    last_error TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_agent_trigger_mcp_subscriptions_refresh
    ON agent_trigger_mcp_subscriptions (refresh_before)
    WHERE status <> 'pending';

CREATE TRIGGER update_agent_trigger_mcp_subscriptions_updated_at
    BEFORE UPDATE ON agent_trigger_mcp_subscriptions
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

-- Outbound MCP Events webhook subscriptions (EVE-1121).
-- See knowledge/integrations/mcp-events.md.
--
-- One row per (principal, callback URL, event name, canonical arguments):
-- `subscription_key` is that tuple's hash, so a repeated `events/subscribe`
-- refreshes the same row instead of stacking deliveries. The signing secret is
-- the client's, encrypted at rest.
CREATE TABLE mcp_event_subscriptions (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    subscription_key TEXT NOT NULL UNIQUE,
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    event_name TEXT NOT NULL,
    arguments JSONB NOT NULL DEFAULT '{}'::jsonb,
    callback_url TEXT NOT NULL,
    secret_encrypted BYTEA NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_mcp_event_subscriptions_org_event
    ON mcp_event_subscriptions(org_id, event_name, expires_at);

CREATE TRIGGER update_mcp_event_subscriptions_updated_at
    BEFORE UPDATE ON mcp_event_subscriptions
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

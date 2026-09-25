CREATE TABLE org_slack_connections (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    org_id BIGINT NOT NULL UNIQUE REFERENCES organizations(org_id) ON DELETE CASCADE,
    access_token_encrypted BYTEA,
    refresh_token_encrypted BYTEA,
    access_token_expires_at TIMESTAMPTZ,
    state TEXT NOT NULL DEFAULT 'connected'
        CHECK (state IN ('connected', 'reconnect_required')),
    token_generation BIGINT NOT NULL DEFAULT 1,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT org_slack_connections_connected_tokens CHECK (
        state <> 'connected'
        OR (
            access_token_encrypted IS NOT NULL
            AND refresh_token_encrypted IS NOT NULL
            AND access_token_expires_at IS NOT NULL
        )
    )
);

CREATE INDEX idx_org_slack_connections_rotation
    ON org_slack_connections(access_token_expires_at)
    WHERE state = 'connected';

CREATE TRIGGER update_org_slack_connections_updated_at
    BEFORE UPDATE ON org_slack_connections
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

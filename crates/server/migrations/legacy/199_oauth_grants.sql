-- Connected AI clients, phase 1: one grant per approval of an MCP OAuth client.
--
-- A grant records that a user approved a client to act as them on `/mcp`.
-- Re-approving the same client updates the same row (one grant per client and
-- user). Refresh tokens point at their grant; revoking a grant deletes them and
-- `/mcp` rejects access tokens that name a revoked grant.
--
-- `access` and `allowed_org_ids` are stored now so the permissions phase needs
-- no schema change; only 'read_and_run' and NULL (all organizations) are
-- written until then. See knowledge/integrations/mcp-connected-clients.md.

CREATE TABLE oauth_grants (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    client_id TEXT NOT NULL REFERENCES oauth_clients(client_id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    access TEXT NOT NULL DEFAULT 'read_and_run'
        CHECK (access IN ('read_only', 'read_and_run')),
    -- NULL means every organization the user belongs to.
    allowed_org_ids BIGINT[],
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_used_at TIMESTAMPTZ,
    revoked_at TIMESTAMPTZ,
    UNIQUE (client_id, user_id)
);

CREATE INDEX idx_oauth_grants_user_id ON oauth_grants(user_id);

-- Nullable: a replica still running the previous release can mint refresh
-- tokens without a grant during a rolling deploy. The token endpoint resolves
-- those to the client and user's grant on their next refresh.
ALTER TABLE oauth_refresh_tokens
    ADD COLUMN grant_id UUID REFERENCES oauth_grants(id) ON DELETE CASCADE;

CREATE INDEX idx_oauth_refresh_tokens_grant_id ON oauth_refresh_tokens(grant_id);
CREATE INDEX idx_oauth_refresh_tokens_client_user ON oauth_refresh_tokens(client_id, user_id);

-- Backfill: every live connection becomes a full-access, all-organizations
-- grant, so nothing already connected stops working.
INSERT INTO oauth_grants (client_id, user_id, created_at)
SELECT rt.client_id, rt.user_id, MIN(rt.created_at)
FROM oauth_refresh_tokens rt
JOIN oauth_clients c ON c.client_id = rt.client_id
WHERE rt.expires_at > NOW()
GROUP BY rt.client_id, rt.user_id
ON CONFLICT (client_id, user_id) DO NOTHING;

UPDATE oauth_refresh_tokens rt
SET grant_id = g.id
FROM oauth_grants g
WHERE g.client_id = rt.client_id
  AND g.user_id = rt.user_id;

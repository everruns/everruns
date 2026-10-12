-- Per-agent GitHub Apps.
--
-- "Connect GitHub" on an agent identity creates a GitHub App for that agent
-- through GitHub's App manifest flow: GitHub returns the App id, private key,
-- client secret and webhook secret to us, so nobody copies credentials by
-- hand. The identity's `github` connection (agent_identity_connections) then
-- stores the installation id, and every GitHub consumer (tools, git, MCP,
-- webhook-driven triggers) resolves through that one connection. The id of
-- this row is embedded in the App's webhook and setup URLs.
--
-- One App per identity; a reconnect reuses it. Secrets are encrypted with the
-- server encryption key, like other connection credentials.

CREATE TABLE github_apps (
    id UUID PRIMARY KEY,
    org_id BIGINT NOT NULL REFERENCES organizations(org_id),
    agent_identity_id UUID NOT NULL UNIQUE REFERENCES agent_identities(id) ON DELETE CASCADE,
    app_id BIGINT NOT NULL UNIQUE,
    slug TEXT NOT NULL,
    name TEXT NOT NULL,
    html_url TEXT NOT NULL,
    owner_login TEXT,
    client_id TEXT,
    client_secret_encrypted BYTEA,
    private_key_encrypted BYTEA NOT NULL,
    webhook_secret_encrypted BYTEA,
    created_by_user_id UUID,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_github_apps_org_id ON github_apps (org_id);

CREATE TRIGGER update_github_apps_updated_at BEFORE UPDATE ON github_apps
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

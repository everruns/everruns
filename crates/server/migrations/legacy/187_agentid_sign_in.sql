-- AgentID consumer sign-in (knowledge/integrations/agentid.md).
--
-- 1. agentid_login_states: one row per browser sign-in in flight. Keyed by
--    the SHA-256 of the opaque `state` parameter, so the table never holds a
--    value that could complete a sign-in by itself. Rows are consumed once and
--    expire after ten minutes; expired rows are swept on the next insert.
-- 2. agentid_agent_owners: the AgentID `owner_sub` of each virtual user an
--    AgentID sign-in created, so an org can cap how many agents one human
--    owner may register. `owner_email` is an optional contact, never a key and
--    never linked to a management user.
-- 3. organization_settings.agentid_agents_per_owner: the org's cap. NULL
--    means the platform default.

CREATE TABLE agentid_login_states (
    state_hash BYTEA PRIMARY KEY,
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    channel_id TEXT NOT NULL,
    code_verifier TEXT NOT NULL,
    nonce TEXT NOT NULL,
    login_hint TEXT,
    expires_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX agentid_login_states_expiry ON agentid_login_states(expires_at);

CREATE TABLE agentid_agent_owners (
    org_id BIGINT NOT NULL,
    virtual_user_id UUID NOT NULL,
    owner_sub TEXT NOT NULL,
    owner_email TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (org_id, virtual_user_id),
    FOREIGN KEY (org_id, virtual_user_id) REFERENCES virtual_users(org_id, id) ON DELETE CASCADE
);
CREATE INDEX agentid_agent_owners_owner ON agentid_agent_owners(org_id, owner_sub);

ALTER TABLE organization_settings
    ADD COLUMN agentid_agents_per_owner INTEGER
    CHECK (agentid_agents_per_owner IS NULL OR agentid_agents_per_owner >= 0);

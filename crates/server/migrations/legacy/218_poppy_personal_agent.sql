-- Everruns agents as Personal Agent Protocol ("Poppy", draft 0.1) personal
-- agents: the client side of the protocol, the other end of the Poppy
-- channel (knowledge/integrations/poppy-personal-agent.md).
--
-- 1. poppy_agent_keys: the deployment's one signing key. Its public half is
--    published at the deployment's client metadata document (`client_id`),
--    and it signs client and session assertions for every agent here.
-- 2. poppy_agent_accounts: the User ID and DPoP key of one person (the
--    session owner) at one company (OAuth issuer). Random per company, so
--    two companies cannot correlate the person (spec 4.2).
-- 3. poppy_agent_conversations: the live Poppy Session and conversation one
--    Everruns session holds with one company: endpoints, cursor, and the
--    encrypted Session Token.

CREATE TABLE poppy_agent_keys (
    id SMALLINT PRIMARY KEY DEFAULT 1 CHECK (id = 1),
    kid TEXT NOT NULL,
    public_jwk JSONB NOT NULL,
    private_key BYTEA NOT NULL,
    encrypted BOOLEAN NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE poppy_agent_accounts (
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    owner_principal_id UUID NOT NULL REFERENCES principals(id) ON DELETE CASCADE,
    issuer TEXT NOT NULL,
    user_id TEXT NOT NULL,
    dpop_jwk JSONB NOT NULL,
    dpop_key BYTEA NOT NULL,
    encrypted BOOLEAN NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (org_id, owner_principal_id, issuer)
);

CREATE TABLE poppy_agent_conversations (
    session_id UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    domain TEXT NOT NULL,
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    issuer TEXT NOT NULL,
    state JSONB NOT NULL DEFAULT '{}'::jsonb,
    access_token BYTEA,
    -- Whether access_token is ciphertext (false only without an encryption key).
    token_sealed BOOLEAN NOT NULL DEFAULT FALSE,
    token_expires_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (session_id, domain)
);

CREATE TRIGGER update_poppy_agent_conversations_updated_at BEFORE UPDATE ON poppy_agent_conversations
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

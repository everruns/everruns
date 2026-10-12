-- PACT Delegated profile (PACT 1.0 §5): the OAuth 2.0 device-code
-- authorization server an A2A endpoint runs for personal agents
-- (knowledge/integrations/a2a-channel.md, "Inbound PACT Delegated profile").
--
-- 1. pact_signing_keys: one ES256 key per endpoint, generated on first use.
--    It signs delegation tokens, consent-page sessions and receipts. The
--    private key is encrypted with the server's secrets key when one is set.
-- 2. pact_device_authorizations: one row per device-code request. Keyed by
--    the SHA-256 of the device code, so the table never holds a value that
--    could redeem it. `user_code` binds the company's sign-in to the request.
-- 3. pact_grants: what a user approved for one personal agent. Tokens name
--    the grant, so revoking it stops every token issued under it.
-- 4. pact_refresh_tokens: single-use refresh tokens, also stored hashed.
-- 5. pact_used_assertions: `jti`s of company sign-in assertions already
--    redeemed, so a captured assertion cannot open a second consent page.

CREATE TABLE pact_signing_keys (
    channel_id UUID PRIMARY KEY REFERENCES agent_channels(id) ON DELETE CASCADE,
    kid TEXT NOT NULL,
    private_key BYTEA NOT NULL,
    encrypted BOOLEAN NOT NULL,
    public_jwk JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE pact_grants (
    id TEXT PRIMARY KEY,
    channel_id UUID NOT NULL REFERENCES agent_channels(id) ON DELETE CASCADE,
    client_id TEXT NOT NULL,
    brand_user_id TEXT NOT NULL,
    scope TEXT NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    revoked_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX pact_grants_user ON pact_grants(channel_id, client_id, brand_user_id);

CREATE TABLE pact_device_authorizations (
    device_code_hash BYTEA PRIMARY KEY,
    channel_id UUID NOT NULL REFERENCES agent_channels(id) ON DELETE CASCADE,
    client_id TEXT NOT NULL,
    user_code TEXT NOT NULL,
    requested_scope TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'approved', 'denied', 'consumed')),
    brand_user_id TEXT,
    grant_id TEXT REFERENCES pact_grants(id) ON DELETE CASCADE,
    interval_secs INTEGER NOT NULL,
    last_polled_at TIMESTAMPTZ,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX pact_device_authorizations_user_code
    ON pact_device_authorizations(channel_id, user_code);
CREATE INDEX pact_device_authorizations_expiry ON pact_device_authorizations(expires_at);

CREATE TABLE pact_refresh_tokens (
    token_hash BYTEA PRIMARY KEY,
    grant_id TEXT NOT NULL REFERENCES pact_grants(id) ON DELETE CASCADE,
    expires_at TIMESTAMPTZ NOT NULL,
    used_at TIMESTAMPTZ
);
CREATE INDEX pact_refresh_tokens_grant ON pact_refresh_tokens(grant_id);

CREATE TABLE pact_used_assertions (
    channel_id UUID NOT NULL REFERENCES agent_channels(id) ON DELETE CASCADE,
    jti TEXT NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (channel_id, jti)
);
CREATE INDEX pact_used_assertions_expiry ON pact_used_assertions(expires_at);

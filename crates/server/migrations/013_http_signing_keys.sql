-- HTTP Message Signatures key directory
--
-- Stores Ed25519 public keys for the /.well-known/http-message-signatures-directory
-- endpoint (draft-meunier-http-message-signatures-directory). Keys are registered
-- when bot-auth is configured on the web_fetch capability.

CREATE TABLE http_signing_keys (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id      BIGINT NOT NULL REFERENCES organizations(id),
    key_id      TEXT NOT NULL,           -- JWK Thumbprint (RFC 7638)
    public_key  JSONB NOT NULL,          -- Full JWK: {"kty":"OKP","crv":"Ed25519","x":"..."}
    label       TEXT,                    -- Optional human-readable label
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at  TIMESTAMPTZ,            -- NULL = no expiry
    UNIQUE(org_id, key_id)
);

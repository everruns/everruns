-- Idempotency keys for POST /v1/commands/{name}. A client that retries a
-- mutating command with the same `Idempotency-Key` gets the first result back
-- instead of running the command twice. Keys are scoped to the caller (org and
-- principal), so one caller can never read another's stored output.
--
-- `locked_until` is the in-flight lease: a request that died mid-command
-- leaves its row in progress, and once the lease passes the same request may
-- claim it again. `expires_at` bounds how long a key is remembered.
-- `response` is the stored response, encrypted with the server's encryption
-- key when one is configured: some commands return a secret exactly once
-- (a share token, for example) that is otherwise kept only as a hash.
CREATE TABLE command_idempotency_keys (
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    principal_id UUID NOT NULL,
    idempotency_key TEXT NOT NULL,
    command TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    response BYTEA NULL,
    locked_until TIMESTAMPTZ NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, principal_id, idempotency_key)
);

CREATE INDEX command_idempotency_keys_expires_idx ON command_idempotency_keys (expires_at);

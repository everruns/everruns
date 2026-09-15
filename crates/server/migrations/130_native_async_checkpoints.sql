-- Private runtime journal. Session deletion owns retention; forks never copy
-- live call identities, provider receipts, or execution ownership.
CREATE TABLE native_async_checkpoints (
    -- The secret-rotation registry addresses rows by a single UUID.
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    session_id UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    turn_id UUID NOT NULL,
    org_id BIGINT NOT NULL REFERENCES organizations(org_id),
    owner UUID NOT NULL,
    lease_until TIMESTAMPTZ NOT NULL,
    payload_encrypted BYTEA NOT NULL,
    UNIQUE (session_id, turn_id)
);

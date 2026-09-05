-- Private runtime journal. Session deletion owns retention; forks never copy
-- live call identities, provider receipts, or execution ownership.
CREATE TABLE native_async_checkpoints (
    session_id UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    turn_id UUID NOT NULL,
    org_id BIGINT NOT NULL REFERENCES organizations(org_id),
    owner UUID NOT NULL,
    lease_until TIMESTAMPTZ NOT NULL,
    payload_encrypted BYTEA NOT NULL,
    PRIMARY KEY (session_id, turn_id)
);

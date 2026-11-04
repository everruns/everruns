-- Runtime-only encrypted maintenance state; separate from conversation and Memory surfaces.
CREATE TABLE dreaming_state (
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    key TEXT NOT NULL,
    revision BIGINT NOT NULL,
    value_encrypted BYTEA NOT NULL,
    session_id UUID REFERENCES sessions(id) ON DELETE CASCADE,
    due_at BIGINT,
    PRIMARY KEY (org_id,key)
);
CREATE INDEX dreaming_state_due ON dreaming_state(due_at) WHERE due_at IS NOT NULL;

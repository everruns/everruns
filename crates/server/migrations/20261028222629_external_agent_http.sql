-- HTTP bindings reuse custom-worker routing. Secrets stay encrypted at rest;
-- the worker token is adapter-only and never leaves the control plane.
ALTER TABLE custom_workers ADD COLUMN endpoint_url TEXT;
ALTER TABLE custom_workers ADD COLUMN endpoint_secret_encrypted BYTEA;
ALTER TABLE custom_workers ADD COLUMN adapter_token_encrypted BYTEA;
ALTER TABLE custom_workers ADD COLUMN last_error TEXT;
ALTER TABLE custom_workers ADD COLUMN last_invoked_at TIMESTAMPTZ;
ALTER TABLE custom_workers ADD CONSTRAINT custom_worker_http_complete CHECK (
    (endpoint_url IS NULL AND endpoint_secret_encrypted IS NULL AND adapter_token_encrypted IS NULL)
    OR (endpoint_url IS NOT NULL AND endpoint_secret_encrypted IS NOT NULL AND adapter_token_encrypted IS NOT NULL)
);

CREATE INDEX custom_workers_http_schedule ON custom_workers(last_invoked_at NULLS FIRST, id)
    WHERE enabled AND endpoint_url IS NOT NULL;

-- Admission reservations hold no database connection during customer execution.
-- Expiry recovers capacity after a CP crash; actor leases still fence all writes.
CREATE TABLE external_worker_slots (
    id UUID PRIMARY KEY,
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    expires_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX external_worker_slots_org_expiry ON external_worker_slots(org_id, expires_at);

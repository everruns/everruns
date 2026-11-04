-- Authorization records, not credentials. No production issuer is wired.
-- Match the maximum actor lease window; admission also checks the actual lease.
CREATE TABLE worker_journal_read_grants (
    id UUID PRIMARY KEY CHECK (id <> '00000000-0000-0000-0000-000000000000'),
    executor_id UUID NOT NULL CHECK (executor_id <> '00000000-0000-0000-0000-000000000000'),
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    session_id UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    lease_holder TEXT NOT NULL CHECK (length(btrim(lease_holder)) > 0 AND octet_length(lease_holder) <= 256),
    lease_fence BIGINT NOT NULL CHECK (lease_fence > 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp() CHECK (isfinite(created_at)),
    expires_at TIMESTAMPTZ NOT NULL CHECK (isfinite(expires_at)),
    revoked_at TIMESTAMPTZ CHECK (revoked_at IS NULL OR isfinite(revoked_at)),
    CHECK (expires_at > created_at AND expires_at <= created_at + INTERVAL '5 minutes')
);
CREATE INDEX worker_journal_read_grants_session_idx ON worker_journal_read_grants(session_id);
CREATE INDEX worker_journal_read_grants_org_idx ON worker_journal_read_grants(org_id);

-- A retained grant reference cannot be retargeted, extended, or un-revoked.
-- Retain grant tombstones while their parent is live, so deleting/recreating
-- a row cannot revive a revoked reference. Parent deletion removes the journal.
CREATE FUNCTION guard_worker_journal_read_grant_update() RETURNS trigger AS $$
BEGIN
    IF (NEW.id, NEW.executor_id, NEW.org_id, NEW.session_id, NEW.lease_holder,
        NEW.lease_fence, NEW.created_at, NEW.expires_at)
        IS DISTINCT FROM
       (OLD.id, OLD.executor_id, OLD.org_id, OLD.session_id, OLD.lease_holder,
        OLD.lease_fence, OLD.created_at, OLD.expires_at) THEN
        RAISE EXCEPTION 'worker journal grant binding is immutable' USING ERRCODE = 'check_violation';
    END IF;
    IF OLD.revoked_at IS NOT NULL AND NEW.revoked_at IS DISTINCT FROM OLD.revoked_at THEN
        RAISE EXCEPTION 'worker journal grant revocation is irreversible' USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;
CREATE TRIGGER worker_journal_read_grants_immutable
    BEFORE UPDATE ON worker_journal_read_grants
    FOR EACH ROW EXECUTE FUNCTION guard_worker_journal_read_grant_update();

CREATE FUNCTION guard_worker_journal_read_grant_delete() RETURNS trigger AS $$
BEGIN
    IF EXISTS (SELECT 1 FROM sessions WHERE id = OLD.session_id)
       AND EXISTS (SELECT 1 FROM organizations WHERE org_id = OLD.org_id) THEN
        RAISE EXCEPTION 'live-session worker journal grant must be retained' USING ERRCODE = 'check_violation';
    END IF;
    RETURN OLD;
END;
$$ LANGUAGE plpgsql;
CREATE TRIGGER worker_journal_read_grants_retained
    BEFORE DELETE ON worker_journal_read_grants
    FOR EACH ROW EXECUTE FUNCTION guard_worker_journal_read_grant_delete();

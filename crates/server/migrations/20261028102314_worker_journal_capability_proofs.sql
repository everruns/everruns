-- Empty verifier records only. No production identity/capability issuer exists.
-- Identity is an exact transport-verified leaf certificate, not its CN or a header.
CREATE TABLE worker_executor_identities (
    certificate_sha256 BYTEA PRIMARY KEY CHECK (octet_length(certificate_sha256) = 32),
    executor_id UUID NOT NULL CHECK (executor_id <> '00000000-0000-0000-0000-000000000000'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp() CHECK (isfinite(created_at)),
    expires_at TIMESTAMPTZ NOT NULL CHECK (isfinite(expires_at) AND expires_at > created_at),
    revoked_at TIMESTAMPTZ CHECK (revoked_at IS NULL OR isfinite(revoked_at))
);
CREATE INDEX worker_executor_identities_executor_idx ON worker_executor_identities(executor_id);

CREATE TABLE worker_journal_capabilities (
    capability_sha256 BYTEA PRIMARY KEY CHECK (octet_length(capability_sha256) = 32),
    -- Parent purge leaves an unusable digest tombstone, preventing proof reuse.
    grant_id UUID REFERENCES worker_journal_read_grants(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp() CHECK (isfinite(created_at)),
    expires_at TIMESTAMPTZ NOT NULL CHECK (isfinite(expires_at) AND expires_at > created_at),
    revoked_at TIMESTAMPTZ CHECK (revoked_at IS NULL OR isfinite(revoked_at)),
    CHECK (expires_at <= created_at + INTERVAL '5 minutes')
);
CREATE INDEX worker_journal_capabilities_grant_idx ON worker_journal_capabilities(grant_id);

CREATE FUNCTION guard_worker_executor_identity() RETURNS trigger AS $$
BEGIN
    -- Retain tombstones: a revoked or expired certificate cannot be rebound to
    -- another executor or resurrected. Issuer retention/rotation is not enabled.
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'worker executor identity must be retained' USING ERRCODE = 'check_violation';
    END IF;
    IF (NEW.certificate_sha256, NEW.executor_id, NEW.created_at, NEW.expires_at)
       IS DISTINCT FROM
       (OLD.certificate_sha256, OLD.executor_id, OLD.created_at, OLD.expires_at) THEN
        RAISE EXCEPTION 'worker executor identity binding is immutable' USING ERRCODE = 'check_violation';
    END IF;
    IF OLD.revoked_at IS NOT NULL AND NEW.revoked_at IS DISTINCT FROM OLD.revoked_at THEN
        RAISE EXCEPTION 'worker executor identity revocation is irreversible' USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;
CREATE TRIGGER worker_executor_identities_guard
    BEFORE UPDATE OR DELETE ON worker_executor_identities
    FOR EACH ROW EXECUTE FUNCTION guard_worker_executor_identity();

CREATE FUNCTION guard_worker_journal_capability() RETURNS trigger AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'worker capability digest must be retained' USING ERRCODE = 'check_violation';
    END IF;
    IF TG_OP = 'INSERT' THEN
        IF NEW.grant_id IS NULL OR NOT EXISTS (SELECT 1 FROM worker_journal_read_grants
                       WHERE id = NEW.grant_id AND expires_at >= NEW.expires_at
                         AND created_at <= NEW.created_at) THEN
            RAISE EXCEPTION 'worker capability exceeds its grant validity' USING ERRCODE = 'check_violation';
        END IF;
    ELSE
        -- Only the FK's parent-purge action may clear the binding. It can never
        -- be set again, even if a trusted issuer accidentally reuses a proof.
        IF OLD.grant_id IS NOT NULL AND NEW.grant_id IS NULL
           AND NOT EXISTS (SELECT 1 FROM worker_journal_read_grants WHERE id = OLD.grant_id)
           AND (NEW.capability_sha256, NEW.created_at, NEW.expires_at, NEW.revoked_at)
               IS NOT DISTINCT FROM
               (OLD.capability_sha256, OLD.created_at, OLD.expires_at, OLD.revoked_at) THEN
            RETURN NEW;
        END IF;
        IF (NEW.capability_sha256, NEW.grant_id, NEW.created_at, NEW.expires_at)
           IS DISTINCT FROM
           (OLD.capability_sha256, OLD.grant_id, OLD.created_at, OLD.expires_at) THEN
            RAISE EXCEPTION 'worker capability binding is immutable' USING ERRCODE = 'check_violation';
        END IF;
        IF OLD.revoked_at IS NOT NULL AND NEW.revoked_at IS DISTINCT FROM OLD.revoked_at THEN
            RAISE EXCEPTION 'worker capability revocation is irreversible' USING ERRCODE = 'check_violation';
        END IF;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;
CREATE TRIGGER worker_journal_capabilities_guard
    BEFORE INSERT OR UPDATE OR DELETE ON worker_journal_capabilities
    FOR EACH ROW EXECUTE FUNCTION guard_worker_journal_capability();

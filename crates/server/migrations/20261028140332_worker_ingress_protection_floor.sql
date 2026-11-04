-- Empty on upgrade: no issuer or activation path is installed.
-- The worker protocol has global operations and bearer-signed object URLs with
-- no assignment selector. A protected plane cannot safely coexist with legacy
-- authority in the same database. This floor remains after grants expire.
CREATE TABLE worker_ingress_protection_floor (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    minimum_version INTEGER NOT NULL CHECK (minimum_version > 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);

CREATE FUNCTION retain_worker_ingress_protection_floor() RETURNS trigger AS $$
BEGIN
    IF TG_OP <> 'UPDATE' THEN
        RAISE EXCEPTION 'worker ingress protection cannot be removed' USING ERRCODE = 'check_violation';
    END IF;
    IF NEW.singleton IS DISTINCT FROM OLD.singleton OR NEW.created_at IS DISTINCT FROM OLD.created_at
       OR NEW.minimum_version < OLD.minimum_version THEN
        RAISE EXCEPTION 'worker ingress protection cannot be downgraded' USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;
CREATE TRIGGER worker_ingress_protection_retained BEFORE UPDATE OR DELETE ON worker_ingress_protection_floor
    FOR EACH ROW EXECUTE FUNCTION retain_worker_ingress_protection_floor();
CREATE TRIGGER worker_ingress_protection_no_truncate BEFORE TRUNCATE ON worker_ingress_protection_floor
    FOR EACH STATEMENT EXECUTE FUNCTION retain_worker_ingress_protection_floor();

-- An opaque capability is protected authority, not an optional legacy hint.
-- Enrollment must not race ahead of refusing the broad worker protocol.
CREATE FUNCTION require_worker_capability_protection_floor() RETURNS trigger AS $$
BEGIN
    PERFORM 1 FROM worker_ingress_protection_floor WHERE singleton AND minimum_version=1;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'worker capability requires a supported protection floor' USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;
CREATE TRIGGER worker_capability_requires_floor BEFORE INSERT ON worker_journal_capabilities
    FOR EACH ROW EXECUTE FUNCTION require_worker_capability_protection_floor();

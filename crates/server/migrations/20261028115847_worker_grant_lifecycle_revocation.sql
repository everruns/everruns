-- Source-only lifecycle and explicit revocation dependencies. No issuer is wired.
ALTER TABLE sessions ADD COLUMN worker_authorization_epoch BIGINT NOT NULL DEFAULT 1 CHECK (worker_authorization_epoch > 0);
ALTER TABLE agents ADD COLUMN worker_authorization_epoch BIGINT NOT NULL DEFAULT 1 CHECK (worker_authorization_epoch > 0);
ALTER TABLE session_actors ADD COLUMN worker_authorization_epoch BIGINT NOT NULL DEFAULT 1 CHECK (worker_authorization_epoch > 0);
ALTER TABLE resource_access ADD COLUMN worker_authorization_epoch BIGINT NOT NULL DEFAULT 1 CHECK (worker_authorization_epoch > 0);
ALTER TABLE resource_grants ADD COLUMN worker_authorization_epoch BIGINT NOT NULL DEFAULT 1 CHECK (worker_authorization_epoch > 0);

-- NULL origins deliberately leave pre-upgrade or invalid grants unusable.
ALTER TABLE worker_journal_read_grants
    ADD COLUMN actor_session_id UUID REFERENCES session_actors(session_id) ON DELETE SET NULL,
    ADD COLUMN actor_authorization_epoch BIGINT,
    ADD COLUMN session_authorization_epoch BIGINT,
    ADD COLUMN agent_id UUID,
    ADD COLUMN agent_authorization_epoch BIGINT,
    ADD CHECK (actor_session_id IS NULL OR actor_session_id = session_id);

CREATE INDEX worker_journal_grants_agent_origin ON worker_journal_read_grants(agent_id);
CREATE INDEX worker_journal_grants_actor_origin ON worker_journal_read_grants(actor_session_id);

CREATE FUNCTION worker_resource_authorization_epoch() RETURNS trigger AS $$
BEGIN
    IF NEW.worker_authorization_epoch IS DISTINCT FROM OLD.worker_authorization_epoch THEN
        RAISE EXCEPTION 'worker authorization epoch is server-owned' USING ERRCODE = 'check_violation';
    END IF;
    -- These records hold authority, not usage telemetry. Capture even changes
    -- made without a canonical revision bump; restoring old values never revives.
    IF NEW IS DISTINCT FROM OLD THEN
        NEW.worker_authorization_epoch := OLD.worker_authorization_epoch + 1;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;
CREATE TRIGGER zz_worker_resource_epoch BEFORE UPDATE ON resource_access
    FOR EACH ROW EXECUTE FUNCTION worker_resource_authorization_epoch();
CREATE TRIGGER zz_worker_grant_epoch BEFORE UPDATE ON resource_grants
    FOR EACH ROW EXECUTE FUNCTION worker_resource_authorization_epoch();

CREATE FUNCTION worker_session_authorization_epoch() RETURNS trigger AS $$
BEGIN
    IF NEW.worker_authorization_epoch IS DISTINCT FROM OLD.worker_authorization_epoch THEN
        RAISE EXCEPTION 'worker authorization epoch is server-owned' USING ERRCODE = 'check_violation';
    END IF;
    IF (NEW.org_id, NEW.owner_principal_id, NEW.resolved_owner_user_id, NEW.agent_id,
        NEW.virtual_user_id, NEW.archived_at) IS DISTINCT FROM
       (OLD.org_id, OLD.owner_principal_id, OLD.resolved_owner_user_id, OLD.agent_id,
        OLD.virtual_user_id, OLD.archived_at) THEN
        NEW.worker_authorization_epoch := OLD.worker_authorization_epoch + 1;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;
CREATE TRIGGER zz_worker_session_epoch BEFORE UPDATE ON sessions
    FOR EACH ROW EXECUTE FUNCTION worker_session_authorization_epoch();

CREATE FUNCTION worker_agent_authorization_epoch() RETURNS trigger AS $$
BEGIN
    IF NEW.worker_authorization_epoch IS DISTINCT FROM OLD.worker_authorization_epoch THEN
        RAISE EXCEPTION 'worker authorization epoch is server-owned' USING ERRCODE = 'check_violation';
    END IF;
    IF (NEW.org_id, NEW.status, NEW.virtual_user_id) IS DISTINCT FROM
       (OLD.org_id, OLD.status, OLD.virtual_user_id) THEN
        NEW.worker_authorization_epoch := OLD.worker_authorization_epoch + 1;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;
CREATE TRIGGER zz_worker_agent_epoch BEFORE UPDATE ON agents
    FOR EACH ROW EXECUTE FUNCTION worker_agent_authorization_epoch();

CREATE FUNCTION worker_actor_authorization_epoch() RETURNS trigger AS $$
BEGIN
    IF NEW.worker_authorization_epoch IS DISTINCT FROM OLD.worker_authorization_epoch THEN
        RAISE EXCEPTION 'worker authorization epoch is server-owned' USING ERRCODE = 'check_violation';
    END IF;
    IF (NEW.org_id, NEW.lease_holder, NEW.lease_fence, NEW.cancel_requested_at, NEW.cancel_inbox_seq)
        IS DISTINCT FROM
       (OLD.org_id, OLD.lease_holder, OLD.lease_fence, OLD.cancel_requested_at, OLD.cancel_inbox_seq)
       OR (OLD.lease_expires_at IS NOT NULL AND
           (NEW.lease_expires_at IS NULL OR NEW.lease_expires_at < OLD.lease_expires_at
            OR OLD.lease_expires_at <= clock_timestamp())) THEN
        NEW.worker_authorization_epoch := OLD.worker_authorization_epoch + 1;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;
CREATE TRIGGER zz_worker_actor_epoch BEFORE UPDATE ON session_actors
    FOR EACH ROW EXECUTE FUNCTION worker_actor_authorization_epoch();

CREATE FUNCTION capture_worker_journal_origin() RETURNS trigger AS $$
DECLARE
    expected_agent UUID;
    captured_agent_epoch BIGINT;
    captured_session_epoch BIGINT;
    captured_actor_epoch BIGINT;
BEGIN
    -- Never trust caller-provided origins. Source locks serialize creation
    -- against lifecycle writes: agent -> session -> actor. Invalid creation
    -- stays permanently unbound rather than inheriting a later lease.
    NEW.actor_session_id := NULL;
    NEW.actor_authorization_epoch := NULL;
    NEW.session_authorization_epoch := NULL;
    NEW.agent_id := NULL;
    NEW.agent_authorization_epoch := NULL;
    SELECT agent_id INTO expected_agent FROM sessions WHERE id=NEW.session_id AND org_id=NEW.org_id;
    IF NOT FOUND THEN RETURN NEW; END IF;
    IF expected_agent IS NOT NULL THEN
        SELECT worker_authorization_epoch INTO captured_agent_epoch FROM agents
        WHERE id=expected_agent AND org_id=NEW.org_id AND status='active' FOR SHARE;
        IF NOT FOUND THEN RETURN NEW; END IF;
    END IF;
    SELECT worker_authorization_epoch INTO captured_session_epoch FROM sessions
    WHERE id=NEW.session_id AND org_id=NEW.org_id AND archived_at IS NULL
      AND agent_id IS NOT DISTINCT FROM expected_agent FOR SHARE;
    IF NOT FOUND THEN RETURN NEW; END IF;
    SELECT worker_authorization_epoch INTO captured_actor_epoch FROM session_actors
    WHERE session_id=NEW.session_id AND org_id=NEW.org_id
      AND lease_holder=NEW.lease_holder AND lease_fence=NEW.lease_fence
      AND lease_expires_at > clock_timestamp() AND cancel_requested_at IS NULL FOR SHARE;
    IF NOT FOUND THEN RETURN NEW; END IF;
    NEW.actor_session_id := NEW.session_id;
    NEW.actor_authorization_epoch := captured_actor_epoch;
    NEW.session_authorization_epoch := captured_session_epoch;
    NEW.agent_id := expected_agent;
    NEW.agent_authorization_epoch := captured_agent_epoch;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;
CREATE TRIGGER worker_journal_capture_origin BEFORE INSERT ON worker_journal_read_grants
    FOR EACH ROW EXECUTE FUNCTION capture_worker_journal_origin();

CREATE FUNCTION guard_worker_journal_origin() RETURNS trigger AS $$
BEGIN
    IF (NEW.actor_session_id, NEW.actor_authorization_epoch, NEW.session_authorization_epoch,
        NEW.agent_id, NEW.agent_authorization_epoch) IS DISTINCT FROM
       (OLD.actor_session_id, OLD.actor_authorization_epoch, OLD.session_authorization_epoch,
        OLD.agent_id, OLD.agent_authorization_epoch) THEN
        -- The actor FK may clear a deleted origin, never bind a replacement.
        IF OLD.actor_session_id IS NOT NULL AND NEW.actor_session_id IS NULL
           AND NOT EXISTS (SELECT 1 FROM session_actors WHERE session_id=OLD.actor_session_id)
           AND (NEW.actor_authorization_epoch, NEW.session_authorization_epoch, NEW.agent_id, NEW.agent_authorization_epoch)
               IS NOT DISTINCT FROM
               (OLD.actor_authorization_epoch, OLD.session_authorization_epoch, OLD.agent_id, OLD.agent_authorization_epoch) THEN
            NEW.revoked_at := COALESCE(OLD.revoked_at, clock_timestamp());
            RETURN NEW;
        END IF;
        RAISE EXCEPTION 'worker journal origin is immutable' USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;
CREATE TRIGGER worker_journal_origin_immutable BEFORE UPDATE ON worker_journal_read_grants
    FOR EACH ROW EXECUTE FUNCTION guard_worker_journal_origin();

-- These are mandatory revocation prerequisites, not resource-use or journal
-- authority. Only explicit session-scoped service grants are supported here;
-- Only organization-owned service resources qualify; personal/agent ownership
-- or consumers need verified invocation provenance before enrollment.
CREATE TABLE worker_journal_grant_dependencies (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    grant_id UUID NOT NULL REFERENCES worker_journal_read_grants(id) ON DELETE CASCADE,
    source_grant_id UUID REFERENCES resource_grants(id) ON DELETE SET NULL,
    source_resource_id UUID NOT NULL,
    grant_revision BIGINT NOT NULL CHECK (grant_revision > 0),
    resource_revision BIGINT NOT NULL CHECK (resource_revision > 0),
    grant_epoch BIGINT NOT NULL CHECK (grant_epoch > 0),
    resource_epoch BIGINT NOT NULL CHECK (resource_epoch > 0),
    UNIQUE(grant_id, source_grant_id)
);
CREATE INDEX worker_journal_dependencies_resource ON worker_journal_grant_dependencies(source_resource_id);
CREATE INDEX worker_journal_dependencies_source ON worker_journal_grant_dependencies(source_grant_id);

CREATE FUNCTION guard_worker_journal_dependency() RETURNS trigger AS $$
DECLARE
    assignment worker_journal_read_grants;
BEGIN
    IF TG_OP='DELETE' THEN
        IF EXISTS (SELECT 1 FROM worker_journal_read_grants WHERE id=OLD.grant_id) THEN
            RAISE EXCEPTION 'live worker dependency must be retained' USING ERRCODE = 'check_violation';
        END IF;
        RETURN OLD;
    ELSIF TG_OP='UPDATE' THEN
        IF NEW IS NOT DISTINCT FROM OLD THEN RETURN NEW; END IF;
        IF OLD.source_grant_id IS NOT NULL AND NEW.source_grant_id IS NULL
           AND NOT EXISTS (SELECT 1 FROM resource_grants WHERE id=OLD.source_grant_id)
           AND (NEW.id, NEW.grant_id, NEW.source_resource_id, NEW.grant_revision, NEW.resource_revision, NEW.grant_epoch, NEW.resource_epoch)
               IS NOT DISTINCT FROM
               (OLD.id, OLD.grant_id, OLD.source_resource_id, OLD.grant_revision, OLD.resource_revision, OLD.grant_epoch, OLD.resource_epoch) THEN
            UPDATE worker_journal_read_grants SET revoked_at=clock_timestamp()
            WHERE id=OLD.grant_id AND revoked_at IS NULL;
            RETURN NEW;
        END IF;
        RAISE EXCEPTION 'worker dependency binding is immutable' USING ERRCODE = 'check_violation';
    END IF;
    SELECT * INTO assignment FROM worker_journal_read_grants WHERE id=NEW.grant_id;
    IF NOT FOUND OR NEW.source_grant_id IS NULL THEN
        RAISE EXCEPTION 'invalid worker dependency' USING ERRCODE = 'check_violation';
    END IF;
    -- Source locks before the assignment lock avoid source-revoker deadlocks.
    -- Revision AND server epoch pin the checked state, including non-revisioned
    -- changes. A revoke/delete racing registration either waits or rejects it.
    PERFORM 1 FROM resource_access WHERE id=NEW.source_resource_id AND org_id=assignment.org_id
      AND revision=NEW.resource_revision AND worker_authorization_epoch=NEW.resource_epoch
      AND revoked_at IS NULL AND scope='organization' AND usage='service' FOR SHARE;
    IF NOT FOUND THEN RAISE EXCEPTION 'invalid worker dependency' USING ERRCODE = 'check_violation'; END IF;
    PERFORM 1 FROM resource_grants WHERE id=NEW.source_grant_id AND org_id=assignment.org_id
      AND resource_id=NEW.source_resource_id AND revision=NEW.grant_revision
      AND worker_authorization_epoch=NEW.grant_epoch AND revoked_at IS NULL
      AND (expires_at IS NULL OR expires_at > clock_timestamp())
      AND consumer_scope='session' AND consumer_session_id=assignment.session_id AND usage='service' FOR SHARE;
    IF NOT FOUND THEN RAISE EXCEPTION 'invalid worker dependency' USING ERRCODE = 'check_violation'; END IF;
    PERFORM 1 FROM worker_journal_read_grants g
      JOIN sessions s ON s.id=g.session_id AND s.org_id=g.org_id
      JOIN session_actors a ON a.session_id=g.actor_session_id AND a.org_id=g.org_id
      LEFT JOIN agents ag ON ag.id=g.agent_id AND ag.org_id=g.org_id
      WHERE g.id=NEW.grant_id AND g.revoked_at IS NULL
        AND g.created_at <= clock_timestamp() AND g.expires_at > clock_timestamp()
        AND s.archived_at IS NULL AND g.session_authorization_epoch=s.worker_authorization_epoch
        AND g.actor_authorization_epoch=a.worker_authorization_epoch
        AND a.lease_holder=g.lease_holder AND a.lease_fence=g.lease_fence
        AND a.lease_expires_at > clock_timestamp() AND a.cancel_requested_at IS NULL
        AND g.agent_id IS NOT DISTINCT FROM s.agent_id
        AND (g.agent_id IS NULL OR (ag.status='active' AND g.agent_authorization_epoch=ag.worker_authorization_epoch))
      FOR UPDATE OF g;
    IF NOT FOUND THEN RAISE EXCEPTION 'invalid worker dependency' USING ERRCODE = 'check_violation'; END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;
CREATE TRIGGER worker_journal_dependency_guard BEFORE INSERT OR UPDATE OR DELETE ON worker_journal_grant_dependencies
    FOR EACH ROW EXECUTE FUNCTION guard_worker_journal_dependency();

CREATE FUNCTION revoke_worker_journal_on_source_change() RETURNS trigger AS $$
BEGIN
    IF TG_OP='UPDATE' THEN
        IF NEW.worker_authorization_epoch=OLD.worker_authorization_epoch THEN RETURN NEW; END IF;
    END IF;
    IF TG_TABLE_NAME='resource_grants' THEN
        UPDATE worker_journal_read_grants g SET revoked_at=clock_timestamp()
        FROM worker_journal_grant_dependencies d
        WHERE d.grant_id=g.id AND d.source_grant_id=OLD.id AND g.org_id=OLD.org_id AND g.revoked_at IS NULL;
    ELSIF TG_TABLE_NAME='resource_access' THEN
        UPDATE worker_journal_read_grants g SET revoked_at=clock_timestamp()
        FROM worker_journal_grant_dependencies d
        WHERE d.grant_id=g.id AND d.source_resource_id=OLD.id AND g.org_id=OLD.org_id AND g.revoked_at IS NULL;
    ELSIF TG_TABLE_NAME='agents' THEN
        UPDATE worker_journal_read_grants SET revoked_at=clock_timestamp()
        WHERE agent_id=OLD.id AND org_id=OLD.org_id AND revoked_at IS NULL;
    ELSIF TG_TABLE_NAME='sessions' THEN
        UPDATE worker_journal_read_grants SET revoked_at=clock_timestamp()
        WHERE session_id=OLD.id AND org_id=OLD.org_id AND revoked_at IS NULL;
    ELSE
        UPDATE worker_journal_read_grants SET revoked_at=clock_timestamp()
        WHERE session_id=OLD.session_id AND org_id=OLD.org_id AND revoked_at IS NULL;
    END IF;
    IF TG_OP='DELETE' THEN RETURN OLD; END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;
CREATE TRIGGER worker_resource_revocation AFTER UPDATE OR DELETE ON resource_access
    FOR EACH ROW EXECUTE FUNCTION revoke_worker_journal_on_source_change();
CREATE TRIGGER worker_resource_grant_revocation AFTER UPDATE OR DELETE ON resource_grants
    FOR EACH ROW EXECUTE FUNCTION revoke_worker_journal_on_source_change();
CREATE TRIGGER worker_session_revocation AFTER UPDATE OR DELETE ON sessions
    FOR EACH ROW EXECUTE FUNCTION revoke_worker_journal_on_source_change();
CREATE TRIGGER worker_agent_revocation AFTER UPDATE OR DELETE ON agents
    FOR EACH ROW EXECUTE FUNCTION revoke_worker_journal_on_source_change();
CREATE TRIGGER worker_actor_revocation AFTER UPDATE OR DELETE ON session_actors
    FOR EACH ROW EXECUTE FUNCTION revoke_worker_journal_on_source_change();

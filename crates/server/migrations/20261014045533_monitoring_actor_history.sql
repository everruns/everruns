-- Renewal acknowledgements and ownership history belong to the platform's
-- actor repository. The generic durable queue has no agent dependency.
ALTER TABLE session_actors ADD COLUMN lease_renewed_at timestamptz;
CREATE TABLE session_actor_lease_history (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    session_id uuid NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    org_id bigint NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    occurred_at timestamptz NOT NULL DEFAULT now(),
    kind text NOT NULL CHECK (kind IN ('acquired', 'reacquired', 'released')),
    holder text NOT NULL,
    fence bigint NOT NULL,
    previous_holder text,
    previous_expires_at timestamptz,
    expires_at timestamptz
);
CREATE INDEX idx_actor_lease_history_session ON session_actor_lease_history (session_id, id DESC);
CREATE INDEX idx_actor_lease_history_recent ON session_actor_lease_history (occurred_at, org_id);

CREATE FUNCTION record_actor_renewal() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.lease_holder IS NULL THEN
        NEW.lease_renewed_at := NULL;
    ELSIF TG_OP = 'INSERT' OR NEW.lease_expires_at IS DISTINCT FROM OLD.lease_expires_at
        OR NEW.lease_fence IS DISTINCT FROM OLD.lease_fence THEN
        NEW.lease_renewed_at := now();
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER actor_renewal BEFORE INSERT OR UPDATE ON session_actors
    FOR EACH ROW EXECUTE FUNCTION record_actor_renewal();

CREATE FUNCTION record_actor_ownership() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.lease_holder IS NOT NULL AND (TG_OP = 'INSERT'
        OR NEW.lease_fence IS DISTINCT FROM OLD.lease_fence
        OR NEW.lease_holder IS DISTINCT FROM OLD.lease_holder) THEN
        INSERT INTO session_actor_lease_history
            (session_id, org_id, kind, holder, fence, previous_holder, previous_expires_at, expires_at)
        VALUES (NEW.session_id, NEW.org_id,
            CASE WHEN TG_OP = 'UPDATE' AND OLD.lease_holder IS NOT NULL
                AND OLD.lease_expires_at <= now() THEN 'reacquired' ELSE 'acquired' END,
            NEW.lease_holder, NEW.lease_fence,
            CASE WHEN TG_OP = 'UPDATE' THEN OLD.lease_holder END,
            CASE WHEN TG_OP = 'UPDATE' THEN OLD.lease_expires_at END, NEW.lease_expires_at);
    ELSIF TG_OP = 'UPDATE' AND NEW.lease_holder IS NULL AND OLD.lease_holder IS NOT NULL THEN
        INSERT INTO session_actor_lease_history
            (session_id, org_id, kind, holder, fence, previous_expires_at)
        VALUES (NEW.session_id, NEW.org_id, 'released', OLD.lease_holder,
            OLD.lease_fence, OLD.lease_expires_at);
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER actor_ownership AFTER INSERT OR UPDATE ON session_actors
    FOR EACH ROW EXECUTE FUNCTION record_actor_ownership();

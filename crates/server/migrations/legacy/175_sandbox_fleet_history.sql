-- Org-wide Sandbox fleet history.
--
-- The Sandboxes page lists every logical Sandbox in an org, including deleted
-- ones, and draws when each was running or paused. Two things were missing:
--
-- 1. Deleted Sandboxes vanished with their Session (ON DELETE CASCADE). The
--    Session link now survives as NULL, and the row keeps a small snapshot
--    (agent, Session title) so the history still reads after the Session is
--    gone. Deleted rows are purged after a retention window
--    (`SANDBOX_HISTORY_RETENTION_DAYS`, crates/server/src/event_retention.rs).
-- 2. Only the current state was stored. `sandbox_state_transitions` is an
--    append-only log written by trigger, so every path that moves
--    `observed_state` or `generation` is recorded without each writer having
--    to remember to.

ALTER TABLE sandboxes
    ADD COLUMN agent_id UUID,
    ADD COLUMN session_title TEXT,
    ADD COLUMN deleted_at TIMESTAMPTZ;

ALTER TABLE sandboxes ALTER COLUMN session_id DROP NOT NULL;
ALTER TABLE sandboxes DROP CONSTRAINT sandboxes_session_id_fkey;
ALTER TABLE sandboxes
    ADD CONSTRAINT sandboxes_session_id_fkey
    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE SET NULL;

UPDATE sandboxes sandbox
SET agent_id = session.agent_id,
    session_title = session.title
FROM sessions session
WHERE session.id = sandbox.session_id;

UPDATE sandboxes SET deleted_at = updated_at WHERE observed_state = 'deleted';

COMMENT ON COLUMN sandboxes.session_title IS
    'Session title captured at creation and again when the Session is deleted, so history stays readable.';
COMMENT ON COLUMN sandboxes.deleted_at IS
    'When observed_state became deleted. Purged after the Sandbox history retention window.';

CREATE TABLE sandbox_state_transitions (
    id          BIGSERIAL   PRIMARY KEY,
    sandbox_id  UUID        NOT NULL REFERENCES sandboxes(id) ON DELETE CASCADE,
    org_id      BIGINT      NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    generation  BIGINT      NOT NULL,
    state       TEXT        NOT NULL,
    at          TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_sandbox_state_transitions_sandbox_at
    ON sandbox_state_transitions (sandbox_id, at);
CREATE INDEX idx_sandbox_state_transitions_org_at
    ON sandbox_state_transitions (org_id, at);
CREATE INDEX idx_sandboxes_org_updated
    ON sandboxes (org_id, updated_at DESC);

COMMENT ON TABLE sandbox_state_transitions IS
    'Append-only log of observed_state and generation changes on sandboxes, written by trigger.';

-- Stamp deleted_at on the row itself so readers never derive it from the log.
CREATE FUNCTION sandboxes_stamp_deleted_at() RETURNS trigger AS $$
BEGIN
    IF NEW.observed_state = 'deleted' THEN
        IF TG_OP = 'INSERT' OR OLD.observed_state IS DISTINCT FROM 'deleted' THEN
            NEW.deleted_at := now();
        END IF;
    ELSE
        NEW.deleted_at := NULL;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER sandboxes_stamp_deleted_at
    BEFORE INSERT OR UPDATE OF observed_state ON sandboxes
    FOR EACH ROW EXECUTE FUNCTION sandboxes_stamp_deleted_at();

CREATE FUNCTION sandboxes_record_transition() RETURNS trigger AS $$
BEGIN
    INSERT INTO sandbox_state_transitions (sandbox_id, org_id, generation, state)
    VALUES (NEW.id, NEW.org_id, NEW.generation, NEW.observed_state);
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER sandboxes_record_transition_insert
    AFTER INSERT ON sandboxes
    FOR EACH ROW EXECUTE FUNCTION sandboxes_record_transition();

CREATE TRIGGER sandboxes_record_transition_update
    AFTER UPDATE OF observed_state, generation ON sandboxes
    FOR EACH ROW
    WHEN (OLD.observed_state IS DISTINCT FROM NEW.observed_state
          OR OLD.generation IS DISTINCT FROM NEW.generation)
    EXECUTE FUNCTION sandboxes_record_transition();

-- Seed one transition per existing Sandbox so timelines have a starting state.
INSERT INTO sandbox_state_transitions (sandbox_id, org_id, generation, state, at)
SELECT id, org_id, generation, observed_state, updated_at FROM sandboxes;

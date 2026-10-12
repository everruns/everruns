-- Session actors run whole turns (actor-based design, step 6c): the inbox
-- cursor, the poison guard and worker wake-ups.
--
-- Decisions:
-- - `consumed_seq` is the inbox cursor: every input up to this event sequence
--   reached a turn. A holder advances it on renew and release (never back),
--   so a turn resumed by another worker delivers only input its predecessor
--   never showed the model, and a fresh turn starts after it.
-- - `next_input_seq` is the input the next fresh turn starts from: the first
--   input marked after the cursor. A later mark leaves it alone while it is
--   still ahead of the cursor, so queued messages start turns in arrival
--   order; inputs before it that were never marked (a channel's history
--   backfill) are context, not a turn's start.
-- - `attempts` counts leases since the last release. A turn whose holder
--   keeps dying (crash, OOM, a step that never returns) is leased again and
--   again without a release; past a threshold the next holder seals the turn
--   instead of running it, so one poison turn cannot loop forever.
-- - Wake-ups ride the durable queue's channel (`task_available`) with their
--   own payloads, so workers already subscribed to it hear them:
--   `session_actor` when a row becomes leasable (runnable and free), and
--   `session_actor_cancel` when a cancel lands on a leased actor so its holder
--   renews at once instead of at its next renew tick. Under NATS the server
--   publishes the same payloads itself.
-- Spec: knowledge/operations/durable-execution-engine.md ("Session actors").

ALTER TABLE session_actors
    ADD COLUMN consumed_seq BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN next_input_seq BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN attempts INTEGER NOT NULL DEFAULT 0;

CREATE OR REPLACE FUNCTION notify_session_actor()
RETURNS TRIGGER AS $$
BEGIN
    IF NEW.runnable_since IS NOT NULL AND NEW.lease_holder IS NULL
       AND (TG_OP = 'INSERT'
            OR OLD.runnable_since IS NULL
            OR OLD.lease_holder IS NOT NULL) THEN
        PERFORM pg_notify('task_available', 'session_actor');
    END IF;
    IF TG_OP = 'UPDATE'
       AND NEW.cancel_requested_at IS NOT NULL
       AND NEW.lease_holder IS NOT NULL
       AND NEW.cancel_requested_at IS DISTINCT FROM OLD.cancel_requested_at THEN
        PERFORM pg_notify('task_available', 'session_actor_cancel');
    END IF;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER notify_session_actor
    AFTER INSERT OR UPDATE OF runnable_since, lease_holder, cancel_requested_at
    ON session_actors
    FOR EACH ROW EXECUTE FUNCTION notify_session_actor();

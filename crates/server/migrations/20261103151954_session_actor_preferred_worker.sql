-- Sticky sessions: a session goes back to the worker that ran its previous
-- turn, whose history cache is warm (capacity program, PR 4).
--
-- - `preferred_worker` is the worker that last released the actor. Release
--   sets it, a hand-off (a draining worker) clears it, nothing else touches it.
-- - `session_actor_worker_ready` says whether that worker can take the turn
--   now: registered, active, accepting, heartbeat within 30 s (three
--   heartbeats) and below its slot count. Lease and wake-up decide with the
--   same function, so they agree on when other workers may take the actor
--   at once.
-- - The wake-up names a ready preferred worker (`session_actor@<worker>`),
--   and only that worker's subscription hears it. Otherwise it goes to all.
-- Spec: knowledge/execution/sticky-sessions.md.

ALTER TABLE session_actors ADD COLUMN preferred_worker TEXT;

-- The lease call's first scan: this worker's own runnable actors.
CREATE INDEX idx_session_actors_preferred
    ON session_actors (preferred_worker, runnable_since, session_id)
    WHERE runnable_since IS NOT NULL AND preferred_worker IS NOT NULL;

CREATE OR REPLACE FUNCTION session_actor_worker_ready(worker TEXT)
RETURNS BOOLEAN
LANGUAGE sql STABLE AS $$
    SELECT EXISTS (
        SELECT 1 FROM durable_workers w
        WHERE w.id = worker
          AND w.status = 'active'
          AND w.accepting_tasks
          AND w.last_heartbeat_at > NOW() - INTERVAL '30 seconds'
          AND w.current_load < w.max_concurrency
    )
$$;

CREATE OR REPLACE FUNCTION notify_session_actor()
RETURNS TRIGGER AS $$
BEGIN
    IF NEW.runnable_since IS NOT NULL AND NEW.lease_holder IS NULL
       AND (TG_OP = 'INSERT'
            OR OLD.runnable_since IS NULL
            OR OLD.lease_holder IS NOT NULL) THEN
        IF NEW.preferred_worker IS NOT NULL
           AND session_actor_worker_ready(NEW.preferred_worker) THEN
            PERFORM pg_notify('task_available', 'session_actor@' || NEW.preferred_worker);
        ELSE
            PERFORM pg_notify('task_available', 'session_actor');
        END IF;
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

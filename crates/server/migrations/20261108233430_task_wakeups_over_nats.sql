-- Task wake-up NOTIFYs are skipped on connections whose server sends wake-ups over NATS.
--
-- A transaction that calls pg_notify takes one database-wide lock at commit and
-- holds it until its commit record is flushed, so every notifying commit in the
-- database waits for the one before it. In the stress bench this lock was the
-- second largest database wait (COMMIT on Lock:object, about a quarter of
-- database load at 100 agents).
--
-- Under NATS nobody listens on `task_available`: the server's broadcaster is
-- the NATS one, and the server publishes the wake-up itself once the
-- transaction commits. The trigger's NOTIFY was paid for and never heard.
--
-- Decision: the server marks its connections with
-- `everruns.task_wakeups = 'nats'` when NATS_URL is set (Database pool
-- options). The triggers skip the NOTIFY, and the worker-readiness lookup
-- that chooses its payload, only on such connections. Every other connection
-- (PostgreSQL LISTEN deployments, tools, tests, psql) notifies as before.
-- A server that configured NATS but fell back to PostgreSQL LISTEN at
-- startup hears no trigger wake-ups from marked connections; workers still
-- find runnable work within their one-second poll.

CREATE OR REPLACE FUNCTION public.task_wakeups_via_postgres()
RETURNS BOOLEAN
LANGUAGE sql STABLE AS $$
    SELECT current_setting('everruns.task_wakeups', true) IS DISTINCT FROM 'nats'
$$;

CREATE OR REPLACE FUNCTION notify_session_actor()
RETURNS TRIGGER AS $$
BEGIN
    IF NOT public.task_wakeups_via_postgres() THEN
        RETURN NULL;
    END IF;
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

CREATE OR REPLACE FUNCTION public.notify_task_available() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    -- Notify with activity_type as payload for filtering. A named queue's
    -- task wakes no default-queue listener: none of them could claim it.
    IF NEW.queue IS NULL AND public.task_wakeups_via_postgres() THEN
        PERFORM pg_notify('task_available', NEW.activity_type);
    END IF;
    RETURN NEW;
END;
$$;

-- Named task queues for durable tasks.
--
-- A task row records the queue it was enqueued to (`ActivityOptions::queue`);
-- NULL is the default queue, which every existing task and every task the
-- server and its workers enqueue stays in. A claim takes tasks from one queue
-- only, so processes that share the store but cannot run each other's tasks
-- (a framework `DurableBackend` runs only its own sessions' steps) keep them
-- apart by queue instead of by tagging the activity type.
--
-- Nullable with no default, so adding it rewrites nothing. The crate's own
-- schema (crates/durable/schema/postgres.sql) carries the same change.

ALTER TABLE durable_task_queue ADD COLUMN queue TEXT;

-- Workers LISTEN for task_available by activity type and claim from the
-- default queue, so a task in a named queue must not wake them.
CREATE OR REPLACE FUNCTION notify_task_available()
RETURNS TRIGGER AS $$
BEGIN
    -- Notify with activity_type as payload for filtering. A named queue's
    -- task wakes no default-queue listener: none of them could claim it.
    IF NEW.queue IS NULL THEN
        PERFORM pg_notify('task_available', NEW.activity_type);
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

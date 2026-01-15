-- Push-based task notifications via PostgreSQL NOTIFY
-- Enables low-latency task pickup (<10ms) by notifying workers when tasks are enqueued

-- Create notification function for task enqueue
CREATE OR REPLACE FUNCTION notify_task_available()
RETURNS TRIGGER AS $$
BEGIN
    -- Notify with activity_type as payload for filtering
    PERFORM pg_notify('task_available', NEW.activity_type);
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- Trigger on task insert (new tasks) and update to pending (retries)
CREATE TRIGGER task_enqueue_notify
    AFTER INSERT ON durable_task_queue
    FOR EACH ROW
    WHEN (NEW.status = 'pending')
    EXECUTE FUNCTION notify_task_available();

-- Also notify when tasks are set back to pending (retries, reclaims)
CREATE TRIGGER task_pending_notify
    AFTER UPDATE ON durable_task_queue
    FOR EACH ROW
    WHEN (OLD.status != 'pending' AND NEW.status = 'pending')
    EXECUTE FUNCTION notify_task_available();

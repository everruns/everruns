-- Event push notifications via PostgreSQL LISTEN/NOTIFY
--
-- Replaces SSE polling (100-500ms intervals) with push-based notifications
-- for low-latency event delivery (<20ms). Mirrors the pattern used by
-- durable task queue (notify_task_available).
--
-- Channel: 'event_available'
-- Payload: session_id UUID (allows per-session subscriber filtering)

CREATE OR REPLACE FUNCTION notify_event_available()
RETURNS TRIGGER AS $$
BEGIN
    -- Notify with session_id as payload for subscriber filtering
    PERFORM pg_notify('event_available', NEW.session_id::text);
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- Trigger on event insert
CREATE TRIGGER event_insert_notify
    AFTER INSERT ON events
    FOR EACH ROW
    EXECUTE FUNCTION notify_event_available();

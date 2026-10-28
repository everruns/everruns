-- Event writes no longer NOTIFY from a trigger. Each NOTIFY makes its
-- transaction take one database-wide lock at commit, so every event write in
-- the cluster committed one at a time, listener or not. The server now sends
-- `event_available` itself, only when the PostgreSQL SSE listener is in use
-- (crates/server/src/storage/repositories/events/mod.rs,
-- `notify_event_available`).
DROP TRIGGER IF EXISTS event_insert_notify ON public.events;
DROP FUNCTION IF EXISTS public.notify_event_available();

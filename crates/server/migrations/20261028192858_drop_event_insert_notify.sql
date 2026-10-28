-- Event writes no longer NOTIFY `event_available`. Each NOTIFY makes its
-- transaction take one database-wide lock at commit, listener or not, so every
-- event write in the cluster committed one at a time; production at 500
-- concurrent agents spent more than half its database load waiting for it.
-- Nothing read the notification: live delivery goes through EventDelivery
-- (NATS, or in-memory in a single process), and its subscribe never fails over
-- to the PostgreSQL listener that consumed this channel, which is removed too.
DROP TRIGGER IF EXISTS event_insert_notify ON public.events;
DROP FUNCTION IF EXISTS public.notify_event_available();

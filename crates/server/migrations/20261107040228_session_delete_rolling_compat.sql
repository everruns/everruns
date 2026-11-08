-- Install before 20261107040229 drops the hot-path foreign keys: the previous
-- release still serves DELETEs during a rolling upgrade and relies on cascades.
-- This new prerequisite deliberately sorts before the already-merged migration;
-- SQLx also applies it when that successor has already run. Neither successor
-- SQL nor its recorded checksum changes.
--
-- Keep the append-only guard: callers must still set app.session_purge in their
-- transaction. This invoker trigger grants no permissions and changes no flags.
CREATE FUNCTION public.cleanup_session_rows_for_legacy_delete() RETURNS trigger
LANGUAGE plpgsql SET search_path = pg_catalog, public AS $$
BEGIN
    DELETE FROM public.session_trace_steps WHERE session_id = OLD.id;
    DELETE FROM public.session_trace_turns WHERE session_id = OLD.id;
    DELETE FROM public.llm_generations WHERE session_id = OLD.id;
    DELETE FROM public.notification_turn_requests WHERE session_id = OLD.id;
    DELETE FROM public.runtime_invocations WHERE session_id = OLD.id;
    DELETE FROM public.session_actor_lease_history WHERE session_id = OLD.id;
    DELETE FROM public.events WHERE session_id = OLD.id;
    RETURN OLD;
END;
$$;

-- AFTER runs once the parent is gone: event deletion updates parent counters,
-- which must not modify the tuple while a BEFORE DELETE is still deleting it.
-- Row triggers also cover cascaded child sessions. New application cleanup may
-- already have removed these rows; empty DELETEs remain harmless.
CREATE TRIGGER sessions_unkeyed_delete_compat AFTER DELETE ON public.sessions
    FOR EACH ROW EXECUTE FUNCTION public.cleanup_session_rows_for_legacy_delete();

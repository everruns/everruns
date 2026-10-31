-- Tell every server when the worker ingress protection floor is set or
-- raised, so an in-memory copy is cleared on commit, whoever wrote the row
-- (crates/server/src/storage/backend/config_row_cache.rs). Delete and
-- truncate are refused by the retention triggers; they notify too so a
-- future change to that rule cannot leave a stale copy.
CREATE FUNCTION public.notify_worker_ingress_floor_changed() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    PERFORM pg_notify('config_rows_changed', 'worker_ingress_floor');
    RETURN NULL;
END;
$$;

CREATE TRIGGER worker_ingress_floor_change_notify
    AFTER INSERT OR UPDATE OR DELETE ON public.worker_ingress_protection_floor
    FOR EACH ROW EXECUTE FUNCTION public.notify_worker_ingress_floor_changed();

CREATE TRIGGER worker_ingress_floor_truncate_notify
    AFTER TRUNCATE ON public.worker_ingress_protection_floor
    FOR EACH STATEMENT EXECUTE FUNCTION public.notify_worker_ingress_floor_changed();

-- Tell every server when an organization's observers change, so the list of
-- active observers the event listener matches against is dropped from memory
-- on commit, whoever wrote it
-- (crates/server/src/storage/backend/config_row_cache.rs). The payload is
-- `observers:<org_id>`. Observers are only written by people configuring them,
-- never on the turn path.
CREATE TRIGGER observers_config_change_notify
    AFTER INSERT OR UPDATE OR DELETE ON public.observers
    FOR EACH ROW EXECUTE FUNCTION public.notify_config_rows_changed('observers', 'org_id');

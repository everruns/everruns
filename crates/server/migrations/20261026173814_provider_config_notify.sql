-- Tell every server when a provider row changes, so the copy that turn
-- execution reads from memory is dropped on commit
-- (crates/server/src/storage/backend/config_row_cache.rs).
CREATE TRIGGER providers_config_change_notify
    AFTER UPDATE OR DELETE ON public.providers
    FOR EACH ROW EXECUTE FUNCTION public.notify_config_rows_changed('provider', 'id');

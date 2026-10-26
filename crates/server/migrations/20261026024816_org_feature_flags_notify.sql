-- Tell every server when an organisation's feature flag overrides change, so
-- an in-memory copy is cleared on commit, whoever wrote the row
-- (crates/server/src/storage/backend/org_feature_flag_cache.rs).
CREATE FUNCTION public.notify_org_feature_flags_changed() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    PERFORM pg_notify(
        'org_feature_flags_changed',
        COALESCE(NEW.org_id, OLD.org_id)::text
    );
    RETURN NULL;
END;
$$;

CREATE TRIGGER org_feature_flags_change_notify
    AFTER INSERT OR UPDATE OR DELETE ON public.org_feature_flags
    FOR EACH ROW EXECUTE FUNCTION public.notify_org_feature_flags_changed();

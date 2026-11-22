-- dp_pushed_session_settings
--
-- What a data plane resolves on its own once host calls are gone
-- (`domains/data_plane/org_settings.rs`): the helper model catalog (an
-- org's models, providers and model presets) and each member's own tool
-- approval settings are pushed with the org's config. A change to any of
-- them owes the org a sync, counted in the same transaction as the change
-- (`dp_orgs.settings_rev`), as the org settings already are.

CREATE FUNCTION public.dp_org_catalog_changed() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    UPDATE dp_orgs SET settings_rev = settings_rev + 1
    WHERE org_id = COALESCE(NEW.org_id, OLD.org_id);
    RETURN NULL;
END;
$$;

CREATE TRIGGER dp_org_models_changed
    AFTER INSERT OR UPDATE OR DELETE ON public.models
    FOR EACH ROW EXECUTE FUNCTION public.dp_org_catalog_changed();

CREATE TRIGGER dp_org_providers_changed
    AFTER INSERT OR UPDATE OR DELETE ON public.providers
    FOR EACH ROW EXECUTE FUNCTION public.dp_org_catalog_changed();

CREATE TRIGGER dp_org_model_presets_changed
    AFTER INSERT OR UPDATE OR DELETE ON public.model_presets
    FOR EACH ROW EXECUTE FUNCTION public.dp_org_catalog_changed();

-- A member joining or leaving adds or drops their own settings.
CREATE TRIGGER dp_org_members_changed
    AFTER INSERT OR UPDATE OR DELETE ON public.organization_members
    FOR EACH ROW EXECUTE FUNCTION public.dp_org_catalog_changed();

-- A person's own approval settings (preference `tool_approval`) reach
-- every org they are a member of.
CREATE FUNCTION public.dp_user_approval_changed() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    IF COALESCE(NEW.key, OLD.key) = 'tool_approval' THEN
        UPDATE dp_orgs SET settings_rev = settings_rev + 1
        WHERE org_id IN (
            SELECT org_id FROM organization_members
            WHERE user_id = COALESCE(NEW.user_id, OLD.user_id)
        );
    END IF;
    RETURN NULL;
END;
$$;

CREATE TRIGGER dp_user_approval_changed
    AFTER INSERT OR UPDATE OR DELETE ON public.user_preferences
    FOR EACH ROW EXECUTE FUNCTION public.dp_user_approval_changed();

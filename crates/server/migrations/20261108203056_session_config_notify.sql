-- Tell every server when a session's configuration columns change, so the
-- in-memory copy the budget and sandbox listeners read is dropped
-- (crates/server/src/storage/backend/config_row_cache.rs).
--
-- Status, counters, titles and the other columns that move every turn are
-- not in the list, so the hot updates of a turn never fire this trigger.
CREATE TRIGGER sessions_config_change_notify
    AFTER UPDATE OF org_id, harness_id, agent_id, capabilities, trigger_id, channel_id,
        source, root_session_id, owner_principal_id, resolved_owner_user_id
    ON public.sessions
    FOR EACH ROW
    WHEN (
        OLD.org_id IS DISTINCT FROM NEW.org_id
        OR OLD.harness_id IS DISTINCT FROM NEW.harness_id
        OR OLD.agent_id IS DISTINCT FROM NEW.agent_id
        OR OLD.capabilities IS DISTINCT FROM NEW.capabilities
        OR OLD.trigger_id IS DISTINCT FROM NEW.trigger_id
        OR OLD.channel_id IS DISTINCT FROM NEW.channel_id
        OR OLD.source IS DISTINCT FROM NEW.source
        OR OLD.root_session_id IS DISTINCT FROM NEW.root_session_id
        OR OLD.owner_principal_id IS DISTINCT FROM NEW.owner_principal_id
        OR OLD.resolved_owner_user_id IS DISTINCT FROM NEW.resolved_owner_user_id
    )
    EXECUTE FUNCTION public.notify_config_rows_changed('session', 'id');

CREATE TRIGGER sessions_config_delete_notify
    AFTER DELETE ON public.sessions
    FOR EACH ROW EXECUTE FUNCTION public.notify_config_rows_changed('session', 'id');

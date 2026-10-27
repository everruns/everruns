-- Tell every server when an agent's or harness's configuration rows change, so
-- an in-memory copy is dropped on commit, whoever wrote the row
-- (crates/server/src/storage/backend/config_row_cache.rs). The payload is
-- `<kind>:<id>`; TG_ARGV names the kind and the column that holds the id.
-- PostgreSQL folds identical payloads within one transaction, so replacing an
-- agent's capabilities notifies once.
CREATE FUNCTION public.notify_config_rows_changed() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    IF TG_OP <> 'INSERT' THEN
        PERFORM pg_notify(
            'config_rows_changed',
            TG_ARGV[0] || ':' || (to_jsonb(OLD) ->> TG_ARGV[1])
        );
    END IF;
    IF TG_OP <> 'DELETE' THEN
        PERFORM pg_notify(
            'config_rows_changed',
            TG_ARGV[0] || ':' || (to_jsonb(NEW) ->> TG_ARGV[1])
        );
    END IF;
    RETURN NULL;
END;
$$;

-- Usage counters change on every model call and are not configuration; a
-- cached agent row carries them as of its load. `updated_at` is excluded too:
-- the update_agents_updated_at trigger bumps it on those same counter writes.
CREATE TRIGGER agents_config_change_notify
    AFTER UPDATE ON public.agents
    FOR EACH ROW
    WHEN (
        (to_jsonb(OLD) - ARRAY[
            'total_input_tokens', 'total_output_tokens', 'total_cache_read_tokens',
            'total_cache_creation_tokens', 'total_actual_cost_usd',
            'total_estimated_cost_usd', 'total_cost_usd', 'updated_at'
        ])
        IS DISTINCT FROM
        (to_jsonb(NEW) - ARRAY[
            'total_input_tokens', 'total_output_tokens', 'total_cache_read_tokens',
            'total_cache_creation_tokens', 'total_actual_cost_usd',
            'total_estimated_cost_usd', 'total_cost_usd', 'updated_at'
        ])
    )
    EXECUTE FUNCTION public.notify_config_rows_changed('agent', 'id');

CREATE TRIGGER agents_config_delete_notify
    AFTER DELETE ON public.agents
    FOR EACH ROW EXECUTE FUNCTION public.notify_config_rows_changed('agent', 'id');

CREATE TRIGGER agent_capabilities_change_notify
    AFTER INSERT OR UPDATE OR DELETE ON public.agent_capabilities
    FOR EACH ROW EXECUTE FUNCTION public.notify_config_rows_changed('agent', 'agent_id');

CREATE TRIGGER harnesses_change_notify
    AFTER UPDATE OR DELETE ON public.harnesses
    FOR EACH ROW EXECUTE FUNCTION public.notify_config_rows_changed('harness', 'id');

CREATE TRIGGER harness_capabilities_change_notify
    AFTER INSERT OR UPDATE OR DELETE ON public.harness_capabilities
    FOR EACH ROW EXECUTE FUNCTION public.notify_config_rows_changed('harness', 'harness_id');

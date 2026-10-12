-- Baseline: the schema and seed rows that legacy migrations 001..222
-- (crates/server/migrations/legacy/) produce on an empty database, generated
-- with pg_dump on 2026-10-11. A fresh database runs this one file instead of
-- replaying the legacy history. A database that already ran the legacy history
-- is adopted instead (crates/server/src/setup/migrations.rs) and never runs it.
-- Never edit: add a new timestamped migration.

SET LOCAL check_function_bodies = false;
SET LOCAL client_min_messages = warning;

CREATE EXTENSION IF NOT EXISTS pgcrypto WITH SCHEMA public;

CREATE FUNCTION public.agents_api_tombstone_provider_session() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        INSERT INTO agents_api_provider_deletions
            (org_id, session_id, provider_key, provider_session_id, reason)
        VALUES (OLD.org_id, OLD.session_id, OLD.provider_key, OLD.provider_session_id,
                'session_deleted')
        ON CONFLICT (provider_session_id) DO NOTHING;
        RETURN OLD;
    END IF;
    INSERT INTO agents_api_provider_deletions
        (org_id, session_id, provider_key, provider_session_id, reason)
    VALUES (OLD.org_id, OLD.session_id, OLD.provider_key, OLD.provider_session_id,
            CASE WHEN NEW.provider_session_id IS NULL THEN 'released' ELSE 'replaced' END)
    ON CONFLICT (provider_session_id) DO NOTHING;
    RETURN NEW;
END;
$$;

CREATE FUNCTION public.allocate_event_sequence(p_session_id uuid) RETURNS integer
    LANGUAGE plpgsql
    AS $$
DECLARE
    allocated_seq INTEGER;
BEGIN
    INSERT INTO event_sequences (session_id, next_sequence, updated_at)
    VALUES (p_session_id, 2, NOW())
    ON CONFLICT (session_id) DO UPDATE
    SET next_sequence = event_sequences.next_sequence + 1,
        updated_at = NOW()
    RETURNING next_sequence - 1 INTO allocated_seq;

    RETURN allocated_seq;
END;
$$;

COMMENT ON FUNCTION public.allocate_event_sequence(p_session_id uuid) IS 'Atomically allocate the next event sequence number for a session';

CREATE FUNCTION public.durable_task_stat_counters() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
DECLARE
    d_completed BIGINT := 0;
    d_failed BIGINT := 0;
    d_started BIGINT := 0;
    my_shard SMALLINT := pg_backend_pid() % 16;
BEGIN
    IF TG_OP IN ('INSERT', 'UPDATE') THEN
        SELECT
            count(*) FILTER (WHERE status = 'completed'),
            count(*) FILTER (WHERE status IN ('failed', 'dead')),
            count(*) FILTER (WHERE claimed_at IS NOT NULL)
        INTO d_completed, d_failed, d_started
        FROM new_rows;
    END IF;

    IF TG_OP IN ('UPDATE', 'DELETE') THEN
        d_completed := d_completed - (SELECT count(*) FROM old_rows WHERE status = 'completed');
        d_failed := d_failed - (SELECT count(*) FROM old_rows WHERE status IN ('failed', 'dead'));
        d_started := d_started - (SELECT count(*) FROM old_rows WHERE claimed_at IS NOT NULL);
    END IF;

    IF d_completed <> 0 THEN
        UPDATE durable_stat_counters SET value = value + d_completed WHERE name = 'tasks_completed' AND shard = my_shard;
    END IF;
    IF d_failed <> 0 THEN
        UPDATE durable_stat_counters SET value = value + d_failed WHERE name = 'tasks_failed' AND shard = my_shard;
    END IF;
    IF d_started <> 0 THEN
        UPDATE durable_stat_counters SET value = value + d_started WHERE name = 'tasks_started' AND shard = my_shard;
    END IF;

    RETURN NULL; -- AFTER trigger: return value is ignored
END;
$$;

CREATE FUNCTION public.durable_workflow_stat_counters() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
DECLARE
    d_completed BIGINT := 0;
    d_failed BIGINT := 0;
    d_started BIGINT := 0;
    my_shard SMALLINT := pg_backend_pid() % 16;
BEGIN
    IF TG_OP IN ('INSERT', 'UPDATE') THEN
        SELECT
            count(*) FILTER (WHERE status = 'completed'),
            count(*) FILTER (WHERE status IN ('failed', 'cancelled')),
            count(*) FILTER (WHERE started_at IS NOT NULL)
        INTO d_completed, d_failed, d_started
        FROM new_rows;
    END IF;

    IF TG_OP IN ('UPDATE', 'DELETE') THEN
        d_completed := d_completed - (SELECT count(*) FROM old_rows WHERE status = 'completed');
        d_failed := d_failed - (SELECT count(*) FROM old_rows WHERE status IN ('failed', 'cancelled'));
        d_started := d_started - (SELECT count(*) FROM old_rows WHERE started_at IS NOT NULL);
    END IF;

    IF d_completed <> 0 THEN
        UPDATE durable_stat_counters SET value = value + d_completed WHERE name = 'workflows_completed' AND shard = my_shard;
    END IF;
    IF d_failed <> 0 THEN
        UPDATE durable_stat_counters SET value = value + d_failed WHERE name = 'workflows_failed' AND shard = my_shard;
    END IF;
    IF d_started <> 0 THEN
        UPDATE durable_stat_counters SET value = value + d_started WHERE name = 'workflows_started' AND shard = my_shard;
    END IF;

    RETURN NULL;
END;
$$;

CREATE FUNCTION public.enforce_agent_service_virtual_user() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    IF NEW.virtual_user_id IS NOT NULL AND NOT EXISTS (
        SELECT 1 FROM virtual_users WHERE id=NEW.virtual_user_id AND org_id=NEW.org_id AND usage='service' AND status='active'
    ) THEN
        RAISE EXCEPTION 'Agent requires an active service virtual user in its organization';
    END IF;
    RETURN NEW;
END;
$$;

CREATE FUNCTION public.enforce_channel_virtual_user_org() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    IF NEW.virtual_user_id IS NOT NULL AND NOT EXISTS (
        SELECT 1 FROM virtual_users v JOIN agents a ON a.org_id = v.org_id
        WHERE v.id = NEW.virtual_user_id AND a.id = NEW.agent_id
    ) THEN
        RAISE EXCEPTION 'Channel virtual user must belong to the agent organization';
    END IF;
    RETURN NEW;
END;
$$;

CREATE FUNCTION public.event_sequences_removed_count_delete() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    UPDATE event_sequences es
    SET removed_count = es.removed_count + delta.removed
    FROM (
        SELECT session_id, count(*) AS removed
        FROM old_rows
        GROUP BY session_id
    ) delta
    WHERE es.session_id = delta.session_id;
    RETURN NULL;
END;
$$;

CREATE FUNCTION public.keep_platform_chat_starter_tag() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    IF 'platform-chat-starter' = ANY(OLD.tags)
       AND NOT 'platform-chat-starter' = ANY(NEW.tags) THEN
        NEW.tags := array_append(NEW.tags, 'platform-chat-starter');
    END IF;
    RETURN NEW;
END;
$$;

CREATE FUNCTION public.mark_platform_chat_starter_tag() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    IF EXISTS (SELECT 1 FROM agents a WHERE a.id = NEW.agent_id AND a.org_id = NEW.org_id
               AND a.is_built_in AND a.name = 'platform-chat')
       AND NOT 'platform-chat-starter' = ANY(NEW.tags) THEN
        NEW.tags := array_append(NEW.tags, 'platform-chat-starter');
    END IF;
    RETURN NEW;
END;
$$;

CREATE FUNCTION public.next_event_sequence(p_session_id uuid) RETURNS integer
    LANGUAGE plpgsql
    AS $$
DECLARE
    next_seq INTEGER;
BEGIN
    SELECT COALESCE(MAX(sequence), 0) + 1 INTO next_seq
    FROM events WHERE session_id = p_session_id;
    RETURN next_seq;
END;
$$;

CREATE FUNCTION public.notify_event_available() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    -- Notify with session_id as payload for subscriber filtering
    PERFORM pg_notify('event_available', NEW.session_id::text);
    RETURN NEW;
END;
$$;

CREATE FUNCTION public.notify_notification_available() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    PERFORM pg_notify(
        'notification_available',
        json_build_object(
            'org_id', NEW.org_id,
            'user_id', NEW.user_id,
            'action', lower(TG_OP)
        )::text
    );
    RETURN NEW;
END;
$$;

CREATE FUNCTION public.notify_task_available() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    -- Notify with activity_type as payload for filtering. A named queue's
    -- task wakes no default-queue listener: none of them could claim it.
    IF NEW.queue IS NULL THEN
        PERFORM pg_notify('task_available', NEW.activity_type);
    END IF;
    RETURN NEW;
END;
$$;

CREATE FUNCTION public.prevent_event_mutation() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    IF TG_OP = 'DELETE'
       AND TG_TABLE_NAME = 'events'
       AND (current_setting('app.archival_bypass', true) = 'true'
            OR current_setting('app.session_purge', true) = 'true') THEN
        RETURN OLD;
    END IF;
    RAISE EXCEPTION 'events are append-only: % operations are not allowed', TG_OP;
END;
$$;

COMMENT ON FUNCTION public.prevent_event_mutation() IS 'Enforces append-only semantics. On events, DELETE is permitted under app.archival_bypass (retention archival) or app.session_purge (session deletion); every other table using this function is strictly append-only.';

CREATE FUNCTION public.prevent_usage_record_mutation() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
DECLARE
    reference_columns CONSTANT text[] := ARRAY[
        'budget_id', 'event_id', 'session_id',
        'user_id', 'principal_id', 'agent_id', 'harness_id'
    ];
    detached integer;
    rewritten integer;
BEGIN
    IF TG_OP = 'UPDATE' THEN
        SELECT
            count(*) FILTER (
                WHERE new_field.value = 'null'::jsonb
                  AND old_field.key = ANY (reference_columns)
            ),
            count(*) FILTER (
                WHERE new_field.value <> 'null'::jsonb
                   OR NOT old_field.key = ANY (reference_columns)
            )
        INTO detached, rewritten
        FROM jsonb_each(to_jsonb(OLD)) AS old_field
        JOIN jsonb_each(to_jsonb(NEW)) AS new_field ON new_field.key = old_field.key
        WHERE old_field.value IS DISTINCT FROM new_field.value;

        IF detached > 0 AND rewritten = 0 THEN
            RETURN NEW;
        END IF;
    END IF;

    RAISE EXCEPTION 'usage records are append-only: % operations are not allowed', TG_OP;
END;
$$;

COMMENT ON FUNCTION public.prevent_usage_record_mutation() IS 'Enforces append-only semantics on usage_journal and usage_ledger, except for ON DELETE SET NULL cascades that detach a deleted session, event, or other referenced row.';

CREATE FUNCTION public.remove_health_issue_notifications() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    DELETE FROM notifications
    WHERE org_id = OLD.org_id AND kind = 'health.issue'
      AND target_id = OLD.id::text;
    RETURN OLD;
END;
$$;

CREATE FUNCTION public.sandboxes_record_transition() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    INSERT INTO sandbox_state_transitions (sandbox_id, org_id, generation, state)
    VALUES (NEW.id, NEW.org_id, NEW.generation, NEW.observed_state);
    RETURN NULL;
END;
$$;

CREATE FUNCTION public.sandboxes_stamp_deleted_at() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    IF NEW.observed_state = 'deleted' THEN
        IF TG_OP = 'INSERT' OR OLD.observed_state IS DISTINCT FROM 'deleted' THEN
            NEW.deleted_at := now();
        END IF;
    ELSE
        NEW.deleted_at := NULL;
    END IF;
    RETURN NEW;
END;
$$;

CREATE FUNCTION public.session_files_name(file_path text) RETURNS text
    LANGUAGE plpgsql IMMUTABLE
    AS $_$
BEGIN
    IF file_path = '/' THEN
        RETURN '/';
    END IF;
    RETURN substring(file_path from '[^/]+$');
END;
$_$;

CREATE FUNCTION public.session_files_parent_path(file_path text) RETURNS text
    LANGUAGE plpgsql IMMUTABLE
    AS $_$
BEGIN
    IF file_path = '/' THEN
        RETURN NULL;
    END IF;
    RETURN COALESCE(substring(file_path from '^(.*)/[^/]+$'), '/');
END;
$_$;

CREATE FUNCTION public.sessions_last_turn_delete() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    UPDATE sessions s
    SET last_turn_status = CASE last_turn.event_type
            WHEN 'turn.completed' THEN 'completed'
            WHEN 'turn.failed' THEN 'failed'
            WHEN 'turn.cancelled' THEN 'cancelled'
        END,
        last_turn_at = last_turn.ts,
        last_turn_sequence = last_turn.sequence
    FROM (
        SELECT affected.session_id, recomputed.event_type, recomputed.ts, recomputed.sequence
        FROM (SELECT DISTINCT session_id FROM old_rows
               WHERE event_type IN ('turn.completed', 'turn.failed', 'turn.cancelled')) affected
        LEFT JOIN LATERAL (
            SELECT e.event_type, e.ts, e.sequence
            FROM events e
            WHERE e.session_id = affected.session_id
              AND e.event_type IN ('turn.completed', 'turn.failed', 'turn.cancelled')
            ORDER BY e.sequence DESC
            LIMIT 1
        ) recomputed ON TRUE
    ) last_turn
    WHERE s.id = last_turn.session_id;
    RETURN NULL;
END;
$$;

CREATE FUNCTION public.sessions_reporting_event_counts_delete() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    UPDATE sessions s
    SET
        turn_count = GREATEST(s.turn_count - delta.turn_count, 0),
        tool_call_count = GREATEST(s.tool_call_count - delta.tool_call_count, 0)
    FROM (
        SELECT
            session_id,
            count(*) FILTER (
                WHERE event_type IN ('turn.completed', 'turn.failed', 'turn.cancelled')
            ) AS turn_count,
            count(*) FILTER (WHERE event_type = 'tool.completed') AS tool_call_count
        FROM old_rows
        GROUP BY session_id
    ) delta
    WHERE s.id = delta.session_id
      AND (delta.turn_count > 0 OR delta.tool_call_count > 0);
    RETURN NULL;
END;
$$;

CREATE FUNCTION public.sessions_task_count_delete() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    UPDATE sessions s
    SET task_count = GREATEST(s.task_count - delta.task_count, 0)
    FROM (
        SELECT session_id, count(*) AS task_count
        FROM old_rows
        GROUP BY session_id
    ) delta
    WHERE s.id = delta.session_id;
    RETURN NULL;
END;
$$;

CREATE FUNCTION public.sessions_task_count_insert() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    UPDATE sessions s
    SET task_count = s.task_count + delta.task_count
    FROM (
        SELECT session_id, count(*) AS task_count
        FROM new_rows
        GROUP BY session_id
    ) delta
    WHERE s.id = delta.session_id;
    RETURN NULL;
END;
$$;

CREATE FUNCTION public.sessions_turn_finished() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    UPDATE sessions s
    SET
        turn_count = s.turn_count + 1,
        tool_call_count = s.tool_call_count + (
            SELECT count(*)
            FROM events e
            WHERE e.session_id = NEW.session_id
              AND e.sequence > COALESCE(s.last_turn_sequence, 0)
              AND e.sequence < NEW.sequence
              AND e.event_type = 'tool.completed'
        ),
        -- Events are append-only, so a higher sequence is strictly newer, and
        -- an out-of-order replay never moves the pointer backwards (118).
        last_turn_status = CASE
            WHEN s.last_turn_sequence IS NULL OR NEW.sequence > s.last_turn_sequence
            THEN CASE NEW.event_type
                WHEN 'turn.completed' THEN 'completed'
                WHEN 'turn.failed' THEN 'failed'
                WHEN 'turn.cancelled' THEN 'cancelled'
            END
            ELSE s.last_turn_status
        END,
        last_turn_at = CASE
            WHEN s.last_turn_sequence IS NULL OR NEW.sequence > s.last_turn_sequence
            THEN NEW.ts
            ELSE s.last_turn_at
        END,
        last_turn_sequence = GREATEST(s.last_turn_sequence, NEW.sequence)
    WHERE s.id = NEW.session_id;
    RETURN NULL;
END;
$$;

CREATE FUNCTION public.sync_user_oauth_identity() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    IF NEW.auth_provider IS NOT NULL AND NEW.auth_provider_id IS NOT NULL THEN
        INSERT INTO user_oauth_identities (user_id, provider, provider_id)
        VALUES (NEW.id, NEW.auth_provider, NEW.auth_provider_id)
        ON CONFLICT DO NOTHING;

        IF NOT EXISTS (
            SELECT 1
            FROM user_oauth_identities
            WHERE user_id = NEW.id
              AND provider = NEW.auth_provider
              AND provider_id = NEW.auth_provider_id
        ) THEN
            RAISE EXCEPTION 'OAuth identity conflicts with an existing binding'
                USING ERRCODE = 'unique_violation';
        END IF;
    END IF;
    RETURN NEW;
END;
$$;

CREATE FUNCTION public.update_durable_updated_at() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$;

CREATE FUNCTION public.update_updated_at_column() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$;

CREATE FUNCTION public.uuidv7() RETURNS uuid
    LANGUAGE plpgsql
    AS $$
        DECLARE
            timestamp_ms BIGINT;
            uuid_bytes BYTEA;
        BEGIN
            timestamp_ms := (EXTRACT(EPOCH FROM clock_timestamp()) * 1000)::BIGINT;
            uuid_bytes := gen_random_bytes(16);
            uuid_bytes := set_byte(uuid_bytes, 0, ((timestamp_ms >> 40) & 255)::INT);
            uuid_bytes := set_byte(uuid_bytes, 1, ((timestamp_ms >> 32) & 255)::INT);
            uuid_bytes := set_byte(uuid_bytes, 2, ((timestamp_ms >> 24) & 255)::INT);
            uuid_bytes := set_byte(uuid_bytes, 3, ((timestamp_ms >> 16) & 255)::INT);
            uuid_bytes := set_byte(uuid_bytes, 4, ((timestamp_ms >> 8) & 255)::INT);
            uuid_bytes := set_byte(uuid_bytes, 5, (timestamp_ms & 255)::INT);
            uuid_bytes := set_byte(uuid_bytes, 6, (get_byte(uuid_bytes, 6) & 15) | 112);
            uuid_bytes := set_byte(uuid_bytes, 8, (get_byte(uuid_bytes, 8) & 63) | 128);
            RETURN encode(uuid_bytes, 'hex')::uuid;
        END;
        $$;

CREATE FUNCTION public.workspaces_file_count_delete() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    UPDATE workspaces w
    SET file_count = GREATEST(w.file_count - delta.file_count, 0)
    FROM (
        SELECT workspace_id, count(*) AS file_count
        FROM old_rows
        WHERE NOT is_directory
        GROUP BY workspace_id
    ) delta
    WHERE w.id = delta.workspace_id;
    RETURN NULL;
END;
$$;

CREATE FUNCTION public.workspaces_file_count_insert() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    UPDATE workspaces w
    SET file_count = w.file_count + delta.file_count
    FROM (
        SELECT workspace_id, count(*) AS file_count
        FROM new_rows
        WHERE NOT is_directory
        GROUP BY workspace_id
    ) delta
    WHERE w.id = delta.workspace_id;
    RETURN NULL;
END;
$$;

CREATE FUNCTION public.workspaces_file_count_update() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    UPDATE workspaces w
    SET file_count = GREATEST(w.file_count + delta.net, 0)
    FROM (
        SELECT
            workspace_id,
            sum(net) AS net
        FROM (
            SELECT workspace_id, count(*) AS net
            FROM new_rows
            WHERE NOT is_directory
            GROUP BY workspace_id
            UNION ALL
            SELECT workspace_id, -count(*) AS net
            FROM old_rows
            WHERE NOT is_directory
            GROUP BY workspace_id
        ) per_side
        GROUP BY workspace_id
    ) delta
    WHERE w.id = delta.workspace_id
      AND delta.net <> 0;
    RETURN NULL;
END;
$$;

CREATE TABLE public.a2a_push_configs (
    id uuid NOT NULL,
    org_id bigint NOT NULL,
    session_id uuid NOT NULL,
    config_id text NOT NULL,
    url text NOT NULL,
    auth_scheme text,
    secrets_encrypted bytea,
    wire_version text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE public.agent_avatar_variants (
    avatar_id uuid NOT NULL,
    variant text NOT NULL,
    content_type text NOT NULL,
    data bytea NOT NULL
);

CREATE TABLE public.agent_avatars (
    id uuid NOT NULL,
    org_id bigint NOT NULL,
    agent_id uuid NOT NULL,
    source text DEFAULT 'upload'::text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT agent_avatars_source_check CHECK (((source = 'upload'::text) OR (source ~ '^preset:[a-z]+-[a-z]+$'::text)))
);

CREATE TABLE public.agent_capabilities (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    agent_id uuid NOT NULL,
    capability_id character varying(71) NOT NULL,
    "position" integer DEFAULT 0 NOT NULL,
    config jsonb DEFAULT '{}'::jsonb NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

COMMENT ON COLUMN public.agent_capabilities.config IS 'Per-agent capability configuration (JSON object)';

CREATE TABLE public.agent_channels (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    agent_id uuid NOT NULL,
    app_id uuid,
    public_id text NOT NULL,
    channel_type character varying(50) NOT NULL,
    channel_config jsonb DEFAULT '{}'::jsonb NOT NULL,
    channel_config_encrypted bytea,
    durable_schedule_id uuid,
    enabled boolean DEFAULT true NOT NULL,
    status text DEFAULT 'draft'::text NOT NULL,
    virtual_user_id uuid,
    owner_principal_id uuid NOT NULL,
    resolved_owner_user_id uuid,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    auth jsonb,
    auth_encrypted bytea,
    legacy_alias_id text,
    CONSTRAINT agent_channels_channel_type_check CHECK (((channel_type)::text = ANY ((ARRAY['slack'::character varying, 'ag_ui'::character varying, 'schedule'::character varying, 'webhook'::character varying, 'a2a'::character varying, 'fcp'::character varying, 'api_endpoint'::character varying, 'public_chat'::character varying, 'voice'::character varying, 'api'::character varying, 'poppy'::character varying])::text[]))),
    CONSTRAINT agent_channels_public_id_format CHECK ((public_id ~ '^appchan_[0-9a-f]{32}$'::text)),
    CONSTRAINT agent_channels_status_check CHECK ((status = ANY (ARRAY['draft'::text, 'live'::text, 'disabled'::text])))
);

COMMENT ON TABLE public.agent_channels IS 'Agent-owned communication channels. Public IDs, archival App aliases and encrypted configuration are preserved.';

COMMENT ON COLUMN public.agent_channels.app_id IS 'Optional archival foreign key to the App that created this endpoint. Native Agent endpoints leave it null.';

COMMENT ON COLUMN public.agent_channels.legacy_alias_id IS 'Immutable alias identity (the {app_id} segment) for permanent /v1/apps/{app_id}/... ingress routes. Native Agent endpoints leave it null.';

CREATE TABLE public.agent_check_rules (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    org_id bigint NOT NULL,
    rule_id text NOT NULL,
    kind text NOT NULL,
    enabled boolean DEFAULT true NOT NULL,
    severity_override text,
    config jsonb,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT agent_check_rules_kind_check CHECK ((kind = ANY (ARRAY['builtin_override'::text, 'declarative'::text, 'nl_rubric'::text]))),
    CONSTRAINT agent_check_rules_severity_override_check CHECK ((severity_override = ANY (ARRAY['warning'::text, 'info'::text, 'suggestion'::text])))
);

CREATE TABLE public.agent_health_check_runs (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    org_id bigint NOT NULL,
    public_id text NOT NULL,
    agent_id uuid,
    config_hash text NOT NULL,
    model_id text,
    status character varying(50) DEFAULT 'pending'::character varying NOT NULL,
    summary jsonb,
    results jsonb,
    error_message text,
    started_at timestamp with time zone,
    completed_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT agent_health_check_runs_public_id_format CHECK ((public_id ~ '^healthcheck_[0-9a-f]{32}$'::text)),
    CONSTRAINT agent_health_check_runs_status_check CHECK (((status)::text = ANY ((ARRAY['pending'::character varying, 'running'::character varying, 'completed'::character varying, 'failed'::character varying])::text[])))
);

CREATE TABLE public.agent_keys (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    org_id bigint DEFAULT 1 NOT NULL,
    channel_id uuid NOT NULL,
    name character varying(128) NOT NULL,
    token_hash text NOT NULL,
    token_prefix text NOT NULL,
    previous_token_hash text,
    previous_valid_until timestamp with time zone,
    permissions text[] DEFAULT ARRAY['sessions'::text] NOT NULL,
    expires_at timestamp with time zone,
    last_used_at timestamp with time zone,
    created_by_user_id uuid,
    revoked_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE public.agent_mcp_secret_bindings (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    org_id bigint NOT NULL,
    agent_id uuid NOT NULL,
    mcp_server_name character varying(255) NOT NULL,
    mcp_server_url text NOT NULL,
    tool_name character varying(255) NOT NULL,
    parameter_name character varying(255) NOT NULL,
    label character varying(255) NOT NULL,
    description text,
    value_encrypted bytea,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);

COMMENT ON TABLE public.agent_mcp_secret_bindings IS 'Encrypted Agent-scoped MCP tool-argument credentials; values are write-only and injected after model tool-call persistence.';

CREATE TABLE public.agent_scripts (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    org_id bigint DEFAULT 1 NOT NULL,
    agent_id uuid NOT NULL,
    name character varying(64) NOT NULL,
    description text NOT NULL,
    input_schema jsonb,
    body text NOT NULL,
    status character varying(50) DEFAULT 'active'::character varying NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    archived_at timestamp with time zone,
    deleted_at timestamp with time zone,
    CONSTRAINT agent_scripts_status_check CHECK (((status)::text = ANY ((ARRAY['active'::character varying, 'archived'::character varying, 'deleted'::character varying])::text[])))
);

CREATE TABLE public.agent_trigger_deliveries (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    org_id bigint NOT NULL,
    trigger_id uuid NOT NULL,
    source character varying(50) NOT NULL,
    event_id text,
    event_type text,
    subject text,
    status character varying(20) NOT NULL,
    reason text,
    session_id uuid,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT agent_trigger_deliveries_status_check CHECK (((status)::text = ANY ((ARRAY['dispatched'::character varying, 'filtered'::character varying, 'duplicate'::character varying, 'failed'::character varying])::text[])))
);

CREATE TABLE public.agent_trigger_mcp_subscriptions (
    trigger_id uuid NOT NULL,
    org_id bigint NOT NULL,
    secret_encrypted bytea NOT NULL,
    remote_subscription_id text,
    refresh_before timestamp with time zone,
    cursor text,
    status character varying(20) DEFAULT 'pending'::character varying NOT NULL,
    last_error text,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT agent_trigger_mcp_subscriptions_status_check CHECK (((status)::text = ANY ((ARRAY['pending'::character varying, 'active'::character varying, 'failed'::character varying])::text[])))
);

CREATE TABLE public.agent_trigger_subject_values (
    trigger_id uuid NOT NULL,
    path text NOT NULL,
    subject text NOT NULL,
    value jsonb NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE public.agent_triggers (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    org_id bigint DEFAULT 1 NOT NULL,
    agent_id uuid NOT NULL,
    trigger_type character varying(50) DEFAULT 'schedule'::character varying NOT NULL,
    config jsonb DEFAULT '{}'::jsonb NOT NULL,
    enabled boolean DEFAULT true NOT NULL,
    durable_schedule_id uuid,
    status character varying(50) DEFAULT 'active'::character varying NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    archived_at timestamp with time zone,
    deleted_at timestamp with time zone,
    execution_harness_id uuid,
    execution_owner_principal_id uuid,
    execution_resolved_owner_user_id uuid,
    execution_virtual_user_id uuid,
    execution_app_id uuid,
    ingress_id character varying(100),
    config_encrypted bytea,
    legacy_alias_id text,
    legacy_alias_name text,
    CONSTRAINT agent_triggers_status_check CHECK (((status)::text = ANY ((ARRAY['active'::character varying, 'archived'::character varying, 'deleted'::character varying])::text[])))
);

COMMENT ON COLUMN public.agent_triggers.execution_app_id IS 'Archival (EVE-1011): the retired App a webhook trigger was migrated from; keeps its App-era name because it references the frozen apps table. Non-null marks a migrated trigger and is copied to sessions.app_id; NULL for native triggers.';

COMMENT ON COLUMN public.agent_triggers.legacy_alias_id IS 'Frozen permanent alias identity (the {app_id} segment of /v1/apps/{app_id}/webhooks/...) for webhook triggers migrated from Apps. Native triggers leave it null.';

COMMENT ON COLUMN public.agent_triggers.legacy_alias_name IS 'Frozen display name of the retired App, used by migrated webhook templates ({{app.name}}) and session titles. Native triggers leave it null.';

CREATE TABLE public.agentid_agent_owners (
    org_id bigint NOT NULL,
    virtual_user_id uuid NOT NULL,
    owner_sub text NOT NULL,
    owner_email text,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE public.agentid_login_states (
    state_hash bytea NOT NULL,
    org_id bigint NOT NULL,
    channel_id text NOT NULL,
    code_verifier text NOT NULL,
    nonce text NOT NULL,
    login_hint text,
    expires_at timestamp with time zone NOT NULL
);

CREATE TABLE public.agents (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    org_id bigint DEFAULT 1 NOT NULL,
    public_id text NOT NULL,
    name character varying(255) NOT NULL,
    description text,
    system_prompt text NOT NULL,
    default_model_id uuid,
    tools jsonb DEFAULT '[]'::jsonb NOT NULL,
    tags text[] DEFAULT '{}'::text[] NOT NULL,
    status character varying(50) DEFAULT 'active'::character varying NOT NULL,
    total_input_tokens bigint DEFAULT 0 NOT NULL,
    total_output_tokens bigint DEFAULT 0 NOT NULL,
    total_cache_read_tokens bigint DEFAULT 0 NOT NULL,
    total_cache_creation_tokens bigint DEFAULT 0 NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    archived_at timestamp with time zone,
    deleted_at timestamp with time zone,
    initial_files jsonb DEFAULT '[]'::jsonb NOT NULL,
    max_iterations integer,
    network_access jsonb,
    display_name text,
    mcp_servers jsonb DEFAULT '{}'::jsonb NOT NULL,
    forked_from_agent_id uuid,
    root_agent_id uuid,
    total_actual_cost_usd double precision DEFAULT 0 NOT NULL,
    total_estimated_cost_usd double precision DEFAULT 0 NOT NULL,
    total_cost_usd double precision DEFAULT 0 NOT NULL,
    parallel_tool_calls boolean,
    harness_id uuid NOT NULL,
    virtual_user_id uuid,
    harness_source character varying(32) DEFAULT 'explicit'::character varying NOT NULL,
    is_built_in boolean DEFAULT false NOT NULL,
    intro_markdown text,
    short_description text,
    starters jsonb DEFAULT '[]'::jsonb,
    exposures_suspended boolean DEFAULT false NOT NULL,
    environments jsonb,
    avatar_id uuid,
    communication text DEFAULT 'direct'::text NOT NULL,
    CONSTRAINT agents_communication_check CHECK ((communication = ANY (ARRAY['direct'::text, 'explicit'::text]))),
    CONSTRAINT agents_harness_source_check CHECK (((harness_source)::text = ANY ((ARRAY['explicit'::character varying, 'organization_default'::character varying])::text[]))),
    CONSTRAINT agents_public_id_format CHECK ((public_id ~ '^agent_[0-9a-f]{32}$'::text)),
    CONSTRAINT agents_status_check CHECK (((status)::text = ANY ((ARRAY['active'::character varying, 'archived'::character varying, 'deleted'::character varying])::text[])))
);

COMMENT ON COLUMN public.agents.total_input_tokens IS 'Denormalized: sum of input_tokens across all sessions';

COMMENT ON COLUMN public.agents.total_output_tokens IS 'Denormalized: sum of output_tokens across all sessions';

COMMENT ON COLUMN public.agents.total_cache_read_tokens IS 'Denormalized: sum of cache_read_tokens across all sessions';

COMMENT ON COLUMN public.agents.total_cache_creation_tokens IS 'Denormalized: sum of cache_creation_tokens across all sessions';

COMMENT ON COLUMN public.agents.initial_files IS 'Starter files copied into each new session for this agent';

COMMENT ON COLUMN public.agents.total_actual_cost_usd IS 'Denormalized: sum of actual_cost_usd across all sessions for this agent.';

COMMENT ON COLUMN public.agents.total_estimated_cost_usd IS 'Denormalized: sum of estimated_cost_usd across all sessions for this agent.';

COMMENT ON COLUMN public.agents.total_cost_usd IS 'Denormalized best-effort spend: sum of actual_cost_usd where present, else estimated_cost_usd, per generation.';

COMMENT ON COLUMN public.agents.is_built_in IS 'Platform-supplied agent: definition is immutable through the API and excluded from the per-org agent limit. Bindings (triggers, identities, credentials) remain editable.';

COMMENT ON COLUMN public.agents.environments IS 'Named authored Environment profiles. Immutable Agent versions copy this value into authored_config.';

CREATE TABLE public.agents_api_provider_deletions (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    org_id bigint NOT NULL,
    session_id uuid NOT NULL,
    provider_key text,
    provider_session_id text NOT NULL,
    reason text NOT NULL,
    state text DEFAULT 'pending'::text NOT NULL,
    attempts integer DEFAULT 0 NOT NULL,
    next_attempt_at timestamp with time zone DEFAULT now() NOT NULL,
    last_error text,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT agents_api_provider_deletions_state_check CHECK ((state = ANY (ARRAY['pending'::text, 'failed'::text])))
);

CREATE TABLE public.agents_api_sessions (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    session_id uuid NOT NULL,
    org_id bigint NOT NULL,
    owner uuid NOT NULL,
    lease_until timestamp with time zone NOT NULL,
    provider_session_id text,
    payload_encrypted bytea NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    provider_key text
);

CREATE VIEW public.app_channels AS
 SELECT id,
    app_id,
    public_id,
    channel_type,
    channel_config,
    channel_config_encrypted,
    durable_schedule_id,
    enabled,
    created_at,
    updated_at,
    auth,
    auth_encrypted
   FROM public.agent_channels;

CREATE TABLE public.apps (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    org_id bigint DEFAULT 1 NOT NULL,
    public_id text NOT NULL,
    name character varying(255) NOT NULL,
    description text,
    harness_id uuid NOT NULL,
    agent_id uuid,
    channel_type character varying(50),
    channel_config jsonb DEFAULT '{}'::jsonb NOT NULL,
    status character varying(50) DEFAULT 'draft'::character varying NOT NULL,
    published_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    archived_at timestamp with time zone,
    deleted_at timestamp with time zone,
    channel_config_encrypted bytea,
    virtual_user_id uuid,
    owner_principal_id uuid NOT NULL,
    resolved_owner_user_id uuid,
    CONSTRAINT apps_channel_type_check CHECK (((channel_type)::text = ANY ((ARRAY['slack'::character varying, 'ag_ui'::character varying, 'schedule'::character varying, 'webhook'::character varying, 'a2a'::character varying, 'fcp'::character varying, 'api_endpoint'::character varying, 'public_chat'::character varying, 'voice'::character varying, 'api'::character varying, 'poppy'::character varying])::text[]))),
    CONSTRAINT apps_public_id_format CHECK ((public_id ~ '^app_[0-9a-f]{32}$'::text)),
    CONSTRAINT apps_status_check CHECK (((status)::text = ANY ((ARRAY['draft'::character varying, 'published'::character varying, 'archived'::character varying, 'deleted'::character varying])::text[])))
);

CREATE TABLE public.archived_events (
    id uuid NOT NULL,
    session_id uuid NOT NULL,
    sequence integer NOT NULL,
    event_type character varying(100) NOT NULL,
    data jsonb DEFAULT '{}'::jsonb NOT NULL,
    ts timestamp with time zone NOT NULL,
    context jsonb DEFAULT '{}'::jsonb NOT NULL,
    metadata jsonb,
    tags text[],
    created_at timestamp with time zone NOT NULL,
    archived_at timestamp with time zone DEFAULT now() NOT NULL
);

COMMENT ON TABLE public.archived_events IS 'Cold storage for events past retention period';

CREATE TABLE public.audit_logs (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    org_id bigint NOT NULL,
    actor_id uuid,
    event_type text NOT NULL,
    ip_address text,
    metadata jsonb DEFAULT '{}'::jsonb NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    domain text DEFAULT 'management'::text NOT NULL,
    action text DEFAULT ''::text NOT NULL,
    target_type text,
    target_id text
);

CREATE TABLE public.budget_ledger (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    budget_id uuid NOT NULL,
    amount double precision NOT NULL,
    meter_source text NOT NULL,
    ref_type text,
    ref_id uuid,
    session_id uuid,
    description text,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE public.budgets (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    org_id bigint NOT NULL,
    subject_type text NOT NULL,
    subject_id text NOT NULL,
    currency text NOT NULL,
    "limit" double precision NOT NULL,
    soft_limit double precision,
    balance double precision NOT NULL,
    period jsonb,
    metadata jsonb DEFAULT '{}'::jsonb,
    status text DEFAULT 'active'::text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    period_started_at timestamp with time zone,
    CONSTRAINT budgets_status_check CHECK ((status = ANY (ARRAY['active'::text, 'paused'::text, 'exhausted'::text, 'disabled'::text]))),
    CONSTRAINT budgets_subject_type_check CHECK ((subject_type = ANY (ARRAY['session'::text, 'agent'::text, 'user'::text, 'org'::text, 'agent_trigger'::text, 'agent_channel'::text])))
);

CREATE TABLE public.channel_bindings (
    channel_id uuid NOT NULL,
    binding_key text NOT NULL,
    session_id uuid NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE public.channel_pending_deliveries (
    session_id uuid NOT NULL,
    input_message_id text NOT NULL,
    channel_id uuid NOT NULL,
    target jsonb NOT NULL,
    after_sequence bigint DEFAULT 0 NOT NULL,
    holder text NOT NULL,
    lease_until timestamp with time zone NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE public.channel_seen (
    channel_id uuid NOT NULL,
    dedup_key text NOT NULL,
    seen_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE public.cli_auth_sessions (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    state text NOT NULL,
    exchange_code text NOT NULL,
    user_id uuid,
    redirect_port integer NOT NULL,
    completed boolean DEFAULT false NOT NULL,
    expires_at timestamp with time zone NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE public.command_idempotency_keys (
    org_id bigint NOT NULL,
    principal_id uuid NOT NULL,
    idempotency_key text NOT NULL,
    command text NOT NULL,
    fingerprint text NOT NULL,
    response bytea,
    locked_until timestamp with time zone NOT NULL,
    expires_at timestamp with time zone NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE public.connection_setup_states (
    state text NOT NULL,
    provider text NOT NULL,
    payload_hash bytea NOT NULL,
    expires_at timestamp with time zone NOT NULL
);

CREATE TABLE public.decision_model_defaults (
    org_id bigint NOT NULL,
    model_id uuid NOT NULL
);

CREATE TABLE public.declarative_capabilities (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    org_id bigint DEFAULT 1 NOT NULL,
    public_id text NOT NULL,
    name character varying(120) NOT NULL,
    display_name character varying(120),
    description text NOT NULL,
    status character varying(50) DEFAULT 'active'::character varying NOT NULL,
    definition jsonb DEFAULT '{}'::jsonb NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    archived_at timestamp with time zone,
    deleted_at timestamp with time zone,
    CONSTRAINT declarative_capabilities_display_name_length CHECK (((display_name IS NULL) OR (octet_length((display_name)::text) <= 80))),
    CONSTRAINT declarative_capabilities_name_format CHECK (((name)::text ~ '^[a-z][a-z0-9_-]*[a-z0-9]$'::text)),
    CONSTRAINT declarative_capabilities_name_length CHECK ((octet_length((name)::text) <= 38)),
    CONSTRAINT declarative_capabilities_public_id_format CHECK ((public_id ~ '^cap_[0-9a-f]{32}$'::text)),
    CONSTRAINT declarative_capabilities_status_check CHECK (((status)::text = ANY ((ARRAY['active'::character varying, 'disabled'::character varying, 'archived'::character varying, 'deleted'::character varying])::text[])))
);

CREATE TABLE public.durable_circuit_breaker_state (
    key text NOT NULL,
    state text DEFAULT 'closed'::text NOT NULL,
    failure_count integer DEFAULT 0 NOT NULL,
    success_count integer DEFAULT 0 NOT NULL,
    last_failure_at timestamp with time zone,
    opened_at timestamp with time zone,
    half_open_at timestamp with time zone,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE public.durable_dead_letter_queue (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    original_task_id uuid NOT NULL,
    workflow_id uuid,
    activity_id text NOT NULL,
    activity_type text NOT NULL,
    input jsonb NOT NULL,
    attempts integer NOT NULL,
    last_error text NOT NULL,
    error_history jsonb NOT NULL,
    dead_at timestamp with time zone DEFAULT now() NOT NULL,
    requeued_at timestamp with time zone,
    requeue_count integer DEFAULT 0 NOT NULL,
    options jsonb
);

CREATE TABLE public.durable_schedule_executions (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    schedule_id uuid NOT NULL,
    scheduled_at timestamp with time zone NOT NULL,
    started_at timestamp with time zone DEFAULT now() NOT NULL,
    completed_at timestamp with time zone,
    status text DEFAULT 'running'::text NOT NULL,
    workflow_id uuid,
    task_id uuid,
    error text,
    duration_ms integer,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT durable_schedule_executions_status_check CHECK ((status = ANY (ARRAY['pending'::text, 'running'::text, 'completed'::text, 'failed'::text, 'skipped'::text])))
);

CREATE TABLE public.durable_scheduler_instances (
    instance_id text NOT NULL,
    started_at timestamp with time zone DEFAULT now() NOT NULL,
    last_heartbeat_at timestamp with time zone DEFAULT now() NOT NULL,
    schedules_processed bigint DEFAULT 0 NOT NULL,
    hostname text,
    version text
);

CREATE TABLE public.durable_schedules (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    name text NOT NULL,
    description text,
    cron_expression text NOT NULL,
    timezone text DEFAULT 'UTC'::text NOT NULL,
    target_type text NOT NULL,
    target_name text NOT NULL,
    target_input jsonb DEFAULT '{}'::jsonb NOT NULL,
    enabled boolean DEFAULT true NOT NULL,
    max_concurrent integer,
    catch_up_missed boolean DEFAULT false NOT NULL,
    max_catch_up integer DEFAULT 1,
    retry_policy jsonb,
    last_triggered_at timestamp with time zone,
    next_trigger_at timestamp with time zone,
    claimed_by text,
    claimed_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT durable_schedules_target_type_check CHECK ((target_type = ANY (ARRAY['workflow'::text, 'activity'::text])))
);

CREATE TABLE public.durable_signals (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    workflow_id uuid NOT NULL,
    signal_type text NOT NULL,
    payload jsonb NOT NULL,
    sent_at timestamp with time zone DEFAULT now() NOT NULL,
    processed_at timestamp with time zone,
    sequence_num integer NOT NULL
);

CREATE SEQUENCE public.durable_signals_sequence_num_seq
    AS integer
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;

ALTER SEQUENCE public.durable_signals_sequence_num_seq OWNED BY public.durable_signals.sequence_num;

CREATE TABLE public.durable_stat_counters (
    name text NOT NULL,
    value bigint DEFAULT 0 NOT NULL,
    shard smallint DEFAULT 0 NOT NULL
);

CREATE TABLE public.durable_task_queue (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    workflow_id uuid,
    activity_id text NOT NULL,
    activity_type text NOT NULL,
    input jsonb NOT NULL,
    options jsonb NOT NULL,
    status text DEFAULT 'pending'::text NOT NULL,
    priority integer DEFAULT 0 NOT NULL,
    scheduled_at timestamp with time zone DEFAULT now() NOT NULL,
    visible_at timestamp with time zone DEFAULT now() NOT NULL,
    claimed_by text,
    claimed_at timestamp with time zone,
    heartbeat_at timestamp with time zone,
    attempt integer DEFAULT 0 NOT NULL,
    max_attempts integer NOT NULL,
    last_error text,
    schedule_to_start_timeout_ms bigint NOT NULL,
    start_to_close_timeout_ms bigint NOT NULL,
    heartbeat_timeout_ms bigint,
    trace_id text,
    span_id text,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    progress_token bigint,
    no_progress_count integer DEFAULT 0 NOT NULL,
    queue text
);

CREATE TABLE public.durable_tool_results (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    turn_id text NOT NULL,
    tool_call_id text NOT NULL,
    tool_name text NOT NULL,
    args_fingerprint text NOT NULL,
    status text DEFAULT 'running'::text NOT NULL,
    claim_token uuid NOT NULL,
    result_json jsonb,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    settled_at timestamp with time zone,
    CONSTRAINT durable_tool_results_status_check CHECK ((status = ANY (ARRAY['running'::text, 'settled'::text, 'interrupted'::text])))
);

COMMENT ON TABLE public.durable_tool_results IS 'Per-tool-call idempotency records for the Act durable activity (EVE-530). Prevents double-execution of AtMostOnce tools on worker reclaim/replay.';

CREATE TABLE public.durable_workers (
    id text NOT NULL,
    worker_group text NOT NULL,
    activity_types text[] NOT NULL,
    max_concurrency integer NOT NULL,
    current_load integer DEFAULT 0 NOT NULL,
    status text DEFAULT 'active'::text NOT NULL,
    started_at timestamp with time zone DEFAULT now() NOT NULL,
    last_heartbeat_at timestamp with time zone DEFAULT now() NOT NULL,
    accepting_tasks boolean DEFAULT true NOT NULL,
    backpressure_reason text,
    hostname text,
    version text,
    metadata jsonb
);

CREATE TABLE public.durable_workflow_events (
    id bigint NOT NULL,
    workflow_id uuid NOT NULL,
    sequence_num integer NOT NULL,
    event_type text NOT NULL,
    event_data jsonb NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    trace_id text,
    span_id text
);

CREATE SEQUENCE public.durable_workflow_events_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;

ALTER SEQUENCE public.durable_workflow_events_id_seq OWNED BY public.durable_workflow_events.id;

CREATE TABLE public.durable_workflow_instances (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    workflow_type text NOT NULL,
    status text DEFAULT 'pending'::text NOT NULL,
    input jsonb NOT NULL,
    result jsonb,
    error jsonb,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    started_at timestamp with time zone,
    completed_at timestamp with time zone,
    partition_key integer DEFAULT 0 NOT NULL,
    trace_id text,
    span_id text,
    continued_as_new_id uuid
);

CREATE TABLE public.durable_workflow_snapshots (
    id bigint NOT NULL,
    workflow_id uuid NOT NULL,
    sequence_num integer NOT NULL,
    snapshot_data bytea NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE SEQUENCE public.durable_workflow_snapshots_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;

ALTER SEQUENCE public.durable_workflow_snapshots_id_seq OWNED BY public.durable_workflow_snapshots.id;

CREATE TABLE public.email_verification_tokens (
    id bigint NOT NULL,
    user_id uuid NOT NULL,
    token_hash text NOT NULL,
    expires_at timestamp with time zone NOT NULL,
    used_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE SEQUENCE public.email_verification_tokens_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;

ALTER SEQUENCE public.email_verification_tokens_id_seq OWNED BY public.email_verification_tokens.id;

CREATE TABLE public.entity_changes (
    id uuid NOT NULL,
    org_id bigint NOT NULL,
    entity_kind text NOT NULL,
    entity_ref text NOT NULL,
    command text NOT NULL,
    action text NOT NULL,
    reason text,
    changed_fields text[] DEFAULT '{}'::text[] NOT NULL,
    actor_kind text NOT NULL,
    actor_user_id uuid,
    via_session_id uuid,
    via_agent_id text,
    surface text NOT NULL,
    request_id text,
    idempotency_key text,
    revision bigint,
    snapshot jsonb,
    snapshot_hash text,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    restored_from_revision bigint,
    CONSTRAINT entity_changes_reason_length CHECK (((reason IS NULL) OR ((char_length(reason) >= 1) AND (char_length(reason) <= 1000))))
);

CREATE TABLE public.entity_manager_context (
    org_id bigint NOT NULL,
    entity_kind text NOT NULL,
    entity_ref text NOT NULL,
    content text NOT NULL,
    revision bigint NOT NULL,
    updated_by_user_id uuid,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT entity_manager_context_revision CHECK ((revision >= 1)),
    CONSTRAINT entity_manager_context_size CHECK ((octet_length(content) <= 16384))
);

CREATE TABLE public.eval_case_results (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    eval_run_id uuid NOT NULL,
    eval_case_id uuid NOT NULL,
    public_id text NOT NULL,
    session_id uuid,
    status character varying(50) DEFAULT 'pending'::character varying NOT NULL,
    scores jsonb,
    turns integer,
    latency_ms bigint,
    input_tokens bigint,
    output_tokens bigint,
    error_message text,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    target jsonb,
    target_snapshot jsonb,
    metadata jsonb,
    artifacts jsonb,
    CONSTRAINT eval_case_results_public_id_format CHECK ((public_id ~ '^evalresult_[0-9a-f]{32}$'::text)),
    CONSTRAINT eval_case_results_status_check CHECK (((status)::text = ANY ((ARRAY['pending'::character varying, 'running'::character varying, 'passed'::character varying, 'failed'::character varying, 'errored'::character varying, 'timeout'::character varying, 'skipped'::character varying])::text[])))
);

CREATE TABLE public.eval_cases (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    eval_id uuid NOT NULL,
    public_id text NOT NULL,
    name text NOT NULL,
    description text,
    tags text[] DEFAULT '{}'::text[] NOT NULL,
    conversation jsonb DEFAULT '[]'::jsonb NOT NULL,
    scorers jsonb DEFAULT '[]'::jsonb NOT NULL,
    max_turns integer,
    timeout_seconds integer,
    "position" integer DEFAULT 0 NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    target jsonb,
    post jsonb,
    artifacts jsonb,
    CONSTRAINT eval_cases_public_id_format CHECK ((public_id ~ '^evalcase_[0-9a-f]{32}$'::text))
);

CREATE TABLE public.eval_run_datasets (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    org_id bigint NOT NULL,
    public_id text NOT NULL,
    eval_run_id uuid,
    request jsonb NOT NULL,
    status character varying(50) DEFAULT 'pending'::character varying NOT NULL,
    body text,
    record_count bigint,
    error_message text,
    started_at timestamp with time zone,
    completed_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT eval_run_datasets_public_id_format CHECK ((public_id ~ '^evaldataset_[0-9a-f]{32}$'::text)),
    CONSTRAINT eval_run_datasets_status_check CHECK (((status)::text = ANY ((ARRAY['pending'::character varying, 'running'::character varying, 'completed'::character varying, 'failed'::character varying])::text[])))
);

CREATE TABLE public.eval_run_share_tokens (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    org_id bigint NOT NULL,
    public_id text NOT NULL,
    eval_run_id uuid NOT NULL,
    token_hash text NOT NULL,
    token_prefix text NOT NULL,
    created_by uuid,
    expires_at timestamp with time zone,
    revoked_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT eval_run_share_tokens_public_id_format CHECK ((public_id ~ '^evalshare_[0-9a-f]{32}$'::text))
);

CREATE TABLE public.eval_runs (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    eval_id uuid NOT NULL,
    org_id bigint NOT NULL,
    public_id text NOT NULL,
    model_override text,
    filter_tags text[],
    status character varying(50) DEFAULT 'pending'::character varying NOT NULL,
    triggered_by text DEFAULT 'user'::text NOT NULL,
    started_at timestamp with time zone,
    completed_at timestamp with time zone,
    summary jsonb,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    target jsonb,
    source text DEFAULT 'internal'::text NOT NULL,
    source_run_id text,
    attribution jsonb,
    CONSTRAINT eval_runs_public_id_format CHECK ((public_id ~ '^evalrun_[0-9a-f]{32}$'::text)),
    CONSTRAINT eval_runs_status_check CHECK (((status)::text = ANY ((ARRAY['pending'::character varying, 'running'::character varying, 'completed'::character varying, 'failed'::character varying, 'cancelled'::character varying])::text[])))
);

CREATE TABLE public.evals (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    org_id bigint NOT NULL,
    public_id text NOT NULL,
    name text NOT NULL,
    description text,
    model_override text,
    tags text[] DEFAULT '{}'::text[] NOT NULL,
    status character varying(50) DEFAULT 'active'::character varying NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    archived_at timestamp with time zone,
    deleted_at timestamp with time zone,
    target jsonb,
    CONSTRAINT evals_public_id_format CHECK ((public_id ~ '^eval_[0-9a-f]{32}$'::text)),
    CONSTRAINT evals_status_check CHECK (((status)::text = ANY ((ARRAY['active'::character varying, 'archived'::character varying, 'deleted'::character varying])::text[])))
);

CREATE TABLE public.event_sequences (
    session_id uuid NOT NULL,
    next_sequence integer DEFAULT 1 NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    removed_count integer DEFAULT 0 NOT NULL
);

COMMENT ON TABLE public.event_sequences IS 'Per-session sequence counter for atomic event sequence allocation';

COMMENT ON COLUMN public.event_sequences.next_sequence IS 'Next sequence number to allocate (1-indexed)';

COMMENT ON COLUMN public.event_sequences.removed_count IS 'Sequence numbers handed out with no live event (deleted by retention, or a conflicting insert); event count = next_sequence - 1 - removed_count';

CREATE TABLE public.events (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    session_id uuid NOT NULL,
    sequence integer NOT NULL,
    event_type character varying(100) NOT NULL,
    data jsonb DEFAULT '{}'::jsonb NOT NULL,
    ts timestamp with time zone DEFAULT now() NOT NULL,
    context jsonb DEFAULT '{}'::jsonb NOT NULL,
    metadata jsonb,
    tags text[],
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    search_vector tsvector GENERATED ALWAYS AS (to_tsvector('english'::regconfig, "left"(COALESCE((data #>> '{message,content}'::text[]), (data #>> '{result}'::text[]), (data ->> 'content'::text), (data ->> 'delta'::text), (data ->> 'accumulated'::text), ''::text), 250000))) STORED,
    turn_resolution_id uuid,
    turn_resolution_event_index integer
);

COMMENT ON COLUMN public.events.ts IS 'Event timestamp (when the event occurred)';

COMMENT ON COLUMN public.events.context IS 'Event correlation context (turn_id, input_message_id, exec_id)';

COMMENT ON COLUMN public.events.metadata IS 'Arbitrary metadata for the event';

COMMENT ON COLUMN public.events.tags IS 'Tags for filtering and categorization';

COMMENT ON COLUMN public.events.search_vector IS 'Generated tsvector for full-text search on canonical event text fields (EVE-87)';

CREATE TABLE public.execution_environment_revisions (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    environment_id uuid NOT NULL,
    revision integer NOT NULL,
    profile jsonb NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT execution_environment_revisions_revision_check CHECK ((revision > 0))
);

COMMENT ON TABLE public.execution_environment_revisions IS 'Immutable execution profile revisions pinned by Agent versions and Sessions.';

CREATE TABLE public.execution_environments (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    org_id bigint NOT NULL,
    name text NOT NULL,
    display_name text NOT NULL,
    description text,
    is_managed boolean DEFAULT false NOT NULL,
    status text DEFAULT 'active'::text NOT NULL,
    current_revision_id uuid,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT execution_environments_status_check CHECK ((status = ANY (ARRAY['active'::text, 'archived'::text])))
);

COMMENT ON TABLE public.execution_environments IS 'Reusable org-scoped execution configuration. Mutable metadata points at an immutable current revision.';

CREATE TABLE public.fact_budget_posting (
    org_id bigint NOT NULL,
    source_key text NOT NULL,
    session_id uuid,
    user_id uuid,
    principal_id uuid,
    agent_id uuid,
    agent_name_snapshot text,
    harness_id uuid,
    harness_name_snapshot text,
    app_id uuid,
    blueprint_id text,
    created_at timestamp with time zone NOT NULL,
    time_bucket_day date NOT NULL,
    budget_id uuid,
    budget_subject_type text,
    budget_subject_id text,
    currency text,
    meter_source text,
    ref_type text,
    amount double precision DEFAULT 0 NOT NULL,
    debit_count bigint DEFAULT 0 NOT NULL,
    credit_count bigint DEFAULT 0 NOT NULL,
    projected_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE public.fact_capability_usage (
    org_id bigint NOT NULL,
    source_key text NOT NULL,
    session_id uuid,
    turn_id text,
    user_id uuid,
    principal_id uuid,
    agent_id uuid,
    agent_name_snapshot text,
    harness_id uuid,
    harness_name_snapshot text,
    app_id uuid,
    blueprint_id text,
    created_at timestamp with time zone NOT NULL,
    time_bucket_day date NOT NULL,
    capability_id text NOT NULL,
    capability_name_snapshot text,
    capability_usage_kind text NOT NULL,
    tool_name text,
    usage_count bigint DEFAULT 1 NOT NULL,
    duration_ms bigint DEFAULT 0 NOT NULL,
    projected_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT fact_capability_usage_capability_usage_kind_check CHECK ((capability_usage_kind = ANY (ARRAY['configured'::text, 'resolved'::text, 'exposed'::text, 'invoked'::text, 'effect_ran'::text])))
);

CREATE TABLE public.fact_llm_generation (
    org_id bigint NOT NULL,
    source_key text NOT NULL,
    session_id uuid,
    turn_id text,
    user_id uuid,
    principal_id uuid,
    agent_id uuid,
    agent_name_snapshot text,
    harness_id uuid,
    harness_name_snapshot text,
    app_id uuid,
    blueprint_id text,
    created_at timestamp with time zone NOT NULL,
    time_bucket_day date NOT NULL,
    model text,
    provider text,
    finish_reason text,
    input_tokens bigint DEFAULT 0 NOT NULL,
    output_tokens bigint DEFAULT 0 NOT NULL,
    cache_read_tokens bigint DEFAULT 0 NOT NULL,
    cache_creation_tokens bigint DEFAULT 0 NOT NULL,
    total_tokens bigint DEFAULT 0 NOT NULL,
    duration_ms bigint DEFAULT 0 NOT NULL,
    time_to_first_token_ms bigint DEFAULT 0 NOT NULL,
    success_count bigint DEFAULT 1 NOT NULL,
    error_count bigint DEFAULT 0 NOT NULL,
    projected_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE public.fact_session (
    org_id bigint NOT NULL,
    source_key text NOT NULL,
    session_id uuid,
    user_id uuid,
    principal_id uuid,
    agent_id uuid,
    agent_name_snapshot text,
    harness_id uuid,
    harness_name_snapshot text,
    app_id uuid,
    blueprint_id text,
    created_at timestamp with time zone NOT NULL,
    time_bucket_day date NOT NULL,
    session_status text,
    started_at timestamp with time zone,
    finished_at timestamp with time zone,
    last_active_at timestamp with time zone,
    session_count bigint DEFAULT 1 NOT NULL,
    duration_ms bigint DEFAULT 0 NOT NULL,
    turn_count bigint DEFAULT 0 NOT NULL,
    input_tokens bigint DEFAULT 0 NOT NULL,
    output_tokens bigint DEFAULT 0 NOT NULL,
    cache_read_tokens bigint DEFAULT 0 NOT NULL,
    cache_creation_tokens bigint DEFAULT 0 NOT NULL,
    total_tokens bigint DEFAULT 0 NOT NULL,
    tool_call_count bigint DEFAULT 0 NOT NULL,
    projected_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE public.fact_tool_call (
    org_id bigint NOT NULL,
    source_key text NOT NULL,
    session_id uuid,
    turn_id text,
    user_id uuid,
    principal_id uuid,
    agent_id uuid,
    agent_name_snapshot text,
    harness_id uuid,
    harness_name_snapshot text,
    app_id uuid,
    blueprint_id text,
    created_at timestamp with time zone NOT NULL,
    time_bucket_day date NOT NULL,
    tool_call_id text,
    tool_name text,
    tool_display_name_snapshot text,
    tool_status text,
    capability_id text,
    capability_name_snapshot text,
    call_count bigint DEFAULT 1 NOT NULL,
    success_count bigint DEFAULT 0 NOT NULL,
    error_count bigint DEFAULT 0 NOT NULL,
    timeout_count bigint DEFAULT 0 NOT NULL,
    cancelled_count bigint DEFAULT 0 NOT NULL,
    duration_ms bigint DEFAULT 0 NOT NULL,
    projected_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE public.fact_turn (
    org_id bigint NOT NULL,
    source_key text NOT NULL,
    session_id uuid,
    turn_id text,
    user_id uuid,
    principal_id uuid,
    agent_id uuid,
    agent_name_snapshot text,
    harness_id uuid,
    harness_name_snapshot text,
    app_id uuid,
    blueprint_id text,
    created_at timestamp with time zone NOT NULL,
    time_bucket_day date NOT NULL,
    turn_status text,
    turn_count bigint DEFAULT 1 NOT NULL,
    duration_ms bigint DEFAULT 0 NOT NULL,
    iterations bigint DEFAULT 0 NOT NULL,
    input_tokens bigint DEFAULT 0 NOT NULL,
    output_tokens bigint DEFAULT 0 NOT NULL,
    cache_read_tokens bigint DEFAULT 0 NOT NULL,
    cache_creation_tokens bigint DEFAULT 0 NOT NULL,
    total_tokens bigint DEFAULT 0 NOT NULL,
    tool_call_count bigint DEFAULT 0 NOT NULL,
    success_count bigint DEFAULT 0 NOT NULL,
    error_count bigint DEFAULT 0 NOT NULL,
    projected_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE public.files (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    org_id bigint NOT NULL,
    filename text,
    content_type text NOT NULL,
    size_bytes bigint NOT NULL,
    data bytea NOT NULL,
    metadata jsonb DEFAULT '{}'::jsonb NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE public.github_apps (
    id uuid NOT NULL,
    org_id bigint NOT NULL,
    virtual_user_id uuid NOT NULL,
    app_id bigint NOT NULL,
    slug text NOT NULL,
    name text NOT NULL,
    html_url text NOT NULL,
    owner_login text,
    client_id text,
    client_secret_encrypted bytea,
    private_key_encrypted bytea NOT NULL,
    webhook_secret_encrypted bytea,
    created_by_user_id uuid,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE public.harness_capabilities (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    harness_id uuid NOT NULL,
    capability_id character varying(71) NOT NULL,
    "position" integer DEFAULT 0 NOT NULL,
    config jsonb DEFAULT '{}'::jsonb NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE public.harnesses (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    org_id bigint DEFAULT 1 NOT NULL,
    name character varying(255) NOT NULL,
    description text,
    system_prompt text,
    default_model_id uuid,
    tags text[] DEFAULT '{}'::text[] NOT NULL,
    status character varying(50) DEFAULT 'active'::character varying NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    archived_at timestamp with time zone,
    deleted_at timestamp with time zone,
    is_built_in boolean DEFAULT false NOT NULL,
    initial_files jsonb DEFAULT '[]'::jsonb NOT NULL,
    parent_harness_id uuid,
    network_access jsonb,
    display_name text,
    mcp_servers jsonb DEFAULT '{}'::jsonb NOT NULL,
    embedder_metadata jsonb DEFAULT '{}'::jsonb NOT NULL,
    icon character varying(64),
    intro_markdown text,
    short_description text,
    starters jsonb DEFAULT '[]'::jsonb,
    CONSTRAINT harnesses_status_check CHECK (((status)::text = ANY ((ARRAY['active'::character varying, 'archived'::character varying, 'deleted'::character varying])::text[])))
);

COMMENT ON COLUMN public.harnesses.initial_files IS 'Starter files copied into each new session for this harness';

CREATE TABLE public.health_issue_reminders (
    issue_id uuid NOT NULL,
    user_id uuid NOT NULL,
    episode_id uuid NOT NULL,
    snoozed_until timestamp with time zone NOT NULL
);

CREATE TABLE public.health_issues (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    org_id bigint NOT NULL,
    channel_id uuid,
    code text NOT NULL,
    episode_id uuid DEFAULT public.uuidv7() NOT NULL,
    status text NOT NULL,
    missing_scopes text[] DEFAULT '{}'::text[] NOT NULL,
    error_code text,
    channel_revision timestamp with time zone,
    first_detected_at timestamp with time zone DEFAULT now() NOT NULL,
    last_checked_at timestamp with time zone NOT NULL,
    CONSTRAINT health_issues_status_check CHECK ((status = ANY (ARRAY['open'::text, 'needs_check'::text, 'resolved'::text, 'inapplicable'::text])))
);

CREATE TABLE public.image_blobs (
    image_id uuid NOT NULL,
    data_key text NOT NULL,
    thumbnail_key text,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE public.images (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    filename text NOT NULL,
    content_type text NOT NULL,
    size_bytes bigint NOT NULL,
    data bytea NOT NULL,
    thumbnail_data bytea,
    thumbnail_content_type text,
    metadata jsonb DEFAULT '{}'::jsonb NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    org_id bigint DEFAULT 1 NOT NULL
);

COMMENT ON TABLE public.images IS 'Global image storage for message attachments';

COMMENT ON COLUMN public.images.content_type IS 'MIME type: image/png, image/jpeg, image/gif, image/webp';

COMMENT ON COLUMN public.images.thumbnail_data IS 'Thumbnail image for efficient display (max 200x200)';

COMMENT ON COLUMN public.images.metadata IS 'JSON metadata including optional session_id for tracking';

CREATE TABLE public.knowledge_bases (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    org_id bigint DEFAULT 1 NOT NULL,
    public_id text NOT NULL,
    name character varying(255) NOT NULL,
    description text,
    owner_principal_id text,
    resolved_owner_user_id uuid,
    status character varying(50) DEFAULT 'active'::character varying NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    archived_at timestamp with time zone,
    deleted_at timestamp with time zone,
    embedding_model_id uuid,
    CONSTRAINT knowledge_bases_public_id_format CHECK ((public_id ~ '^kb_[0-9a-f]{32}$'::text)),
    CONSTRAINT knowledge_bases_status_check CHECK (((status)::text = ANY ((ARRAY['active'::character varying, 'archived'::character varying, 'deleted'::character varying])::text[])))
);

COMMENT ON TABLE public.knowledge_bases IS 'Knowledge Bases — org-scoped curated entry collections. See specs/knowledge-bases.md.';

COMMENT ON COLUMN public.knowledge_bases.embedding_model_id IS 'Optional embedding model for hybrid retrieval. NULL = keyword search only. See specs/knowledge-bases.md.';

CREATE TABLE public.knowledge_entries (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    kb_id uuid NOT NULL,
    public_id text NOT NULL,
    title character varying(512) NOT NULL,
    body text NOT NULL,
    kind character varying(32) DEFAULT 'note'::character varying NOT NULL,
    tags text[] DEFAULT '{}'::text[] NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    resource text,
    CONSTRAINT knowledge_entries_body_size CHECK ((octet_length(body) <= 65536)),
    CONSTRAINT knowledge_entries_kind_check CHECK (((kind)::text = ANY ((ARRAY['note'::character varying, 'table'::character varying, 'business'::character varying, 'query'::character varying, 'runbook'::character varying])::text[]))),
    CONSTRAINT knowledge_entries_public_id_format CHECK ((public_id ~ '^kbe_[0-9a-f]{32}$'::text)),
    CONSTRAINT knowledge_entries_tags_count CHECK ((cardinality(tags) <= 16))
);

COMMENT ON TABLE public.knowledge_entries IS 'Curated entries inside a Knowledge Base. Title + markdown body, kind-tagged.';

COMMENT ON COLUMN public.knowledge_entries.resource IS 'Optional OKF resource URI identifying the underlying asset. See specs/okf-adoption.md.';

CREATE TABLE public.knowledge_index_chunks (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    document_id uuid NOT NULL,
    index_id uuid NOT NULL,
    public_id text NOT NULL,
    ordinal integer NOT NULL,
    text text NOT NULL,
    location jsonb,
    token_count integer,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT knowledge_index_chunks_public_id_format CHECK ((public_id ~ '^kchk_[0-9a-f]{32}$'::text))
);

COMMENT ON TABLE public.knowledge_index_chunks IS 'Citable chunks of an index document. Embedding vectors live in the external vector store, keyed by public_id.';

CREATE TABLE public.knowledge_index_documents (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    index_id uuid NOT NULL,
    public_id text NOT NULL,
    source_uri text NOT NULL,
    title character varying(1024),
    mime_type character varying(255),
    content_hash text,
    size_bytes bigint,
    chunk_count integer DEFAULT 0 NOT NULL,
    last_seen_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT knowledge_index_documents_public_id_format CHECK ((public_id ~ '^kidoc_[0-9a-f]{32}$'::text))
);

COMMENT ON TABLE public.knowledge_index_documents IS 'Ingested source documents inside a Knowledge Index.';

CREATE TABLE public.knowledge_indexes (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    org_id bigint DEFAULT 1 NOT NULL,
    public_id text NOT NULL,
    name character varying(255) NOT NULL,
    description text,
    source_type character varying(50) DEFAULT 'github'::character varying NOT NULL,
    source_config jsonb DEFAULT '{}'::jsonb NOT NULL,
    embedding_model_id uuid NOT NULL,
    vector_dim integer,
    vector_namespace text,
    owner_principal_id text,
    resolved_owner_user_id uuid,
    status character varying(50) DEFAULT 'active'::character varying NOT NULL,
    sync_status character varying(50) DEFAULT 'idle'::character varying NOT NULL,
    last_synced_at timestamp with time zone,
    last_sync_error text,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    archived_at timestamp with time zone,
    deleted_at timestamp with time zone,
    CONSTRAINT knowledge_indexes_public_id_format CHECK ((public_id ~ '^kidx_[0-9a-f]{32}$'::text)),
    CONSTRAINT knowledge_indexes_source_type_check CHECK (((source_type)::text = ANY ((ARRAY['github'::character varying, 'git'::character varying])::text[]))),
    CONSTRAINT knowledge_indexes_status_check CHECK (((status)::text = ANY ((ARRAY['active'::character varying, 'archived'::character varying, 'deleted'::character varying])::text[]))),
    CONSTRAINT knowledge_indexes_sync_status_check CHECK (((sync_status)::text = ANY ((ARRAY['idle'::character varying, 'pending'::character varying, 'syncing'::character varying, 'synced'::character varying, 'failed'::character varying])::text[])))
);

COMMENT ON TABLE public.knowledge_indexes IS 'Knowledge Indexes — org-scoped source-backed embedded collections. See specs/knowledge-indexes.md.';

CREATE TABLE public.leased_resources (
    id uuid NOT NULL,
    public_id text NOT NULL,
    org_id bigint NOT NULL,
    session_id uuid,
    provider text NOT NULL,
    resource_type text NOT NULL,
    external_id text NOT NULL,
    display_name text,
    status text NOT NULL,
    owner_user_id uuid,
    lease_duration_seconds integer NOT NULL,
    last_touched_at timestamp with time zone NOT NULL,
    lease_expires_at timestamp with time zone NOT NULL,
    cleanup_started_at timestamp with time zone,
    cleanup_completed_at timestamp with time zone,
    cleanup_attempts integer DEFAULT 0 NOT NULL,
    last_cleanup_error text,
    metadata jsonb DEFAULT '{}'::jsonb NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    pending_connection_id uuid,
    connection_id uuid,
    CONSTRAINT leased_resources_cleanup_attempts_check CHECK ((cleanup_attempts >= 0)),
    CONSTRAINT leased_resources_lease_duration_seconds_check CHECK ((lease_duration_seconds > 0)),
    CONSTRAINT leased_resources_status_check CHECK ((status = ANY (ARRAY['active'::text, 'cleaning'::text, 'released'::text, 'cleanup_failed'::text])))
);

COMMENT ON TABLE public.leased_resources IS 'Cross-provider leased remote resources that require eventual cleanup.';

COMMENT ON COLUMN public.leased_resources.owner_user_id IS 'User connection owner used to resolve the same provider credentials during cleanup.';

COMMENT ON COLUMN public.leased_resources.lease_expires_at IS 'Absolute cleanup deadline used by the durable leased-resource cleanup schedule.';

COMMENT ON COLUMN public.leased_resources.metadata IS 'Non-secret provider metadata for UI, debugging, and cleanup handlers.';

COMMENT ON COLUMN public.leased_resources.connection_id IS 'Exact provider connection used to create the resource; cleanup must use the same account.';

CREATE TABLE public.llm_generations (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    org_id bigint DEFAULT 1 NOT NULL,
    session_id uuid,
    turn_id uuid,
    event_id uuid,
    model text,
    provider text,
    input_tokens bigint DEFAULT 0 NOT NULL,
    output_tokens bigint DEFAULT 0 NOT NULL,
    cache_read_tokens bigint DEFAULT 0 NOT NULL,
    cache_creation_tokens bigint DEFAULT 0 NOT NULL,
    duration_ms integer,
    finish_reason text,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    actual_cost_usd double precision,
    estimated_cost_usd double precision,
    provider_response_id text,
    reconciled_at timestamp with time zone,
    reconciliation_attempts integer DEFAULT 0 NOT NULL,
    reconcile_after timestamp with time zone,
    provider_session_id text,
    provider_config_id text,
    usage_pending boolean DEFAULT false NOT NULL,
    managed boolean DEFAULT false NOT NULL
);

COMMENT ON TABLE public.llm_generations IS 'Source of truth for all LLM generation calls and their token usage';

COMMENT ON COLUMN public.llm_generations.session_id IS 'Owning conversation, or NULL for org-attributed background inference (e.g. knowledge-index sync embeddings) that no session triggered.';

COMMENT ON COLUMN public.llm_generations.turn_id IS 'Groups generations within a turn';

COMMENT ON COLUMN public.llm_generations.event_id IS 'Reference to original event (for traceability)';

COMMENT ON COLUMN public.llm_generations.actual_cost_usd IS 'Provider-reported actual cost in USD (e.g. OpenRouter usage.cost); NULL when the provider does not report one.';

COMMENT ON COLUMN public.llm_generations.estimated_cost_usd IS 'Price-table estimate of cost in USD from the model profile; NULL when there is no profile cost data.';

COMMENT ON COLUMN public.llm_generations.provider_config_id IS 'Public id of the provider account that served the call; NULL when the host resolved no stored provider.';

COMMENT ON COLUMN public.llm_generations.usage_pending IS 'Usage the provider had not reported when the generation was billed; cleared once the late usage is applied (EVE-1145).';

COMMENT ON COLUMN public.llm_generations.managed IS 'Whether the serving provider was host-managed at the time of the call (managed spend vs BYOK).';

CREATE TABLE public.mcp_event_subscriptions (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    subscription_key text NOT NULL,
    org_id bigint NOT NULL,
    user_id uuid NOT NULL,
    event_name text NOT NULL,
    arguments jsonb DEFAULT '{}'::jsonb NOT NULL,
    callback_url text NOT NULL,
    secret_encrypted bytea NOT NULL,
    expires_at timestamp with time zone NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE public.mcp_servers (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    org_id bigint DEFAULT 1 NOT NULL,
    name character varying(255) NOT NULL,
    description text,
    url text NOT NULL,
    transport_type character varying(50) DEFAULT 'http'::character varying NOT NULL,
    status character varying(50) DEFAULT 'active'::character varying NOT NULL,
    api_key_encrypted bytea,
    api_key_set boolean DEFAULT false NOT NULL,
    headers jsonb DEFAULT '{}'::jsonb NOT NULL,
    settings jsonb DEFAULT '{}'::jsonb NOT NULL,
    cached_tools jsonb DEFAULT '[]'::jsonb NOT NULL,
    tools_cached_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    archived_at timestamp with time zone,
    deleted_at timestamp with time zone,
    owner_virtual_user_id uuid,
    catalog_mcp_server_id uuid,
    deferred boolean DEFAULT true NOT NULL,
    CONSTRAINT mcp_servers_status_check CHECK (((status)::text = ANY ((ARRAY['active'::character varying, 'disabled'::character varying, 'archived'::character varying, 'deleted'::character varying])::text[]))),
    CONSTRAINT mcp_servers_transport_type_check CHECK (((transport_type)::text = 'http'::text))
);

COMMENT ON COLUMN public.mcp_servers.cached_tools IS 'Cached tool definitions from MCP server tools/list endpoint';

COMMENT ON COLUMN public.mcp_servers.tools_cached_at IS 'Timestamp when tools were last fetched from the MCP server';

COMMENT ON COLUMN public.mcp_servers.owner_virtual_user_id IS 'Virtual user that owns this server (User MCP server); NULL for organization catalog presets';

COMMENT ON COLUMN public.mcp_servers.catalog_mcp_server_id IS 'Catalog preset a user server was added from; NULL for custom user servers and for catalog rows';

COMMENT ON COLUMN public.mcp_servers.deferred IS 'User MCP servers only: whether the server loads on demand (tool search reveal); ignored for catalog presets';

CREATE TABLE public.mcp_service_tool_caches (
    org_id bigint NOT NULL,
    mcp_server_id uuid NOT NULL,
    agent_id uuid NOT NULL,
    cache_scope text NOT NULL,
    credential_hash text DEFAULT ''::text NOT NULL,
    cached_tools jsonb NOT NULL,
    ttl_ms bigint NOT NULL,
    tools_cached_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT mcp_service_tool_caches_cache_scope_check CHECK ((cache_scope = ANY (ARRAY['public'::text, 'private'::text]))),
    CONSTRAINT mcp_service_tool_caches_check CHECK ((((cache_scope = 'public'::text) AND (credential_hash = ''::text)) OR ((cache_scope = 'private'::text) AND (credential_hash <> ''::text)))),
    CONSTRAINT mcp_service_tool_caches_ttl_ms_check CHECK ((ttl_ms > 0))
);

CREATE TABLE public.mcp_tool_labels (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    org_id bigint DEFAULT 1 NOT NULL,
    mcp_server_id uuid NOT NULL,
    tool_name text NOT NULL,
    label text,
    suggested_label text,
    set_by uuid,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT mcp_tool_labels_label_check CHECK ((label = ANY (ARRAY['read_only'::text, 'changes'::text]))),
    CONSTRAINT mcp_tool_labels_suggested_label_check CHECK ((suggested_label = ANY (ARRAY['read_only'::text, 'changes'::text])))
);

CREATE TABLE public.memories (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    org_id bigint DEFAULT 1 NOT NULL,
    public_id text NOT NULL,
    name character varying(255) NOT NULL,
    description text,
    owner_principal_id text,
    resolved_owner_user_id uuid,
    status character varying(50) DEFAULT 'active'::character varying NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    archived_at timestamp with time zone,
    deleted_at timestamp with time zone,
    source_type character varying(32) DEFAULT 'manual'::character varying NOT NULL,
    source_config jsonb DEFAULT '{}'::jsonb NOT NULL,
    is_readonly boolean DEFAULT false NOT NULL,
    sync_status character varying(32) DEFAULT 'idle'::character varying NOT NULL,
    last_synced_at timestamp with time zone,
    last_sync_error text,
    scope character varying(16) DEFAULT 'org'::character varying NOT NULL,
    owner_agent_id uuid,
    owner_user_id uuid,
    CONSTRAINT memories_manual_source_shape CHECK ((((source_type)::text <> 'manual'::text) OR ((source_config = '{}'::jsonb) AND (is_readonly = false) AND ((sync_status)::text = 'idle'::text) AND (last_synced_at IS NULL) AND (last_sync_error IS NULL)))),
    CONSTRAINT memories_public_id_format CHECK ((public_id ~ '^mem_[0-9a-f]{32}$'::text)),
    CONSTRAINT memories_scope_check CHECK (((scope)::text = ANY ((ARRAY['org'::character varying, 'agent'::character varying, 'user'::character varying])::text[]))),
    CONSTRAINT memories_scope_owner_shape CHECK (((((scope)::text = 'org'::text) AND (owner_agent_id IS NULL) AND (owner_user_id IS NULL)) OR (((scope)::text = 'agent'::text) AND (owner_agent_id IS NOT NULL) AND (owner_user_id IS NULL)) OR (((scope)::text = 'user'::text) AND (owner_agent_id IS NULL) AND (owner_user_id IS NOT NULL)))),
    CONSTRAINT memories_scoped_manual_source CHECK ((((scope)::text = 'org'::text) OR (((source_type)::text = 'manual'::text) AND (source_config = '{}'::jsonb) AND (is_readonly = false)))),
    CONSTRAINT memories_source_readonly CHECK ((((source_type)::text = 'manual'::text) OR (is_readonly = true))),
    CONSTRAINT volumes_source_type_check CHECK (((source_type)::text = ANY ((ARRAY['manual'::character varying, 'github'::character varying, 'git'::character varying])::text[]))),
    CONSTRAINT volumes_status_check CHECK (((status)::text = ANY ((ARRAY['active'::character varying, 'archived'::character varying, 'deleted'::character varying])::text[]))),
    CONSTRAINT volumes_sync_status_check CHECK (((sync_status)::text = ANY ((ARRAY['idle'::character varying, 'pending'::character varying, 'syncing'::character varying, 'synced'::character varying, 'failed'::character varying])::text[])))
);

COMMENT ON TABLE public.memories IS 'Memories — org-scoped named stores. See specs/memory.md.';

COMMENT ON COLUMN public.memories.scope IS 'Memory scope: org, agent, or user. See specs/memory.md.';

COMMENT ON COLUMN public.memories.owner_agent_id IS 'Owning agent for scope=agent memories.';

COMMENT ON COLUMN public.memories.owner_user_id IS 'Owning user for scope=user memories.';

CREATE TABLE public.memory_files (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    memory_id uuid NOT NULL,
    path text NOT NULL,
    content bytea,
    is_directory boolean DEFAULT false NOT NULL,
    size_bytes bigint DEFAULT 0 NOT NULL,
    content_hash text,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT memory_files_directory_no_content CHECK (((NOT is_directory) OR (content IS NULL))),
    CONSTRAINT memory_files_path_check CHECK ((path ~ '^/([^/\0]+(/[^/\0]+)*)?$'::text))
);

COMMENT ON TABLE public.memory_files IS 'Files and directories inside a Memory. Path validation mirrors session_files.';

CREATE TABLE public.message_feedback (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    org_id bigint DEFAULT 1 NOT NULL,
    session_id uuid NOT NULL,
    message_id text NOT NULL,
    user_id uuid NOT NULL,
    rating text NOT NULL,
    comment text,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT message_feedback_rating_check CHECK ((rating = ANY (ARRAY['good'::text, 'bad'::text])))
);

CREATE TABLE public.model_router_candidates (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    route_id uuid NOT NULL,
    model_id uuid NOT NULL,
    request_overrides jsonb DEFAULT '{}'::jsonb NOT NULL,
    weight integer DEFAULT 1 NOT NULL,
    rules jsonb,
    "position" integer DEFAULT 0 NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT model_router_candidates_weight_check CHECK ((weight >= 0))
);

COMMENT ON TABLE public.model_router_candidates IS 'Ordered candidates inside a Model Router route (concrete model + overrides + weight/rules).';

CREATE TABLE public.model_router_routes (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    router_id uuid NOT NULL,
    key character varying(64) NOT NULL,
    purpose text NOT NULL,
    when_to_use text NOT NULL,
    strategy character varying(32) NOT NULL,
    "position" integer DEFAULT 0 NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT model_router_routes_key_format CHECK (((key)::text ~ '^[a-z0-9]([a-z0-9-]*[a-z0-9])?$'::text)),
    CONSTRAINT model_router_routes_strategy_check CHECK (((strategy)::text = ANY ((ARRAY['single'::character varying, 'ordered_fallback'::character varying, 'weighted'::character varying, 'rules'::character varying, 'custom'::character varying])::text[])))
);

COMMENT ON TABLE public.model_router_routes IS 'Named routes inside a Model Router (key, purpose, when_to_use, strategy).';

CREATE TABLE public.model_routers (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    org_id bigint DEFAULT 1 NOT NULL,
    public_id text NOT NULL,
    name character varying(255) NOT NULL,
    description text,
    param_schema jsonb DEFAULT '{}'::jsonb NOT NULL,
    status character varying(50) DEFAULT 'active'::character varying NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    archived_at timestamp with time zone,
    deleted_at timestamp with time zone,
    CONSTRAINT model_routers_public_id_format CHECK ((public_id ~ '^mrtr_[0-9a-f]{32}$'::text)),
    CONSTRAINT model_routers_status_check CHECK (((status)::text = ANY ((ARRAY['active'::character varying, 'archived'::character varying, 'deleted'::character varying])::text[])))
);

COMMENT ON TABLE public.model_routers IS 'Model Routers — semantic LLM selection. See specs/model-router.md.';

CREATE TABLE public.models (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    org_id bigint DEFAULT 1 NOT NULL,
    provider_id uuid NOT NULL,
    model_id text NOT NULL,
    display_name text NOT NULL,
    capabilities jsonb DEFAULT '[]'::jsonb NOT NULL,
    is_favorite boolean DEFAULT false NOT NULL,
    source text DEFAULT 'manual'::text NOT NULL,
    last_seen_at timestamp with time zone,
    provider_metadata jsonb,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    enabled boolean DEFAULT false NOT NULL,
    CONSTRAINT llm_models_source_check CHECK ((source = ANY (ARRAY['manual'::text, 'discovered'::text, 'predefined'::text])))
);

COMMENT ON COLUMN public.models.is_favorite IS 'Whether this model is marked as a favorite for quick access';

COMMENT ON COLUMN public.models.source IS 'How the model was added: manual, discovered, or predefined';

COMMENT ON COLUMN public.models.last_seen_at IS 'Last time model was seen in provider API response';

COMMENT ON COLUMN public.models.provider_metadata IS 'Raw metadata from provider API response';

CREATE TABLE public.native_async_checkpoints (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    session_id uuid NOT NULL,
    turn_id uuid NOT NULL,
    org_id bigint NOT NULL,
    owner uuid NOT NULL,
    lease_until timestamp with time zone NOT NULL,
    payload_encrypted bytea NOT NULL
);

CREATE TABLE public.notification_turn_requests (
    input_message_id uuid NOT NULL,
    org_id bigint NOT NULL,
    user_id uuid NOT NULL,
    session_id uuid NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE public.notifications (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    org_id bigint NOT NULL,
    user_id uuid NOT NULL,
    kind character varying(100) NOT NULL,
    title text NOT NULL,
    body text NOT NULL,
    target_type character varying(50),
    target_id text,
    href text,
    payload jsonb DEFAULT '{}'::jsonb NOT NULL,
    dedupe_key text,
    occurrence_count integer DEFAULT 1 NOT NULL,
    viewed_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    source_type character varying(50),
    source_id text,
    source_name text,
    CONSTRAINT notifications_occurrence_count_check CHECK ((occurrence_count > 0))
);

COMMENT ON COLUMN public.notifications.source_type IS 'What sent the notification: agent or system. NULL for rows written before sources existed.';

CREATE TABLE public.oauth_authorization_codes (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    code_hash text NOT NULL,
    client_id text NOT NULL,
    user_id uuid NOT NULL,
    org_id bigint NOT NULL,
    redirect_uri text NOT NULL,
    code_challenge text NOT NULL,
    code_challenge_method text DEFAULT 'S256'::text NOT NULL,
    scope text DEFAULT 'mcp'::text NOT NULL,
    consumed boolean DEFAULT false NOT NULL,
    expires_at timestamp with time zone NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE public.oauth_clients (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    client_id text NOT NULL,
    client_secret_hash text NOT NULL,
    client_name text NOT NULL,
    redirect_uris jsonb DEFAULT '[]'::jsonb NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE public.oauth_grants (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    client_id text NOT NULL,
    user_id uuid NOT NULL,
    access text DEFAULT 'read_and_run'::text NOT NULL,
    allowed_org_ids bigint[],
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    last_used_at timestamp with time zone,
    revoked_at timestamp with time zone,
    CONSTRAINT oauth_grants_access_check CHECK ((access = ANY (ARRAY['read_only'::text, 'read_and_run'::text])))
);

CREATE TABLE public.oauth_refresh_tokens (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    token_hash text NOT NULL,
    client_id text NOT NULL,
    user_id uuid NOT NULL,
    org_id bigint NOT NULL,
    scope text DEFAULT 'mcp'::text NOT NULL,
    expires_at timestamp with time zone NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    grant_id uuid
);

CREATE TABLE public.observers (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    org_id bigint NOT NULL,
    public_id text NOT NULL,
    name text NOT NULL,
    description text,
    match_config jsonb DEFAULT '{}'::jsonb NOT NULL,
    sampling_rate double precision DEFAULT 0.1 NOT NULL,
    scorers jsonb DEFAULT '[]'::jsonb NOT NULL,
    status character varying(50) DEFAULT 'active'::character varying NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    archived_at timestamp with time zone,
    CONSTRAINT observers_public_id_format CHECK ((public_id ~ '^observer_[0-9a-f]{32}$'::text)),
    CONSTRAINT observers_sampling_rate_check CHECK (((sampling_rate >= (0.0)::double precision) AND (sampling_rate <= (1.0)::double precision))),
    CONSTRAINT observers_status_check CHECK (((status)::text = ANY ((ARRAY['active'::character varying, 'paused'::character varying, 'archived'::character varying, 'deleted'::character varying])::text[])))
);

CREATE TABLE public.org_feature_flags (
    org_id bigint NOT NULL,
    flag_name text NOT NULL,
    enabled boolean DEFAULT true NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE public.org_invitations (
    id bigint NOT NULL,
    public_id text NOT NULL,
    org_id bigint NOT NULL,
    email text NOT NULL,
    role text DEFAULT 'member'::text NOT NULL,
    invited_by uuid NOT NULL,
    token_hash text NOT NULL,
    expires_at timestamp with time zone NOT NULL,
    accepted_at timestamp with time zone,
    accepted_by uuid,
    revoked_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT org_invitations_public_id_format CHECK ((public_id ~ '^orginv_[0-9a-f]{32}$'::text)),
    CONSTRAINT org_invitations_role_check CHECK ((role = ANY (ARRAY['owner'::text, 'admin'::text, 'member'::text])))
);

CREATE SEQUENCE public.org_invitations_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;

ALTER SEQUENCE public.org_invitations_id_seq OWNED BY public.org_invitations.id;

CREATE TABLE public.org_slack_connections (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    org_id bigint NOT NULL,
    access_token_encrypted bytea,
    refresh_token_encrypted bytea,
    access_token_expires_at timestamp with time zone,
    state text DEFAULT 'connected'::text NOT NULL,
    token_generation bigint DEFAULT 1 NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    team_id text,
    team_name text,
    CONSTRAINT org_slack_connections_connected_tokens CHECK (((state <> ALL (ARRAY['connected'::text, 'rotating'::text])) OR ((access_token_encrypted IS NOT NULL) AND (refresh_token_encrypted IS NOT NULL) AND (access_token_expires_at IS NOT NULL)))),
    CONSTRAINT org_slack_connections_state_check CHECK ((state = ANY (ARRAY['connected'::text, 'rotating'::text, 'reconnect_required'::text])))
);

CREATE TABLE public.organization_members (
    org_id bigint NOT NULL,
    user_id uuid NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    role text DEFAULT 'member'::text NOT NULL,
    CONSTRAINT organization_members_role_check CHECK ((role = ANY (ARRAY['owner'::text, 'admin'::text, 'member'::text])))
);

CREATE TABLE public.organization_settings (
    org_id bigint NOT NULL,
    default_model_id uuid,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    default_harness_id uuid,
    base_harness_id uuid,
    default_provider_per_service jsonb DEFAULT '{}'::jsonb NOT NULL,
    system_decisions text DEFAULT 'deployment'::text NOT NULL,
    agentid_agents_per_owner integer,
    egress_allowlist_extension_granted boolean DEFAULT false NOT NULL,
    egress_allowlist_extension text[] DEFAULT '{}'::text[] NOT NULL,
    egress_open boolean DEFAULT false NOT NULL,
    CONSTRAINT organization_settings_agentid_agents_per_owner_check CHECK (((agentid_agents_per_owner IS NULL) OR (agentid_agents_per_owner >= 0))),
    CONSTRAINT organization_settings_egress_allowlist_extension_check CHECK ((cardinality(egress_allowlist_extension) <= 50)),
    CONSTRAINT organization_settings_system_decisions_check CHECK ((system_decisions = ANY (ARRAY['deployment'::text, 'organization'::text])))
);

CREATE TABLE public.organization_task_webhooks (
    id bigint NOT NULL,
    public_id text NOT NULL,
    org_id bigint NOT NULL,
    url text NOT NULL,
    secret text,
    enabled boolean DEFAULT true NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE SEQUENCE public.organization_task_webhooks_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;

ALTER SEQUENCE public.organization_task_webhooks_id_seq OWNED BY public.organization_task_webhooks.id;

CREATE TABLE public.organizations (
    org_id bigint NOT NULL,
    public_id text NOT NULL,
    name text NOT NULL,
    external_id text,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    created_by uuid,
    onboarding_completed_at timestamp with time zone,
    CONSTRAINT organizations_public_id_format CHECK ((public_id ~ '^org_[0-9a-f]{32}$'::text))
);

CREATE SEQUENCE public.organizations_org_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;

ALTER SEQUENCE public.organizations_org_id_seq OWNED BY public.organizations.org_id;

CREATE TABLE public.pact_device_authorizations (
    device_code_hash bytea NOT NULL,
    channel_id uuid NOT NULL,
    client_id text NOT NULL,
    user_code text NOT NULL,
    requested_scope text NOT NULL,
    status text DEFAULT 'pending'::text NOT NULL,
    brand_user_id text,
    grant_id text,
    interval_secs integer NOT NULL,
    last_polled_at timestamp with time zone,
    expires_at timestamp with time zone NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT pact_device_authorizations_status_check CHECK ((status = ANY (ARRAY['pending'::text, 'approved'::text, 'denied'::text, 'consumed'::text])))
);

CREATE TABLE public.pact_grants (
    id text NOT NULL,
    channel_id uuid NOT NULL,
    client_id text NOT NULL,
    brand_user_id text NOT NULL,
    scope text NOT NULL,
    expires_at timestamp with time zone NOT NULL,
    revoked_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE public.pact_refresh_tokens (
    token_hash bytea NOT NULL,
    grant_id text NOT NULL,
    expires_at timestamp with time zone NOT NULL,
    used_at timestamp with time zone
);

CREATE TABLE public.pact_signing_keys (
    channel_id uuid NOT NULL,
    kid text NOT NULL,
    private_key bytea NOT NULL,
    encrypted boolean NOT NULL,
    public_jwk jsonb NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE public.pact_used_assertions (
    channel_id uuid NOT NULL,
    jti text NOT NULL,
    expires_at timestamp with time zone NOT NULL
);

CREATE TABLE public.password_reset_tokens (
    id bigint NOT NULL,
    user_id uuid NOT NULL,
    token_hash text NOT NULL,
    expires_at timestamp with time zone NOT NULL,
    used_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE SEQUENCE public.password_reset_tokens_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;

ALTER SEQUENCE public.password_reset_tokens_id_seq OWNED BY public.password_reset_tokens.id;

CREATE TABLE public.payment_accounts (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    org_id bigint NOT NULL,
    owner_type text NOT NULL,
    owner_id text NOT NULL,
    rail text NOT NULL,
    label text NOT NULL,
    public_address text,
    credential_encrypted bytea,
    status text DEFAULT 'active'::text NOT NULL,
    metadata jsonb DEFAULT '{}'::jsonb NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT payment_accounts_owner_type_check CHECK ((owner_type = ANY (ARRAY['user'::text, 'virtual_user'::text, 'organization'::text]))),
    CONSTRAINT payment_accounts_rail_check CHECK ((rail = ANY (ARRAY['mpp_tempo'::text, 'x402_base'::text]))),
    CONSTRAINT payment_accounts_status_check CHECK ((status = ANY (ARRAY['active'::text, 'disabled'::text])))
);

CREATE TABLE public.payment_attempts (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    org_id bigint NOT NULL,
    payment_account_id uuid,
    session_id uuid,
    capability text NOT NULL,
    operation text NOT NULL,
    rail text,
    amount_usd double precision DEFAULT 0 NOT NULL,
    currency text DEFAULT 'usd'::text NOT NULL,
    target_url text NOT NULL,
    request_hash text,
    status text DEFAULT 'pending'::text NOT NULL,
    error_message text,
    receipt jsonb DEFAULT '{}'::jsonb NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    payment_policy_id uuid,
    input_message_id uuid,
    CONSTRAINT payment_attempts_rail_check CHECK ((rail = ANY (ARRAY['mpp_tempo'::text, 'x402_base'::text]))),
    CONSTRAINT payment_attempts_status_check CHECK ((status = ANY (ARRAY['pending'::text, 'succeeded'::text, 'failed'::text, 'released'::text])))
);

CREATE TABLE public.payment_policies (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    org_id bigint NOT NULL,
    payment_account_id uuid NOT NULL,
    subject_type text NOT NULL,
    subject_id text NOT NULL,
    allowed_capabilities text[] DEFAULT '{}'::text[] NOT NULL,
    allowed_hosts text[] DEFAULT '{}'::text[] NOT NULL,
    rail_preference text[] DEFAULT '{}'::text[] NOT NULL,
    max_amount_usd_per_request double precision,
    max_amount_usd_per_turn double precision,
    max_amount_usd_per_day double precision,
    require_approval_above_usd double precision,
    status text DEFAULT 'active'::text NOT NULL,
    metadata jsonb DEFAULT '{}'::jsonb NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT payment_policies_status_check CHECK ((status = ANY (ARRAY['active'::text, 'disabled'::text]))),
    CONSTRAINT payment_policies_subject_type_check CHECK ((subject_type = ANY (ARRAY['user'::text, 'virtual_user'::text, 'agent'::text, 'agent_channel'::text, 'session'::text, 'org'::text]))),
    CONSTRAINT payment_policy_positive_limits CHECK ((((max_amount_usd_per_request IS NULL) OR (max_amount_usd_per_request > (0)::double precision)) AND ((max_amount_usd_per_turn IS NULL) OR (max_amount_usd_per_turn > (0)::double precision)) AND ((max_amount_usd_per_day IS NULL) OR (max_amount_usd_per_day > (0)::double precision)) AND ((require_approval_above_usd IS NULL) OR (require_approval_above_usd > (0)::double precision))))
);

CREATE TABLE public.pending_user_connections (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    user_id uuid NOT NULL,
    provider character varying(50) NOT NULL,
    connection_type character varying(20) DEFAULT 'oauth'::character varying NOT NULL,
    provider_user_id text,
    provider_username text,
    access_token_encrypted bytea,
    refresh_token_encrypted bytea,
    installation_id bigint,
    scopes text,
    expires_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    provider_metadata jsonb
);

CREATE TABLE public.personal_access_tokens (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    user_id uuid NOT NULL,
    name text NOT NULL,
    token_hash text NOT NULL,
    token_prefix text NOT NULL,
    scopes jsonb DEFAULT '["*"]'::jsonb NOT NULL,
    expires_at timestamp with time zone,
    last_used_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    metadata jsonb DEFAULT '{}'::jsonb NOT NULL
);

CREATE TABLE public.pinned_sessions (
    user_id uuid NOT NULL,
    session_id uuid NOT NULL,
    org_id bigint NOT NULL,
    pinned_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE public.plugin_installs (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    org_id bigint DEFAULT 1 NOT NULL,
    public_id text NOT NULL,
    name character varying(64) NOT NULL,
    marketplace_id uuid,
    source jsonb DEFAULT '{}'::jsonb NOT NULL,
    version text,
    pinned_sha text,
    manifest jsonb DEFAULT '{}'::jsonb NOT NULL,
    definition jsonb DEFAULT '{}'::jsonb NOT NULL,
    warnings jsonb DEFAULT '[]'::jsonb NOT NULL,
    status character varying(20) DEFAULT 'active'::character varying NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT plugin_installs_name_format CHECK (((name)::text ~ '^([a-z0-9]|[a-z0-9][a-z0-9._-]*[a-z0-9])$'::text)),
    CONSTRAINT plugin_installs_name_length CHECK ((octet_length((name)::text) <= 64)),
    CONSTRAINT plugin_installs_public_id_format CHECK ((public_id ~ '^plugin_[0-9a-f]{32}$'::text)),
    CONSTRAINT plugin_installs_status_check CHECK (((status)::text = ANY ((ARRAY['active'::character varying, 'disabled'::character varying])::text[])))
);

CREATE TABLE public.plugin_marketplaces (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    org_id bigint DEFAULT 1 NOT NULL,
    public_id text NOT NULL,
    name character varying(120) NOT NULL,
    source_type character varying(20) NOT NULL,
    source jsonb DEFAULT '{}'::jsonb NOT NULL,
    status character varying(20) DEFAULT 'active'::character varying NOT NULL,
    catalog jsonb,
    last_synced_at timestamp with time zone,
    last_synced_sha text,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT plugin_marketplaces_name_format CHECK (((name)::text ~ '^[a-z][a-z0-9_-]*[a-z0-9]$'::text)),
    CONSTRAINT plugin_marketplaces_name_length CHECK ((octet_length((name)::text) <= 80)),
    CONSTRAINT plugin_marketplaces_public_id_format CHECK ((public_id ~ '^plgmkt_[0-9a-f]{32}$'::text)),
    CONSTRAINT plugin_marketplaces_source_type_check CHECK (((source_type)::text = ANY ((ARRAY['github'::character varying, 'url'::character varying, 'local_path'::character varying])::text[]))),
    CONSTRAINT plugin_marketplaces_status_check CHECK (((status)::text = ANY ((ARRAY['active'::character varying, 'disabled'::character varying])::text[])))
);

CREATE TABLE public.poppy_agent_accounts (
    org_id bigint NOT NULL,
    owner_principal_id uuid NOT NULL,
    issuer text NOT NULL,
    user_id text NOT NULL,
    dpop_jwk jsonb NOT NULL,
    dpop_key bytea NOT NULL,
    encrypted boolean NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE public.poppy_agent_conversations (
    session_id uuid NOT NULL,
    domain text NOT NULL,
    org_id bigint NOT NULL,
    issuer text NOT NULL,
    state jsonb DEFAULT '{}'::jsonb NOT NULL,
    access_token bytea,
    token_sealed boolean DEFAULT false NOT NULL,
    token_expires_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE public.poppy_agent_keys (
    id smallint DEFAULT 1 NOT NULL,
    kid text NOT NULL,
    public_jwk jsonb NOT NULL,
    private_key bytea NOT NULL,
    encrypted boolean NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT poppy_agent_keys_id_check CHECK ((id = 1))
);

CREATE TABLE public.poppy_conversations (
    session_id uuid NOT NULL,
    channel_id uuid NOT NULL,
    client_id text NOT NULL,
    user_id text NOT NULL,
    account text,
    context jsonb DEFAULT '{}'::jsonb NOT NULL,
    closed_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE public.poppy_messages (
    channel_id uuid NOT NULL,
    client_id text NOT NULL,
    user_id text NOT NULL,
    message_id text NOT NULL,
    session_id uuid NOT NULL,
    content_hash bytea NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE public.poppy_sessions (
    id text NOT NULL,
    channel_id uuid NOT NULL,
    client_id text NOT NULL,
    user_id text NOT NULL,
    account text,
    scope text DEFAULT ''::text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    renewed_at timestamp with time zone DEFAULT now() NOT NULL,
    ended_at timestamp with time zone
);

CREATE TABLE public.principals (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    public_id text NOT NULL,
    org_id bigint NOT NULL,
    kind text NOT NULL,
    subject_id uuid,
    parent_principal_id uuid,
    resolved_user_id uuid,
    metadata jsonb DEFAULT '{}'::jsonb NOT NULL,
    status text DEFAULT 'active'::text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    archived_at timestamp with time zone,
    deleted_at timestamp with time zone,
    CONSTRAINT principals_kind_check CHECK ((kind = ANY (ARRAY['user'::text, 'virtual_user'::text, 'system'::text]))),
    CONSTRAINT principals_public_id_format CHECK ((public_id ~ '^principal_[0-9a-f]{32}$'::text)),
    CONSTRAINT principals_status_check CHECK ((status = ANY (ARRAY['active'::text, 'archived'::text, 'deleted'::text])))
);

CREATE TABLE public.providers (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    org_id bigint DEFAULT 1 NOT NULL,
    name text NOT NULL,
    provider_type text NOT NULL,
    base_url text,
    api_key_encrypted bytea,
    api_key_set boolean DEFAULT false NOT NULL,
    settings jsonb DEFAULT '{}'::jsonb NOT NULL,
    status text DEFAULT 'active'::text NOT NULL,
    last_synced_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    managed boolean DEFAULT false NOT NULL,
    models_reviewed_at timestamp with time zone,
    CONSTRAINT llm_providers_status_check CHECK ((status = ANY (ARRAY['active'::text, 'disabled'::text])))
);

COMMENT ON COLUMN public.providers.settings IS 'Provider-specific settings as JSON.';

COMMENT ON COLUMN public.providers.last_synced_at IS 'Last time models were synced from provider API';

CREATE TABLE public.refresh_tokens (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    user_id uuid NOT NULL,
    token_hash text NOT NULL,
    expires_at timestamp with time zone NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE public.reporting_event_repair_cursor (
    singleton boolean DEFAULT true NOT NULL,
    last_event_created_at timestamp with time zone,
    last_event_id uuid,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT reporting_event_repair_cursor_check CHECK ((((last_event_created_at IS NULL) AND (last_event_id IS NULL)) OR ((last_event_created_at IS NOT NULL) AND (last_event_id IS NOT NULL)))),
    CONSTRAINT reporting_event_repair_cursor_singleton_check CHECK (singleton)
);

COMMENT ON TABLE public.reporting_event_repair_cursor IS 'Global watermark for bounded, cyclic verification of reporting event projections';

CREATE TABLE public.reporting_outbox (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    org_id bigint NOT NULL,
    source_type text NOT NULL,
    source_id text NOT NULL,
    source_version text DEFAULT ''::text NOT NULL,
    reason text NOT NULL,
    status text DEFAULT 'pending'::text NOT NULL,
    attempts integer DEFAULT 0 NOT NULL,
    next_attempt_at timestamp with time zone DEFAULT now() NOT NULL,
    last_error text,
    claimed_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT reporting_outbox_status_check CHECK ((status = ANY (ARRAY['pending'::text, 'processing'::text, 'completed'::text, 'failed'::text])))
);

CREATE TABLE public.reporting_saved_reports (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    org_id bigint NOT NULL,
    name text NOT NULL,
    description text,
    query jsonb NOT NULL,
    dashboard jsonb,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT reporting_saved_reports_dashboard_is_object CHECK (((dashboard IS NULL) OR (jsonb_typeof(dashboard) = 'object'::text))),
    CONSTRAINT reporting_saved_reports_name_not_blank CHECK ((length(btrim(name)) > 0)),
    CONSTRAINT reporting_saved_reports_query_is_object CHECK ((jsonb_typeof(query) = 'object'::text))
);

CREATE TABLE public.runtime_invocations (
    input_message_id uuid NOT NULL,
    org_id bigint NOT NULL,
    session_id uuid NOT NULL,
    virtual_user_id uuid,
    management_user_id uuid,
    responder_agent_id uuid
);

CREATE TABLE public.sandbox_checkpoints (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    sandbox_id uuid NOT NULL,
    generation bigint NOT NULL,
    source_turn_id text,
    source_tool_call_id text,
    kind text NOT NULL,
    provider_ref text,
    workspace_revision text NOT NULL,
    attached_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT sandbox_checkpoints_generation_check CHECK ((generation > 0)),
    CONSTRAINT sandbox_checkpoints_kind_check CHECK ((kind = ANY (ARRAY['provider_native'::text, 'portable_workspace'::text])))
);

COMMENT ON TABLE public.sandbox_checkpoints IS 'Recoverable workspace revisions (EVE-870). A row is created when the archive upload succeeds; `attached_at` is set only once the revision is committed as authoritative.';

COMMENT ON COLUMN public.sandbox_checkpoints.attached_at IS 'NULL for uploaded-but-uncommitted checkpoints, which garbage collection may remove. Never NULL for a revision any committed turn depends on.';

CREATE TABLE public.sandbox_instances (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    sandbox_id uuid NOT NULL,
    generation bigint NOT NULL,
    external_id text NOT NULL,
    display_name text,
    workspace_path text,
    provider_state jsonb DEFAULT '{}'::jsonb NOT NULL,
    metadata jsonb DEFAULT '{}'::jsonb NOT NULL,
    status text NOT NULL,
    init_completed_at timestamp with time zone,
    last_init_error text,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    retired_at timestamp with time zone,
    CONSTRAINT sandbox_instances_generation_check CHECK ((generation > 0)),
    CONSTRAINT sandbox_instances_status_check CHECK ((status = ANY (ARRAY['running'::text, 'paused'::text, 'lost'::text])))
);

COMMENT ON TABLE public.sandbox_instances IS 'Physical provider incarnations of a durable logical sandbox. Replacements advance sandboxes.generation and retire the previous row.';

COMMENT ON COLUMN public.sandbox_instances.provider_state IS 'Opaque non-secret provider state. Credentials must never be persisted here.';

CREATE TABLE public.sandbox_orphan_sightings (
    provider text NOT NULL,
    external_id text NOT NULL,
    org_id bigint NOT NULL,
    first_seen_at timestamp with time zone DEFAULT now() NOT NULL,
    last_seen_at timestamp with time zone DEFAULT now() NOT NULL,
    stopped_at timestamp with time zone,
    deleted_at timestamp with time zone
);

COMMENT ON TABLE public.sandbox_orphan_sightings IS 'Provider sandboxes labelled as Everruns-managed that no live sandbox_instances row tracks. First sighting stops; a sighting 24h later deletes.';

CREATE TABLE public.sandbox_state_transitions (
    id bigint NOT NULL,
    sandbox_id uuid NOT NULL,
    org_id bigint NOT NULL,
    generation bigint NOT NULL,
    state text NOT NULL,
    at timestamp with time zone DEFAULT now() NOT NULL
);

COMMENT ON TABLE public.sandbox_state_transitions IS 'Append-only log of observed_state and generation changes on sandboxes, written by trigger.';

CREATE SEQUENCE public.sandbox_state_transitions_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;

ALTER SEQUENCE public.sandbox_state_transitions_id_seq OWNED BY public.sandbox_state_transitions.id;

CREATE TABLE public.sandboxes (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    org_id bigint NOT NULL,
    session_id uuid,
    provider text NOT NULL,
    generation bigint DEFAULT 1 NOT NULL,
    current_checkpoint_id uuid,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    current_instance_id uuid,
    profile_name text,
    profile_snapshot jsonb,
    desired_state text DEFAULT 'ready'::text NOT NULL,
    observed_state text DEFAULT 'absent'::text NOT NULL,
    last_activity_at timestamp with time zone,
    role text DEFAULT 'primary'::text NOT NULL,
    environment_revision_id uuid,
    agent_id uuid,
    session_title text,
    deleted_at timestamp with time zone,
    CONSTRAINT sandboxes_desired_state_check CHECK ((desired_state = ANY (ARRAY['ready'::text, 'paused'::text, 'deleted'::text]))),
    CONSTRAINT sandboxes_generation_check CHECK ((generation > 0)),
    CONSTRAINT sandboxes_observed_state_check CHECK ((observed_state = ANY (ARRAY['absent'::text, 'provisioning'::text, 'ready'::text, 'paused'::text, 'lost'::text, 'deleting'::text, 'deleted'::text, 'failed'::text]))),
    CONSTRAINT sandboxes_role_check CHECK ((role = ANY (ARRAY['primary'::text, 'resource'::text])))
);

COMMENT ON TABLE public.sandboxes IS 'Logical sandbox resource (EVE-870). Authoritative product state for a session workspace; the physical provider resource is a subordinate incarnation identified by `generation`.';

COMMENT ON COLUMN public.sandboxes.generation IS 'Fencing counter. Incremented on every physical replacement so stale responses from a lost incarnation cannot advance logical state.';

COMMENT ON COLUMN public.sandboxes.profile_snapshot IS 'Fully resolved, immutable Environment profile pinned when the owning Session is created.';

COMMENT ON COLUMN public.sandboxes.desired_state IS 'Control-plane lifecycle intent. Reconciliation moves observed_state toward this value.';

COMMENT ON COLUMN public.sandboxes.role IS 'primary is implicitly addressed by shell/files; resource Sandboxes require explicit ids.';

COMMENT ON COLUMN public.sandboxes.session_title IS 'Session title captured at creation and again when the Session is deleted, so history stays readable.';

COMMENT ON COLUMN public.sandboxes.deleted_at IS 'When observed_state became deleted. Purged after the Sandbox history retention window.';

CREATE TABLE public.session_actors (
    session_id uuid NOT NULL,
    org_id bigint NOT NULL,
    runnable_since timestamp with time zone,
    inbox_seq bigint DEFAULT 0 NOT NULL,
    lease_holder text,
    lease_fence bigint DEFAULT 0 NOT NULL,
    lease_expires_at timestamp with time zone,
    cancel_requested_at timestamp with time zone,
    cancel_reported boolean DEFAULT false NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT session_actors_check CHECK (((lease_holder IS NULL) = (lease_expires_at IS NULL)))
);

CREATE TABLE public.session_compaction_checkpoints (
    id uuid NOT NULL,
    session_id uuid NOT NULL,
    source_sequence integer NOT NULL,
    provider_type text NOT NULL,
    model text NOT NULL,
    format_version integer NOT NULL,
    payload_encrypted bytea NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT session_compaction_checkpoints_format_version_check CHECK ((format_version > 0)),
    CONSTRAINT session_compaction_checkpoints_source_sequence_check CHECK ((source_sequence >= 0))
);

CREATE TABLE public.session_database_pages (
    database_id uuid NOT NULL,
    page_number integer NOT NULL,
    data bytea NOT NULL
);

CREATE TABLE public.session_databases (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    session_id uuid NOT NULL,
    name text NOT NULL,
    size_bytes bigint DEFAULT 0 NOT NULL,
    page_count integer DEFAULT 0 NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    generation bigint DEFAULT 0 NOT NULL,
    CONSTRAINT session_databases_name_check CHECK ((name ~ '^[a-zA-Z_][a-zA-Z0-9_]{0,63}$'::text))
);

CREATE TABLE public.session_git_objects (
    session_id uuid NOT NULL,
    oid bytea NOT NULL,
    obj_type smallint NOT NULL,
    size bigint NOT NULL,
    data bytea NOT NULL,
    CONSTRAINT chk_git_object_oid_len CHECK ((octet_length(oid) = 20)),
    CONSTRAINT chk_git_object_type CHECK ((obj_type = ANY (ARRAY[1, 2, 3, 4])))
);

CREATE TABLE public.session_git_refs (
    session_id uuid NOT NULL,
    name text NOT NULL,
    target bytea NOT NULL,
    is_symbolic boolean DEFAULT false NOT NULL,
    CONSTRAINT chk_git_ref_target_len CHECK (((is_symbolic = true) OR (octet_length(target) = 20)))
);

CREATE TABLE public.session_key_values (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    session_id uuid NOT NULL,
    key character varying(255) NOT NULL,
    value text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE public.session_memory_mounts (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    session_id uuid NOT NULL,
    memory_id uuid NOT NULL,
    mount_path text NOT NULL,
    access character varying(16) DEFAULT 'readonly'::character varying NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT session_memory_mounts_path_check CHECK ((mount_path ~ '^/workspace(/[^/\0]+)*$'::text)),
    CONSTRAINT session_volume_mounts_access_check CHECK (((access)::text = ANY ((ARRAY['readonly'::character varying, 'readwrite'::character varying])::text[])))
);

COMMENT ON TABLE public.session_memory_mounts IS 'Per-session snapshot of resolved memory capability mounts.';

CREATE TABLE public.session_participants (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    org_id bigint NOT NULL,
    session_id uuid NOT NULL,
    kind text NOT NULL,
    agent_id uuid,
    principal_id uuid NOT NULL,
    role text NOT NULL,
    joined_at timestamp with time zone DEFAULT now() NOT NULL,
    left_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    display_name text,
    CONSTRAINT session_participants_agent_shape CHECK ((((kind = 'agent'::text) AND (agent_id IS NOT NULL)) OR ((kind = 'user'::text) AND (agent_id IS NULL)))),
    CONSTRAINT session_participants_host_agent CHECK (((role <> 'host'::text) OR (kind = 'agent'::text))),
    CONSTRAINT session_participants_interval CHECK (((left_at IS NULL) OR (left_at >= joined_at))),
    CONSTRAINT session_participants_kind_check CHECK ((kind = ANY (ARRAY['agent'::text, 'user'::text]))),
    CONSTRAINT session_participants_role_check CHECK ((role = ANY (ARRAY['host'::text, 'member'::text])))
);

COMMENT ON TABLE public.session_participants IS 'Durable membership rows for users and agents participating in a session.';

COMMENT ON COLUMN public.session_participants.kind IS 'Participant actor kind: agent or user.';

COMMENT ON COLUMN public.session_participants.role IS 'Participant role within the session. Host is reserved for the active agent.';

COMMENT ON COLUMN public.session_participants.display_name IS 'Human-readable participant name captured from the authoritative identity profile or external actor.';

CREATE TABLE public.session_resources (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    session_id uuid NOT NULL,
    resource_id text NOT NULL,
    kind text NOT NULL,
    display_name text DEFAULT ''::text NOT NULL,
    status text DEFAULT 'active'::text NOT NULL,
    metadata jsonb DEFAULT '{}'::jsonb NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT session_resources_status_check CHECK ((status = ANY (ARRAY['active'::text, 'completed'::text, 'failed'::text, 'released'::text])))
);

CREATE TABLE public.session_schedules (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    public_id text NOT NULL,
    org_id bigint NOT NULL,
    session_id uuid NOT NULL,
    description text NOT NULL,
    cron_expression text,
    scheduled_at timestamp with time zone,
    timezone text DEFAULT 'UTC'::text NOT NULL,
    enabled boolean DEFAULT true NOT NULL,
    next_trigger_at timestamp with time zone,
    last_triggered_at timestamp with time zone,
    trigger_count integer DEFAULT 0 NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    owner_principal_id uuid NOT NULL,
    resolved_owner_user_id uuid,
    claimed_by text,
    claimed_at timestamp with time zone,
    CONSTRAINT session_schedules_has_schedule CHECK (((cron_expression IS NOT NULL) OR (scheduled_at IS NOT NULL))),
    CONSTRAINT session_schedules_public_id_format CHECK ((public_id ~ '^sched_[0-9a-f]{32}$'::text))
);

CREATE TABLE public.session_secrets (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    session_id uuid NOT NULL,
    name character varying(255) NOT NULL,
    value_encrypted bytea NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    virtual_user_id uuid
);

CREATE TABLE public.session_task_messages (
    id text NOT NULL,
    task_id text NOT NULL,
    session_id uuid NOT NULL,
    direction text NOT NULL,
    content jsonb DEFAULT '[]'::jsonb NOT NULL,
    in_reply_to text,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT session_task_messages_direction_check CHECK ((direction = ANY (ARRAY['inbound'::text, 'outbound'::text])))
);

CREATE TABLE public.session_task_push_configs (
    id bigint NOT NULL,
    public_id text NOT NULL,
    session_id uuid NOT NULL,
    task_id text NOT NULL,
    url text NOT NULL,
    secret text,
    event_filter text[] DEFAULT '{terminal}'::text[] NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE SEQUENCE public.session_task_push_configs_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;

ALTER SEQUENCE public.session_task_push_configs_id_seq OWNED BY public.session_task_push_configs.id;

CREATE TABLE public.session_tasks (
    id text NOT NULL,
    session_id uuid NOT NULL,
    kind text NOT NULL,
    display_name text DEFAULT ''::text NOT NULL,
    spec jsonb DEFAULT '{}'::jsonb NOT NULL,
    state text DEFAULT 'queued'::text NOT NULL,
    state_detail text,
    progress jsonb,
    input_request jsonb,
    cancel_requested_at timestamp with time zone,
    summary text,
    result_path text,
    artifacts jsonb DEFAULT '[]'::jsonb NOT NULL,
    error jsonb,
    attempt integer DEFAULT 1 NOT NULL,
    worker_id text,
    heartbeat_at timestamp with time zone,
    links jsonb DEFAULT '{}'::jsonb NOT NULL,
    wake_policy text DEFAULT 'silent'::text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    started_at timestamp with time zone,
    finished_at timestamp with time zone,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    root_session_id uuid,
    CONSTRAINT session_tasks_state_check CHECK ((state = ANY (ARRAY['queued'::text, 'running'::text, 'awaiting_input'::text, 'succeeded'::text, 'failed'::text, 'canceled'::text])))
);

COMMENT ON TABLE public.session_tasks IS 'Background work owned by a session (subagents, external agents, background tools). One uniform lifecycle per specs/session-tasks.md.';

CREATE TABLE public.session_trace_state (
    session_id uuid NOT NULL,
    projected_sequence integer DEFAULT 0 NOT NULL,
    turn_count integer DEFAULT 0 NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE public.session_trace_steps (
    session_id uuid NOT NULL,
    turn_no integer NOT NULL,
    step_no integer NOT NULL,
    kind text NOT NULL,
    status text NOT NULL,
    start_sequence integer NOT NULL,
    end_sequence integer,
    started_at timestamp with time zone NOT NULL,
    duration_ms bigint,
    name text,
    target text,
    result text,
    narration text,
    tool_call_id text,
    model text,
    input_tokens integer,
    output_tokens integer,
    message_count integer,
    requested_tool_call_ids text[] DEFAULT '{}'::text[] NOT NULL
);

CREATE TABLE public.session_trace_turns (
    session_id uuid NOT NULL,
    turn_no integer NOT NULL,
    turn_id uuid NOT NULL,
    start_sequence integer NOT NULL,
    end_sequence integer,
    started_at timestamp with time zone NOT NULL,
    ended_at timestamp with time zone,
    status text NOT NULL,
    prompt_preview text,
    error text,
    step_count integer DEFAULT 0 NOT NULL,
    model_calls integer DEFAULT 0 NOT NULL,
    tool_calls integer DEFAULT 0 NOT NULL,
    subagent_calls integer DEFAULT 0 NOT NULL,
    error_count integer DEFAULT 0 NOT NULL,
    input_tokens bigint DEFAULT 0 NOT NULL,
    output_tokens bigint DEFAULT 0 NOT NULL
);

CREATE TABLE public.sessions (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    agent_id uuid,
    harness_id uuid,
    org_id bigint DEFAULT 1 NOT NULL,
    title character varying(255),
    tags text[] DEFAULT '{}'::text[] NOT NULL,
    model_id uuid,
    status character varying(50) DEFAULT 'started'::character varying NOT NULL,
    capabilities jsonb DEFAULT '[]'::jsonb NOT NULL,
    tools jsonb DEFAULT '[]'::jsonb NOT NULL,
    total_input_tokens bigint DEFAULT 0 NOT NULL,
    total_output_tokens bigint DEFAULT 0 NOT NULL,
    total_cache_read_tokens bigint DEFAULT 0 NOT NULL,
    total_cache_creation_tokens bigint DEFAULT 0 NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    started_at timestamp with time zone,
    finished_at timestamp with time zone,
    parent_session_id uuid,
    locale text,
    hints jsonb,
    virtual_user_id uuid,
    blueprint_id text,
    blueprint_config jsonb,
    system_prompt text,
    initial_files jsonb DEFAULT '[]'::jsonb NOT NULL,
    max_iterations integer,
    network_access jsonb,
    mcp_servers jsonb DEFAULT '{}'::jsonb NOT NULL,
    owner_principal_id uuid NOT NULL,
    resolved_owner_user_id uuid,
    app_id uuid,
    total_actual_cost_usd double precision DEFAULT 0 NOT NULL,
    total_estimated_cost_usd double precision DEFAULT 0 NOT NULL,
    total_cost_usd double precision DEFAULT 0 NOT NULL,
    workspace_id uuid NOT NULL,
    parallel_tool_calls boolean,
    forked_from_session_id uuid,
    forked_from_sequence integer,
    root_session_id uuid,
    turn_count bigint DEFAULT 0 NOT NULL,
    tool_call_count bigint DEFAULT 0 NOT NULL,
    goal text,
    source text DEFAULT 'unknown'::text NOT NULL,
    last_turn_status text,
    last_turn_at timestamp with time zone,
    last_turn_sequence integer,
    run_summary text,
    run_summary_turn_sequence bigint,
    archived_at timestamp with time zone,
    task_count bigint DEFAULT 0 NOT NULL,
    channel_id uuid,
    turn_resolution_id uuid,
    turn_resolution_claim_token uuid,
    turn_resolution_lease_expires_at timestamp with time zone,
    turn_resolution_plan jsonb,
    trigger_id uuid,
    playground_user_id uuid,
    agent_revision bigint,
    CONSTRAINT sessions_playground_user_shape CHECK (((source = 'playground'::text) = (playground_user_id IS NOT NULL))),
    CONSTRAINT sessions_source_check CHECK ((source = ANY (ARRAY['chat'::text, 'playground'::text, 'api'::text, 'slack'::text, 'ag_ui'::text, 'fcp'::text, 'schedule'::text, 'webhook'::text, 'a2a'::text, 'eval'::text, 'subagent'::text, 'unknown'::text]))),
    CONSTRAINT sessions_status_check CHECK (((status)::text = ANY ((ARRAY['started'::character varying, 'active'::character varying, 'idle'::character varying, 'waiting_for_tool_results'::character varying, 'resolving_tool_results'::character varying, 'paused'::character varying])::text[])))
);

COMMENT ON COLUMN public.sessions.total_input_tokens IS 'Denormalized: sum of input_tokens from llm_generations';

COMMENT ON COLUMN public.sessions.total_output_tokens IS 'Denormalized: sum of output_tokens from llm_generations';

COMMENT ON COLUMN public.sessions.total_cache_read_tokens IS 'Denormalized: sum of cache_read_tokens from llm_generations';

COMMENT ON COLUMN public.sessions.total_cache_creation_tokens IS 'Denormalized: sum of cache_creation_tokens from llm_generations';

COMMENT ON COLUMN public.sessions.parent_session_id IS 'Parent session that spawned this subagent (NULL for top-level sessions)';

COMMENT ON COLUMN public.sessions.locale IS 'Optional BCP 47 locale tag for localized agent behavior (for example uk-UA)';

COMMENT ON COLUMN public.sessions.app_id IS 'Archival provenance for sessions created through the retired App model; channel_id is the live ingress pointer.';

COMMENT ON COLUMN public.sessions.total_actual_cost_usd IS 'Denormalized: sum of actual_cost_usd from llm_generations in this session.';

COMMENT ON COLUMN public.sessions.total_estimated_cost_usd IS 'Denormalized: sum of estimated_cost_usd from llm_generations in this session.';

COMMENT ON COLUMN public.sessions.total_cost_usd IS 'Denormalized best-effort spend: sum of actual_cost_usd where present, else estimated_cost_usd, per generation.';

COMMENT ON COLUMN public.sessions.turn_count IS 'Denormalized: count of turn.completed, turn.failed, and turn.cancelled events';

COMMENT ON COLUMN public.sessions.tool_call_count IS 'Denormalized: count of tool.completed events';

COMMENT ON COLUMN public.sessions.source IS 'How the session was started; server-owned for every ingress path (EVE-852)';

COMMENT ON COLUMN public.sessions.last_turn_status IS 'Denormalized: status of the highest-sequence turn.completed/failed/cancelled event';

COMMENT ON COLUMN public.sessions.run_summary IS 'Generated one-sentence description of what the run did, and where it failed (EVE-867). NULL when no utility LLM is configured, when the session is a chat thread, or before the first terminal turn.';

COMMENT ON COLUMN public.sessions.run_summary_turn_sequence IS 'events.sequence of the terminal turn `run_summary` describes. Summaries are generated out of band, so this fences a late write for an older turn from overwriting a newer summary.';

COMMENT ON COLUMN public.sessions.archived_at IS 'When the session was archived; NULL means active. Archived sessions are hidden from default list results.';

COMMENT ON COLUMN public.sessions.task_count IS 'Denormalized: total session_tasks owned by this session (EVE-868 tab badge)';

COMMENT ON COLUMN public.sessions.channel_id IS 'Channel whose ingress created this session. Archival App attribution remains in app_id.';

COMMENT ON COLUMN public.sessions.trigger_id IS 'Agent trigger whose ingress created this session (EVE-1138). NULL for user, API, endpoint and platform-created sessions. Replaces the app_channel: session tag as the budget attribution for trigger ingress.';

COMMENT ON COLUMN public.sessions.agent_revision IS 'Revision of the agent''s entity history this session started on; `everruns history show` reproduces that configuration.';

CREATE TABLE public.skill_files (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    skill_id uuid NOT NULL,
    path character varying(500) NOT NULL,
    content text,
    content_binary bytea,
    is_binary boolean DEFAULT false NOT NULL,
    size_bytes bigint DEFAULT 0 NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE public.skills (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    public_id text NOT NULL,
    org_id bigint DEFAULT 1 NOT NULL,
    name character varying(64) NOT NULL,
    description character varying(1024) NOT NULL,
    license text,
    compatibility character varying(500),
    metadata jsonb DEFAULT '{}'::jsonb NOT NULL,
    allowed_tools text,
    instructions text NOT NULL,
    source_type character varying(20) DEFAULT 'markdown'::character varying NOT NULL,
    archive_data bytea,
    status character varying(50) DEFAULT 'active'::character varying NOT NULL,
    version character varying(50) DEFAULT '1.0'::character varying NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    archived_at timestamp with time zone,
    deleted_at timestamp with time zone,
    CONSTRAINT skills_public_id_format CHECK ((public_id ~ '^skill_[0-9a-f]{32}$'::text)),
    CONSTRAINT skills_source_type_check CHECK (((source_type)::text = ANY ((ARRAY['markdown'::character varying, 'archive'::character varying])::text[]))),
    CONSTRAINT skills_status_check CHECK (((status)::text = ANY ((ARRAY['active'::character varying, 'disabled'::character varying, 'archived'::character varying, 'deleted'::character varying])::text[])))
);

CREATE TABLE public.subagent_spawn_handles (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    parent_session_id uuid NOT NULL,
    tool_call_id text NOT NULL,
    child_session_id uuid,
    status text DEFAULT 'pending'::text NOT NULL,
    terminal_result text,
    terminal_status text,
    claim_token uuid DEFAULT public.uuidv7() NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    settled_at timestamp with time zone,
    CONSTRAINT subagent_spawn_handles_status_check CHECK ((status = ANY (ARRAY['pending'::text, 'running'::text, 'settled'::text])))
);

COMMENT ON TABLE public.subagent_spawn_handles IS 'Durable spawn handles linking parent tool calls to child sessions (EVE-535). Prevents duplicate subagent spawning on parent worker reclaim/replay.';

CREATE TABLE public.trace_scores (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    org_id bigint NOT NULL,
    public_id text NOT NULL,
    observer_id uuid NOT NULL,
    scorer_key text NOT NULL,
    session_id uuid NOT NULL,
    turn_id text NOT NULL,
    agent_id uuid,
    harness_id uuid,
    status character varying(50) DEFAULT 'pending'::character varying NOT NULL,
    attempts integer DEFAULT 0 NOT NULL,
    pass boolean,
    value double precision,
    reason text,
    error_message text,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    label text,
    judge_input_tokens bigint,
    judge_output_tokens bigint,
    judge_cost_usd double precision,
    CONSTRAINT trace_scores_public_id_format CHECK ((public_id ~ '^score_[0-9a-f]{32}$'::text)),
    CONSTRAINT trace_scores_status_check CHECK (((status)::text = ANY ((ARRAY['pending'::character varying, 'scoring'::character varying, 'completed'::character varying, 'errored'::character varying, 'skipped'::character varying])::text[])))
);

CREATE TABLE public.usage_journal (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    org_id bigint NOT NULL,
    kind text NOT NULL,
    source_type text,
    source_id text,
    event_id uuid,
    session_id uuid,
    turn_id uuid,
    user_id uuid,
    principal_id uuid,
    agent_id uuid,
    harness_id uuid,
    measures jsonb DEFAULT '{}'::jsonb NOT NULL,
    metadata jsonb DEFAULT '{}'::jsonb NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE public.usage_ledger (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    journal_id uuid NOT NULL,
    budget_id uuid,
    org_id bigint NOT NULL,
    session_id uuid,
    user_id uuid,
    principal_id uuid,
    agent_id uuid,
    harness_id uuid,
    currency text NOT NULL,
    amount double precision NOT NULL,
    meter_source text NOT NULL,
    ref_type text,
    ref_id uuid,
    description text,
    rating_metadata jsonb DEFAULT '{}'::jsonb NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE public.user_oauth_identities (
    user_id uuid NOT NULL,
    provider text NOT NULL,
    provider_id text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE public.user_preferences (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    user_id uuid NOT NULL,
    key character varying(255) NOT NULL,
    value text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE public.users (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    email text NOT NULL,
    name text NOT NULL,
    avatar_url text,
    roles jsonb DEFAULT '[]'::jsonb NOT NULL,
    password_hash text,
    email_verified boolean DEFAULT false NOT NULL,
    auth_provider text,
    auth_provider_id text,
    external_id text,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE public.virtual_user_bindings (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    org_id bigint NOT NULL,
    virtual_user_id uuid NOT NULL,
    status text DEFAULT 'active'::text NOT NULL,
    provider text NOT NULL,
    realm text NOT NULL,
    subject text NOT NULL,
    management_user_id uuid,
    CONSTRAINT virtual_user_bindings_status_check CHECK ((status = ANY (ARRAY['active'::text, 'revoked'::text])))
);

CREATE TABLE public.virtual_user_connections (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    virtual_user_id uuid NOT NULL,
    provider character varying(50) NOT NULL,
    connection_type character varying(20) DEFAULT 'api_key'::character varying NOT NULL,
    provider_user_id text,
    provider_username text,
    access_token_encrypted bytea,
    refresh_token_encrypted bytea,
    installation_id bigint,
    scopes text,
    expires_at timestamp with time zone,
    provider_metadata jsonb,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    owner_scope text DEFAULT 'virtual_user'::text NOT NULL,
    name character varying(255),
    CONSTRAINT organization_connection_name CHECK (((owner_scope <> 'organization'::text) OR ((name IS NOT NULL) AND (btrim((name)::text) <> ''::text)))),
    CONSTRAINT virtual_user_connections_owner_scope_check CHECK ((owner_scope = ANY (ARRAY['virtual_user'::text, 'organization'::text])))
);

COMMENT ON COLUMN public.virtual_user_connections.owner_scope IS 'Product ownership scope. Each organization row is carried by a hidden organization virtual user.';

CREATE TABLE public.virtual_user_preferences (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    virtual_user_id uuid NOT NULL,
    key character varying(255) NOT NULL,
    value text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE public.virtual_users (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    org_id bigint DEFAULT 1 NOT NULL,
    name character varying(255) NOT NULL,
    description text,
    avatar_url text,
    locale text,
    timezone text,
    status character varying(50) DEFAULT 'active'::character varying NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    archived_at timestamp with time zone,
    deleted_at timestamp with time zone,
    usage text DEFAULT 'service'::text NOT NULL,
    CONSTRAINT agent_identities_status_check CHECK (((status)::text = ANY ((ARRAY['active'::character varying, 'archived'::character varying, 'deleted'::character varying])::text[]))),
    CONSTRAINT virtual_users_usage_check CHECK ((usage = ANY (ARRAY['end_user'::text, 'service'::text, 'organization'::text])))
);

CREATE TABLE public.workspace_file_blobs (
    file_id uuid NOT NULL,
    blob_key text NOT NULL,
    content_sha256 text NOT NULL,
    size_bytes bigint NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE public.workspace_files (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    workspace_id uuid NOT NULL,
    path text NOT NULL,
    content bytea,
    is_directory boolean DEFAULT false NOT NULL,
    is_readonly boolean DEFAULT false NOT NULL,
    size_bytes bigint DEFAULT 0 NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT workspace_files_directory_no_content CHECK (((NOT is_directory) OR (content IS NULL))),
    CONSTRAINT workspace_files_path_check CHECK ((path ~ '^/([^/\0]+(/[^/\0]+)*)?$'::text))
);

COMMENT ON TABLE public.workspace_files IS 'Files and directories inside a Workspace. Path validation mirrors the original session_files shape.';

CREATE TABLE public.workspaces (
    id uuid DEFAULT public.uuidv7() NOT NULL,
    org_id bigint DEFAULT 1 NOT NULL,
    public_id text NOT NULL,
    name character varying(255) NOT NULL,
    description text,
    owner_principal_id text,
    resolved_owner_user_id uuid,
    status character varying(50) DEFAULT 'active'::character varying NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    archived_at timestamp with time zone,
    deleted_at timestamp with time zone,
    file_count bigint DEFAULT 0 NOT NULL,
    CONSTRAINT workspaces_public_id_format CHECK ((public_id ~ '^wsp_[0-9a-f]{32}$'::text)),
    CONSTRAINT workspaces_status_check CHECK (((status)::text = ANY ((ARRAY['active'::character varying, 'archived'::character varying, 'deleted'::character varying])::text[])))
);

COMMENT ON TABLE public.workspaces IS 'Org-scoped named working areas. See specs/workspace.md.';

COMMENT ON COLUMN public.workspaces.file_count IS 'Denormalized: non-directory workspace_files rows (EVE-868 tab badge)';

ALTER TABLE ONLY public.durable_signals ALTER COLUMN sequence_num SET DEFAULT nextval('public.durable_signals_sequence_num_seq'::regclass);

ALTER TABLE ONLY public.durable_workflow_events ALTER COLUMN id SET DEFAULT nextval('public.durable_workflow_events_id_seq'::regclass);

ALTER TABLE ONLY public.durable_workflow_snapshots ALTER COLUMN id SET DEFAULT nextval('public.durable_workflow_snapshots_id_seq'::regclass);

ALTER TABLE ONLY public.email_verification_tokens ALTER COLUMN id SET DEFAULT nextval('public.email_verification_tokens_id_seq'::regclass);

ALTER TABLE ONLY public.org_invitations ALTER COLUMN id SET DEFAULT nextval('public.org_invitations_id_seq'::regclass);

ALTER TABLE ONLY public.organization_task_webhooks ALTER COLUMN id SET DEFAULT nextval('public.organization_task_webhooks_id_seq'::regclass);

ALTER TABLE ONLY public.organizations ALTER COLUMN org_id SET DEFAULT nextval('public.organizations_org_id_seq'::regclass);

ALTER TABLE ONLY public.password_reset_tokens ALTER COLUMN id SET DEFAULT nextval('public.password_reset_tokens_id_seq'::regclass);

ALTER TABLE ONLY public.sandbox_state_transitions ALTER COLUMN id SET DEFAULT nextval('public.sandbox_state_transitions_id_seq'::regclass);

ALTER TABLE ONLY public.session_task_push_configs ALTER COLUMN id SET DEFAULT nextval('public.session_task_push_configs_id_seq'::regclass);

INSERT INTO public.durable_stat_counters (name, value, shard) VALUES
	('tasks_completed', 0, 0),
	('tasks_failed', 0, 0),
	('tasks_started', 0, 0),
	('workflows_completed', 0, 0),
	('workflows_failed', 0, 0),
	('workflows_started', 0, 0),
	('tasks_completed', 0, 1),
	('tasks_failed', 0, 1),
	('tasks_started', 0, 1),
	('workflows_completed', 0, 1),
	('workflows_failed', 0, 1),
	('workflows_started', 0, 1),
	('tasks_completed', 0, 2),
	('tasks_failed', 0, 2),
	('tasks_started', 0, 2),
	('workflows_completed', 0, 2),
	('workflows_failed', 0, 2),
	('workflows_started', 0, 2),
	('tasks_completed', 0, 3),
	('tasks_failed', 0, 3),
	('tasks_started', 0, 3),
	('workflows_completed', 0, 3),
	('workflows_failed', 0, 3),
	('workflows_started', 0, 3),
	('tasks_completed', 0, 4),
	('tasks_failed', 0, 4),
	('tasks_started', 0, 4),
	('workflows_completed', 0, 4),
	('workflows_failed', 0, 4),
	('workflows_started', 0, 4),
	('tasks_completed', 0, 5),
	('tasks_failed', 0, 5),
	('tasks_started', 0, 5),
	('workflows_completed', 0, 5),
	('workflows_failed', 0, 5),
	('workflows_started', 0, 5),
	('tasks_completed', 0, 6),
	('tasks_failed', 0, 6),
	('tasks_started', 0, 6),
	('workflows_completed', 0, 6),
	('workflows_failed', 0, 6),
	('workflows_started', 0, 6),
	('tasks_completed', 0, 7),
	('tasks_failed', 0, 7),
	('tasks_started', 0, 7),
	('workflows_completed', 0, 7),
	('workflows_failed', 0, 7),
	('workflows_started', 0, 7),
	('tasks_completed', 0, 8),
	('tasks_failed', 0, 8),
	('tasks_started', 0, 8),
	('workflows_completed', 0, 8),
	('workflows_failed', 0, 8),
	('workflows_started', 0, 8),
	('tasks_completed', 0, 9),
	('tasks_failed', 0, 9),
	('tasks_started', 0, 9),
	('workflows_completed', 0, 9),
	('workflows_failed', 0, 9),
	('workflows_started', 0, 9),
	('tasks_completed', 0, 10),
	('tasks_failed', 0, 10),
	('tasks_started', 0, 10),
	('workflows_completed', 0, 10),
	('workflows_failed', 0, 10),
	('workflows_started', 0, 10),
	('tasks_completed', 0, 11),
	('tasks_failed', 0, 11),
	('tasks_started', 0, 11),
	('workflows_completed', 0, 11),
	('workflows_failed', 0, 11),
	('workflows_started', 0, 11),
	('tasks_completed', 0, 12),
	('tasks_failed', 0, 12),
	('tasks_started', 0, 12),
	('workflows_completed', 0, 12),
	('workflows_failed', 0, 12),
	('workflows_started', 0, 12),
	('tasks_completed', 0, 13),
	('tasks_failed', 0, 13),
	('tasks_started', 0, 13),
	('workflows_completed', 0, 13),
	('workflows_failed', 0, 13),
	('workflows_started', 0, 13),
	('tasks_completed', 0, 14),
	('tasks_failed', 0, 14),
	('tasks_started', 0, 14),
	('workflows_completed', 0, 14),
	('workflows_failed', 0, 14),
	('workflows_started', 0, 14),
	('tasks_completed', 0, 15),
	('tasks_failed', 0, 15),
	('tasks_started', 0, 15),
	('workflows_completed', 0, 15),
	('workflows_failed', 0, 15),
	('workflows_started', 0, 15);

INSERT INTO public.organization_settings (org_id, default_model_id, created_at, updated_at, default_harness_id, base_harness_id, default_provider_per_service, system_decisions, agentid_agents_per_owner, egress_allowlist_extension_granted, egress_allowlist_extension, egress_open) VALUES
	(1, NULL, '2026-10-11 22:34:45.157616+00', '2026-10-11 22:34:45.157616+00', NULL, NULL, '{}', 'deployment', NULL, false, '{}', false);

INSERT INTO public.organizations (org_id, public_id, name, external_id, created_at, updated_at, created_by, onboarding_completed_at) VALUES
	(1, 'org_00000000000000000000000000000001', 'Default Organization', NULL, '2026-10-11 22:34:44.521217+00', '2026-10-11 22:34:49.636781+00', NULL, '2026-10-11 22:34:44.521217+00');

INSERT INTO public.plugin_marketplaces (id, org_id, public_id, name, source_type, source, status, catalog, last_synced_at, last_synced_sha, created_at, updated_at) VALUES
	('01a12d1a-b7d8-72f2-a516-3fe00c607097', 1, 'plgmkt_01a12d1ab7d67c0caa46806bf70e3702', 'everruns', 'github', '{"repo": "everruns/plugins"}', 'active', NULL, NULL, NULL, '2026-10-11 22:34:48.145165+00', '2026-10-11 22:34:54.630494+00');

INSERT INTO public.principals (id, public_id, org_id, kind, subject_id, parent_principal_id, resolved_user_id, metadata, status, created_at, updated_at, archived_at, deleted_at) VALUES
	('01a12d1a-aed0-7e92-b378-717ffd6cb14e', 'principal_01a12d1aaed17049967a365f7473987d', 1, 'system', '00000000-0000-0000-0000-000000000001', NULL, NULL, '{"reason": "missing_human_owner", "source": "system_backfill"}', 'active', '2026-10-11 22:34:45.807618+00', '2026-10-11 22:34:45.807618+00', NULL, NULL);

INSERT INTO public.reporting_event_repair_cursor (singleton, last_event_created_at, last_event_id, updated_at) VALUES
	(true, NULL, NULL, '2026-10-11 22:34:49.926366+00');

SELECT pg_catalog.setval('public.durable_signals_sequence_num_seq', 1, false);

SELECT pg_catalog.setval('public.durable_workflow_events_id_seq', 1, false);

SELECT pg_catalog.setval('public.durable_workflow_snapshots_id_seq', 1, false);

SELECT pg_catalog.setval('public.email_verification_tokens_id_seq', 1, false);

SELECT pg_catalog.setval('public.org_invitations_id_seq', 1, false);

SELECT pg_catalog.setval('public.organization_task_webhooks_id_seq', 1, false);

SELECT pg_catalog.setval('public.organizations_org_id_seq', 1, true);

SELECT pg_catalog.setval('public.password_reset_tokens_id_seq', 1, false);

SELECT pg_catalog.setval('public.sandbox_state_transitions_id_seq', 1, false);

SELECT pg_catalog.setval('public.session_task_push_configs_id_seq', 1, false);

ALTER TABLE ONLY public.a2a_push_configs
    ADD CONSTRAINT a2a_push_configs_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.a2a_push_configs
    ADD CONSTRAINT a2a_push_configs_session_id_config_id_key UNIQUE (session_id, config_id);

ALTER TABLE ONLY public.agent_avatar_variants
    ADD CONSTRAINT agent_avatar_variants_pkey PRIMARY KEY (avatar_id, variant);

ALTER TABLE ONLY public.agent_avatars
    ADD CONSTRAINT agent_avatars_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.agent_capabilities
    ADD CONSTRAINT agent_capabilities_agent_id_capability_id_key UNIQUE (agent_id, capability_id);

ALTER TABLE ONLY public.agent_capabilities
    ADD CONSTRAINT agent_capabilities_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.agent_channels
    ADD CONSTRAINT agent_channels_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.agent_channels
    ADD CONSTRAINT agent_channels_public_id_key UNIQUE (public_id);

ALTER TABLE ONLY public.agent_check_rules
    ADD CONSTRAINT agent_check_rules_org_id_rule_id_key UNIQUE (org_id, rule_id);

ALTER TABLE ONLY public.agent_check_rules
    ADD CONSTRAINT agent_check_rules_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.agent_health_check_runs
    ADD CONSTRAINT agent_health_check_runs_org_id_public_id_key UNIQUE (org_id, public_id);

ALTER TABLE ONLY public.agent_health_check_runs
    ADD CONSTRAINT agent_health_check_runs_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.virtual_users
    ADD CONSTRAINT agent_identities_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.virtual_user_connections
    ADD CONSTRAINT agent_identity_connections_agent_identity_id_provider_key UNIQUE (virtual_user_id, provider);

ALTER TABLE ONLY public.virtual_user_connections
    ADD CONSTRAINT agent_identity_connections_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.agent_keys
    ADD CONSTRAINT agent_keys_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.agent_mcp_secret_bindings
    ADD CONSTRAINT agent_mcp_secret_bindings_agent_id_mcp_server_name_tool_nam_key UNIQUE (agent_id, mcp_server_name, tool_name, parameter_name);

ALTER TABLE ONLY public.agent_mcp_secret_bindings
    ADD CONSTRAINT agent_mcp_secret_bindings_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.agent_scripts
    ADD CONSTRAINT agent_scripts_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.agent_trigger_deliveries
    ADD CONSTRAINT agent_trigger_deliveries_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.agent_trigger_mcp_subscriptions
    ADD CONSTRAINT agent_trigger_mcp_subscriptions_pkey PRIMARY KEY (trigger_id);

ALTER TABLE ONLY public.agent_trigger_subject_values
    ADD CONSTRAINT agent_trigger_subject_values_pkey PRIMARY KEY (trigger_id, path, subject);

ALTER TABLE ONLY public.agent_triggers
    ADD CONSTRAINT agent_triggers_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.agentid_agent_owners
    ADD CONSTRAINT agentid_agent_owners_pkey PRIMARY KEY (org_id, virtual_user_id);

ALTER TABLE ONLY public.agentid_login_states
    ADD CONSTRAINT agentid_login_states_pkey PRIMARY KEY (state_hash);

ALTER TABLE ONLY public.agents_api_provider_deletions
    ADD CONSTRAINT agents_api_provider_deletions_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.agents_api_provider_deletions
    ADD CONSTRAINT agents_api_provider_deletions_provider_session_id_key UNIQUE (provider_session_id);

ALTER TABLE ONLY public.agents_api_sessions
    ADD CONSTRAINT agents_api_sessions_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.agents_api_sessions
    ADD CONSTRAINT agents_api_sessions_session_id_key UNIQUE (session_id);

ALTER TABLE ONLY public.agents
    ADD CONSTRAINT agents_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.personal_access_tokens
    ADD CONSTRAINT api_keys_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.apps
    ADD CONSTRAINT apps_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.archived_events
    ADD CONSTRAINT archived_events_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.audit_logs
    ADD CONSTRAINT audit_logs_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.budget_ledger
    ADD CONSTRAINT budget_ledger_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.budgets
    ADD CONSTRAINT budgets_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.channel_bindings
    ADD CONSTRAINT channel_bindings_pkey PRIMARY KEY (channel_id, binding_key);

ALTER TABLE ONLY public.channel_pending_deliveries
    ADD CONSTRAINT channel_pending_deliveries_pkey PRIMARY KEY (session_id, input_message_id);

ALTER TABLE ONLY public.channel_seen
    ADD CONSTRAINT channel_seen_pkey PRIMARY KEY (channel_id, dedup_key);

ALTER TABLE ONLY public.cli_auth_sessions
    ADD CONSTRAINT cli_auth_sessions_exchange_code_key UNIQUE (exchange_code);

ALTER TABLE ONLY public.cli_auth_sessions
    ADD CONSTRAINT cli_auth_sessions_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.cli_auth_sessions
    ADD CONSTRAINT cli_auth_sessions_state_key UNIQUE (state);

ALTER TABLE ONLY public.command_idempotency_keys
    ADD CONSTRAINT command_idempotency_keys_pkey PRIMARY KEY (org_id, principal_id, idempotency_key);

ALTER TABLE ONLY public.connection_setup_states
    ADD CONSTRAINT connection_setup_states_pkey PRIMARY KEY (state);

ALTER TABLE ONLY public.decision_model_defaults
    ADD CONSTRAINT decision_model_defaults_pkey PRIMARY KEY (org_id);

ALTER TABLE ONLY public.declarative_capabilities
    ADD CONSTRAINT declarative_capabilities_org_id_name_key UNIQUE (org_id, name);

ALTER TABLE ONLY public.declarative_capabilities
    ADD CONSTRAINT declarative_capabilities_org_id_public_id_key UNIQUE (org_id, public_id);

ALTER TABLE ONLY public.declarative_capabilities
    ADD CONSTRAINT declarative_capabilities_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.durable_circuit_breaker_state
    ADD CONSTRAINT durable_circuit_breaker_state_pkey PRIMARY KEY (key);

ALTER TABLE ONLY public.durable_dead_letter_queue
    ADD CONSTRAINT durable_dead_letter_queue_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.durable_schedule_executions
    ADD CONSTRAINT durable_schedule_executions_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.durable_scheduler_instances
    ADD CONSTRAINT durable_scheduler_instances_pkey PRIMARY KEY (instance_id);

ALTER TABLE ONLY public.durable_schedules
    ADD CONSTRAINT durable_schedules_name_key UNIQUE (name);

ALTER TABLE ONLY public.durable_schedules
    ADD CONSTRAINT durable_schedules_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.durable_signals
    ADD CONSTRAINT durable_signals_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.durable_stat_counters
    ADD CONSTRAINT durable_stat_counters_pkey PRIMARY KEY (name, shard);

ALTER TABLE ONLY public.durable_task_queue
    ADD CONSTRAINT durable_task_queue_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.durable_tool_results
    ADD CONSTRAINT durable_tool_results_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.durable_tool_results
    ADD CONSTRAINT durable_tool_results_turn_call_uq UNIQUE (turn_id, tool_call_id);

ALTER TABLE ONLY public.durable_workers
    ADD CONSTRAINT durable_workers_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.durable_workflow_events
    ADD CONSTRAINT durable_workflow_events_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.durable_workflow_events
    ADD CONSTRAINT durable_workflow_events_workflow_id_sequence_num_key UNIQUE (workflow_id, sequence_num);

ALTER TABLE ONLY public.durable_workflow_instances
    ADD CONSTRAINT durable_workflow_instances_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.durable_workflow_snapshots
    ADD CONSTRAINT durable_workflow_snapshots_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.durable_workflow_snapshots
    ADD CONSTRAINT durable_workflow_snapshots_workflow_id_sequence_num_key UNIQUE (workflow_id, sequence_num);

ALTER TABLE ONLY public.email_verification_tokens
    ADD CONSTRAINT email_verification_tokens_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.entity_changes
    ADD CONSTRAINT entity_changes_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.entity_manager_context
    ADD CONSTRAINT entity_manager_context_pkey PRIMARY KEY (org_id, entity_kind, entity_ref);

ALTER TABLE ONLY public.eval_case_results
    ADD CONSTRAINT eval_case_results_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.eval_cases
    ADD CONSTRAINT eval_cases_eval_id_public_id_key UNIQUE (eval_id, public_id);

ALTER TABLE ONLY public.eval_cases
    ADD CONSTRAINT eval_cases_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.eval_run_datasets
    ADD CONSTRAINT eval_run_datasets_org_id_public_id_key UNIQUE (org_id, public_id);

ALTER TABLE ONLY public.eval_run_datasets
    ADD CONSTRAINT eval_run_datasets_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.eval_run_share_tokens
    ADD CONSTRAINT eval_run_share_tokens_org_id_public_id_key UNIQUE (org_id, public_id);

ALTER TABLE ONLY public.eval_run_share_tokens
    ADD CONSTRAINT eval_run_share_tokens_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.eval_runs
    ADD CONSTRAINT eval_runs_org_id_public_id_key UNIQUE (org_id, public_id);

ALTER TABLE ONLY public.eval_runs
    ADD CONSTRAINT eval_runs_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.evals
    ADD CONSTRAINT evals_org_id_public_id_key UNIQUE (org_id, public_id);

ALTER TABLE ONLY public.evals
    ADD CONSTRAINT evals_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.event_sequences
    ADD CONSTRAINT event_sequences_pkey PRIMARY KEY (session_id);

ALTER TABLE ONLY public.events
    ADD CONSTRAINT events_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.events
    ADD CONSTRAINT events_session_id_sequence_key UNIQUE (session_id, sequence);

ALTER TABLE ONLY public.execution_environment_revisions
    ADD CONSTRAINT execution_environment_revisions_environment_id_revision_key UNIQUE (environment_id, revision);

ALTER TABLE ONLY public.execution_environment_revisions
    ADD CONSTRAINT execution_environment_revisions_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.execution_environments
    ADD CONSTRAINT execution_environments_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.fact_budget_posting
    ADD CONSTRAINT fact_budget_posting_pkey PRIMARY KEY (org_id, source_key);

ALTER TABLE ONLY public.fact_capability_usage
    ADD CONSTRAINT fact_capability_usage_pkey PRIMARY KEY (org_id, source_key);

ALTER TABLE ONLY public.fact_llm_generation
    ADD CONSTRAINT fact_llm_generation_pkey PRIMARY KEY (org_id, source_key);

ALTER TABLE ONLY public.fact_session
    ADD CONSTRAINT fact_session_pkey PRIMARY KEY (org_id, source_key);

ALTER TABLE ONLY public.fact_tool_call
    ADD CONSTRAINT fact_tool_call_pkey PRIMARY KEY (org_id, source_key);

ALTER TABLE ONLY public.fact_turn
    ADD CONSTRAINT fact_turn_pkey PRIMARY KEY (org_id, source_key);

ALTER TABLE ONLY public.files
    ADD CONSTRAINT files_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.github_apps
    ADD CONSTRAINT github_apps_agent_identity_id_key UNIQUE (virtual_user_id);

ALTER TABLE ONLY public.github_apps
    ADD CONSTRAINT github_apps_app_id_key UNIQUE (app_id);

ALTER TABLE ONLY public.github_apps
    ADD CONSTRAINT github_apps_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.harness_capabilities
    ADD CONSTRAINT harness_capabilities_harness_id_capability_id_key UNIQUE (harness_id, capability_id);

ALTER TABLE ONLY public.harness_capabilities
    ADD CONSTRAINT harness_capabilities_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.harnesses
    ADD CONSTRAINT harnesses_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.health_issue_reminders
    ADD CONSTRAINT health_issue_reminders_pkey PRIMARY KEY (issue_id, user_id);

ALTER TABLE ONLY public.health_issues
    ADD CONSTRAINT health_issues_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.image_blobs
    ADD CONSTRAINT image_blobs_pkey PRIMARY KEY (image_id);

ALTER TABLE ONLY public.images
    ADD CONSTRAINT images_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.knowledge_bases
    ADD CONSTRAINT knowledge_bases_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.knowledge_entries
    ADD CONSTRAINT knowledge_entries_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.knowledge_index_chunks
    ADD CONSTRAINT knowledge_index_chunks_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.knowledge_index_documents
    ADD CONSTRAINT knowledge_index_documents_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.knowledge_indexes
    ADD CONSTRAINT knowledge_indexes_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.leased_resources
    ADD CONSTRAINT leased_resources_org_id_provider_resource_type_external_id_key UNIQUE (org_id, provider, resource_type, external_id);

ALTER TABLE ONLY public.leased_resources
    ADD CONSTRAINT leased_resources_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.leased_resources
    ADD CONSTRAINT leased_resources_public_id_key UNIQUE (public_id);

ALTER TABLE ONLY public.llm_generations
    ADD CONSTRAINT llm_generations_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.mcp_event_subscriptions
    ADD CONSTRAINT mcp_event_subscriptions_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.mcp_event_subscriptions
    ADD CONSTRAINT mcp_event_subscriptions_subscription_key_key UNIQUE (subscription_key);

ALTER TABLE ONLY public.mcp_servers
    ADD CONSTRAINT mcp_servers_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.mcp_service_tool_caches
    ADD CONSTRAINT mcp_service_tool_caches_pkey PRIMARY KEY (org_id, mcp_server_id, agent_id, cache_scope, credential_hash);

ALTER TABLE ONLY public.mcp_tool_labels
    ADD CONSTRAINT mcp_tool_labels_mcp_server_id_tool_name_key UNIQUE (mcp_server_id, tool_name);

ALTER TABLE ONLY public.mcp_tool_labels
    ADD CONSTRAINT mcp_tool_labels_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.message_feedback
    ADD CONSTRAINT message_feedback_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.message_feedback
    ADD CONSTRAINT message_feedback_session_id_message_id_user_id_key UNIQUE (session_id, message_id, user_id);

ALTER TABLE ONLY public.model_router_candidates
    ADD CONSTRAINT model_router_candidates_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.model_router_routes
    ADD CONSTRAINT model_router_routes_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.model_routers
    ADD CONSTRAINT model_routers_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.models
    ADD CONSTRAINT models_org_id_identity UNIQUE (org_id, id);

ALTER TABLE ONLY public.models
    ADD CONSTRAINT models_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.native_async_checkpoints
    ADD CONSTRAINT native_async_checkpoints_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.native_async_checkpoints
    ADD CONSTRAINT native_async_checkpoints_session_id_turn_id_key UNIQUE (session_id, turn_id);

ALTER TABLE ONLY public.notification_turn_requests
    ADD CONSTRAINT notification_turn_requests_pkey PRIMARY KEY (input_message_id);

ALTER TABLE ONLY public.notifications
    ADD CONSTRAINT notifications_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.oauth_authorization_codes
    ADD CONSTRAINT oauth_authorization_codes_code_hash_key UNIQUE (code_hash);

ALTER TABLE ONLY public.oauth_authorization_codes
    ADD CONSTRAINT oauth_authorization_codes_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.oauth_clients
    ADD CONSTRAINT oauth_clients_client_id_key UNIQUE (client_id);

ALTER TABLE ONLY public.oauth_clients
    ADD CONSTRAINT oauth_clients_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.oauth_grants
    ADD CONSTRAINT oauth_grants_client_id_user_id_key UNIQUE (client_id, user_id);

ALTER TABLE ONLY public.oauth_grants
    ADD CONSTRAINT oauth_grants_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.oauth_refresh_tokens
    ADD CONSTRAINT oauth_refresh_tokens_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.oauth_refresh_tokens
    ADD CONSTRAINT oauth_refresh_tokens_token_hash_key UNIQUE (token_hash);

ALTER TABLE ONLY public.observers
    ADD CONSTRAINT observers_org_id_public_id_key UNIQUE (org_id, public_id);

ALTER TABLE ONLY public.observers
    ADD CONSTRAINT observers_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.org_feature_flags
    ADD CONSTRAINT org_feature_flags_pkey PRIMARY KEY (org_id, flag_name);

ALTER TABLE ONLY public.org_invitations
    ADD CONSTRAINT org_invitations_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.org_invitations
    ADD CONSTRAINT org_invitations_public_id_key UNIQUE (public_id);

ALTER TABLE ONLY public.org_slack_connections
    ADD CONSTRAINT org_slack_connections_org_team_key UNIQUE (org_id, team_id);

ALTER TABLE ONLY public.org_slack_connections
    ADD CONSTRAINT org_slack_connections_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.organization_members
    ADD CONSTRAINT organization_members_pkey PRIMARY KEY (org_id, user_id);

ALTER TABLE ONLY public.organization_settings
    ADD CONSTRAINT organization_settings_pkey PRIMARY KEY (org_id);

ALTER TABLE ONLY public.organization_task_webhooks
    ADD CONSTRAINT organization_task_webhooks_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.organization_task_webhooks
    ADD CONSTRAINT organization_task_webhooks_public_id_key UNIQUE (public_id);

ALTER TABLE ONLY public.organizations
    ADD CONSTRAINT organizations_external_id_key UNIQUE (external_id);

ALTER TABLE ONLY public.organizations
    ADD CONSTRAINT organizations_pkey PRIMARY KEY (org_id);

ALTER TABLE ONLY public.organizations
    ADD CONSTRAINT organizations_public_id_key UNIQUE (public_id);

ALTER TABLE ONLY public.pact_device_authorizations
    ADD CONSTRAINT pact_device_authorizations_pkey PRIMARY KEY (device_code_hash);

ALTER TABLE ONLY public.pact_grants
    ADD CONSTRAINT pact_grants_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.pact_refresh_tokens
    ADD CONSTRAINT pact_refresh_tokens_pkey PRIMARY KEY (token_hash);

ALTER TABLE ONLY public.pact_signing_keys
    ADD CONSTRAINT pact_signing_keys_pkey PRIMARY KEY (channel_id);

ALTER TABLE ONLY public.pact_used_assertions
    ADD CONSTRAINT pact_used_assertions_pkey PRIMARY KEY (channel_id, jti);

ALTER TABLE ONLY public.password_reset_tokens
    ADD CONSTRAINT password_reset_tokens_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.payment_accounts
    ADD CONSTRAINT payment_accounts_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.payment_attempts
    ADD CONSTRAINT payment_attempts_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.payment_policies
    ADD CONSTRAINT payment_policies_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.pinned_sessions
    ADD CONSTRAINT pinned_sessions_pkey PRIMARY KEY (user_id, session_id);

ALTER TABLE ONLY public.plugin_installs
    ADD CONSTRAINT plugin_installs_org_id_name_key UNIQUE (org_id, name);

ALTER TABLE ONLY public.plugin_installs
    ADD CONSTRAINT plugin_installs_org_id_public_id_key UNIQUE (org_id, public_id);

ALTER TABLE ONLY public.plugin_installs
    ADD CONSTRAINT plugin_installs_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.plugin_marketplaces
    ADD CONSTRAINT plugin_marketplaces_org_id_name_key UNIQUE (org_id, name);

ALTER TABLE ONLY public.plugin_marketplaces
    ADD CONSTRAINT plugin_marketplaces_org_id_public_id_key UNIQUE (org_id, public_id);

ALTER TABLE ONLY public.plugin_marketplaces
    ADD CONSTRAINT plugin_marketplaces_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.poppy_agent_accounts
    ADD CONSTRAINT poppy_agent_accounts_pkey PRIMARY KEY (org_id, owner_principal_id, issuer);

ALTER TABLE ONLY public.poppy_agent_conversations
    ADD CONSTRAINT poppy_agent_conversations_pkey PRIMARY KEY (session_id, domain);

ALTER TABLE ONLY public.poppy_agent_keys
    ADD CONSTRAINT poppy_agent_keys_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.poppy_conversations
    ADD CONSTRAINT poppy_conversations_pkey PRIMARY KEY (session_id);

ALTER TABLE ONLY public.poppy_messages
    ADD CONSTRAINT poppy_messages_pkey PRIMARY KEY (channel_id, client_id, user_id, message_id);

ALTER TABLE ONLY public.poppy_sessions
    ADD CONSTRAINT poppy_sessions_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.principals
    ADD CONSTRAINT principals_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.principals
    ADD CONSTRAINT principals_public_id_key UNIQUE (public_id);

ALTER TABLE ONLY public.providers
    ADD CONSTRAINT providers_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.refresh_tokens
    ADD CONSTRAINT refresh_tokens_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.reporting_event_repair_cursor
    ADD CONSTRAINT reporting_event_repair_cursor_pkey PRIMARY KEY (singleton);

ALTER TABLE ONLY public.reporting_outbox
    ADD CONSTRAINT reporting_outbox_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.reporting_outbox
    ADD CONSTRAINT reporting_outbox_source_unique UNIQUE (org_id, source_type, source_id, source_version, reason);

ALTER TABLE ONLY public.reporting_saved_reports
    ADD CONSTRAINT reporting_saved_reports_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.runtime_invocations
    ADD CONSTRAINT runtime_invocations_pkey PRIMARY KEY (input_message_id);

ALTER TABLE ONLY public.sandbox_checkpoints
    ADD CONSTRAINT sandbox_checkpoints_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.sandbox_checkpoints
    ADD CONSTRAINT sandbox_checkpoints_revision_uq UNIQUE (sandbox_id, workspace_revision);

ALTER TABLE ONLY public.sandbox_instances
    ADD CONSTRAINT sandbox_instances_generation_uq UNIQUE (sandbox_id, generation);

ALTER TABLE ONLY public.sandbox_instances
    ADD CONSTRAINT sandbox_instances_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.sandbox_orphan_sightings
    ADD CONSTRAINT sandbox_orphan_sightings_pkey PRIMARY KEY (provider, external_id);

ALTER TABLE ONLY public.sandbox_state_transitions
    ADD CONSTRAINT sandbox_state_transitions_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.sandboxes
    ADD CONSTRAINT sandboxes_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.sandboxes
    ADD CONSTRAINT sandboxes_session_provider_uq UNIQUE (session_id, provider);

ALTER TABLE ONLY public.session_actors
    ADD CONSTRAINT session_actors_pkey PRIMARY KEY (session_id);

ALTER TABLE ONLY public.session_compaction_checkpoints
    ADD CONSTRAINT session_compaction_checkpoint_session_id_provider_type_mode_key UNIQUE (session_id, provider_type, model, format_version);

ALTER TABLE ONLY public.session_compaction_checkpoints
    ADD CONSTRAINT session_compaction_checkpoints_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.session_database_pages
    ADD CONSTRAINT session_database_pages_pkey PRIMARY KEY (database_id, page_number);

ALTER TABLE ONLY public.session_databases
    ADD CONSTRAINT session_databases_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.session_databases
    ADD CONSTRAINT session_databases_unique_name UNIQUE (session_id, name);

ALTER TABLE ONLY public.workspace_files
    ADD CONSTRAINT session_files_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.session_git_objects
    ADD CONSTRAINT session_git_objects_pkey PRIMARY KEY (session_id, oid);

ALTER TABLE ONLY public.session_git_refs
    ADD CONSTRAINT session_git_refs_pkey PRIMARY KEY (session_id, name);

ALTER TABLE ONLY public.session_key_values
    ADD CONSTRAINT session_key_values_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.session_key_values
    ADD CONSTRAINT session_key_values_unique_key UNIQUE (session_id, key);

ALTER TABLE ONLY public.session_participants
    ADD CONSTRAINT session_participants_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.session_resources
    ADD CONSTRAINT session_resources_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.session_resources
    ADD CONSTRAINT session_resources_session_id_resource_id_key UNIQUE (session_id, resource_id);

ALTER TABLE ONLY public.session_schedules
    ADD CONSTRAINT session_schedules_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.session_secrets
    ADD CONSTRAINT session_secrets_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.session_secrets
    ADD CONSTRAINT session_secrets_unique_name UNIQUE (session_id, name);

ALTER TABLE ONLY public.session_task_messages
    ADD CONSTRAINT session_task_messages_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.session_task_push_configs
    ADD CONSTRAINT session_task_push_configs_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.session_task_push_configs
    ADD CONSTRAINT session_task_push_configs_public_id_key UNIQUE (public_id);

ALTER TABLE ONLY public.session_tasks
    ADD CONSTRAINT session_tasks_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.session_trace_state
    ADD CONSTRAINT session_trace_state_pkey PRIMARY KEY (session_id);

ALTER TABLE ONLY public.session_trace_steps
    ADD CONSTRAINT session_trace_steps_pkey PRIMARY KEY (session_id, turn_no, step_no);

ALTER TABLE ONLY public.session_trace_turns
    ADD CONSTRAINT session_trace_turns_pkey PRIMARY KEY (session_id, turn_no);

ALTER TABLE ONLY public.session_memory_mounts
    ADD CONSTRAINT session_volume_mounts_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.sessions
    ADD CONSTRAINT sessions_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.skill_files
    ADD CONSTRAINT skill_files_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.skill_files
    ADD CONSTRAINT skill_files_skill_id_path_key UNIQUE (skill_id, path);

ALTER TABLE ONLY public.skills
    ADD CONSTRAINT skills_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.subagent_spawn_handles
    ADD CONSTRAINT subagent_spawn_handles_parent_call_uq UNIQUE (parent_session_id, tool_call_id);

ALTER TABLE ONLY public.subagent_spawn_handles
    ADD CONSTRAINT subagent_spawn_handles_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.trace_scores
    ADD CONSTRAINT trace_scores_observer_id_scorer_key_session_id_turn_id_key UNIQUE (observer_id, scorer_key, session_id, turn_id);

ALTER TABLE ONLY public.trace_scores
    ADD CONSTRAINT trace_scores_org_id_public_id_key UNIQUE (org_id, public_id);

ALTER TABLE ONLY public.trace_scores
    ADD CONSTRAINT trace_scores_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.usage_journal
    ADD CONSTRAINT usage_journal_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.usage_ledger
    ADD CONSTRAINT usage_ledger_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.pending_user_connections
    ADD CONSTRAINT user_connections_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.user_oauth_identities
    ADD CONSTRAINT user_oauth_identities_pkey PRIMARY KEY (provider, provider_id);

ALTER TABLE ONLY public.user_oauth_identities
    ADD CONSTRAINT user_oauth_identities_user_id_provider_key UNIQUE (user_id, provider);

ALTER TABLE ONLY public.user_preferences
    ADD CONSTRAINT user_preferences_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.user_preferences
    ADD CONSTRAINT user_preferences_unique_key UNIQUE (user_id, key);

ALTER TABLE ONLY public.users
    ADD CONSTRAINT users_external_id_key UNIQUE (external_id);

ALTER TABLE ONLY public.users
    ADD CONSTRAINT users_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.virtual_user_bindings
    ADD CONSTRAINT virtual_user_bindings_org_id_provider_realm_subject_key UNIQUE (org_id, provider, realm, subject);

ALTER TABLE ONLY public.virtual_user_bindings
    ADD CONSTRAINT virtual_user_bindings_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.virtual_user_preferences
    ADD CONSTRAINT virtual_user_preferences_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.virtual_user_preferences
    ADD CONSTRAINT virtual_user_preferences_virtual_user_id_key_key UNIQUE (virtual_user_id, key);

ALTER TABLE ONLY public.virtual_users
    ADD CONSTRAINT virtual_users_org_id_id_unique UNIQUE (org_id, id);

ALTER TABLE ONLY public.memory_files
    ADD CONSTRAINT volume_files_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.memories
    ADD CONSTRAINT volumes_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.workspace_file_blobs
    ADD CONSTRAINT workspace_file_blobs_pkey PRIMARY KEY (file_id);

ALTER TABLE ONLY public.workspaces
    ADD CONSTRAINT workspaces_pkey PRIMARY KEY (id);

CREATE INDEX agent_avatars_agent_idx ON public.agent_avatars USING btree (agent_id);

CREATE UNIQUE INDEX agent_channels_durable_schedule_id_idx ON public.agent_channels USING btree (durable_schedule_id) WHERE (durable_schedule_id IS NOT NULL);

CREATE UNIQUE INDEX agent_keys_previous_token_hash_idx ON public.agent_keys USING btree (previous_token_hash) WHERE (previous_token_hash IS NOT NULL);

CREATE UNIQUE INDEX agent_keys_token_hash_idx ON public.agent_keys USING btree (token_hash);

CREATE UNIQUE INDEX agent_scripts_agent_name_active_idx ON public.agent_scripts USING btree (agent_id, name) WHERE ((status)::text = 'active'::text);

CREATE UNIQUE INDEX agent_trigger_deliveries_event_id_idx ON public.agent_trigger_deliveries USING btree (trigger_id, event_id) WHERE ((event_id IS NOT NULL) AND ((status)::text = ANY ((ARRAY['dispatched'::character varying, 'filtered'::character varying])::text[])));

CREATE UNIQUE INDEX agent_triggers_durable_schedule_id_idx ON public.agent_triggers USING btree (durable_schedule_id) WHERE (durable_schedule_id IS NOT NULL);

CREATE UNIQUE INDEX agent_triggers_ingress_id_idx ON public.agent_triggers USING btree (ingress_id) WHERE (ingress_id IS NOT NULL);

CREATE INDEX agentid_agent_owners_owner ON public.agentid_agent_owners USING btree (org_id, owner_sub);

CREATE INDEX agentid_login_states_expiry ON public.agentid_login_states USING btree (expires_at);

CREATE INDEX command_idempotency_keys_expires_idx ON public.command_idempotency_keys USING btree (expires_at);

CREATE INDEX connection_setup_states_expiry ON public.connection_setup_states USING btree (expires_at);

CREATE INDEX durable_tool_results_turn_idx ON public.durable_tool_results USING btree (turn_id);

CREATE INDEX entity_changes_entity_idx ON public.entity_changes USING btree (org_id, entity_kind, entity_ref, created_at DESC, id DESC);

CREATE INDEX entity_changes_org_idx ON public.entity_changes USING btree (org_id, created_at DESC, id DESC);

CREATE UNIQUE INDEX execution_environments_org_name_active_uq ON public.execution_environments USING btree (org_id, name) WHERE (status = 'active'::text);

CREATE UNIQUE INDEX health_issues_channel_code ON public.health_issues USING btree (org_id, channel_id, code) WHERE (channel_id IS NOT NULL);

CREATE UNIQUE INDEX health_issues_org_code ON public.health_issues USING btree (org_id, code) WHERE (channel_id IS NULL);

CREATE INDEX health_issues_org_status ON public.health_issues USING btree (org_id, status, first_detected_at DESC);

CREATE INDEX idx_agent_capabilities_agent_id ON public.agent_capabilities USING btree (agent_id);

CREATE INDEX idx_agent_capabilities_position ON public.agent_capabilities USING btree (agent_id, "position");

CREATE INDEX idx_agent_channels_agent_id ON public.agent_channels USING btree (agent_id);

CREATE INDEX idx_agent_channels_app_id ON public.agent_channels USING btree (app_id);

CREATE INDEX idx_agent_channels_legacy_alias_channel_type ON public.agent_channels USING btree (legacy_alias_id, channel_type);

CREATE INDEX idx_agent_channels_status ON public.agent_channels USING btree (status);

CREATE INDEX idx_agent_check_rules_org_id ON public.agent_check_rules USING btree (org_id);

CREATE INDEX idx_agent_health_check_runs_agent_config ON public.agent_health_check_runs USING btree (agent_id, config_hash, created_at DESC);

CREATE INDEX idx_agent_health_check_runs_org_id ON public.agent_health_check_runs USING btree (org_id);

CREATE INDEX idx_agent_identities_org_id ON public.virtual_users USING btree (org_id);

CREATE INDEX idx_agent_identities_org_name ON public.virtual_users USING btree (org_id, name);

CREATE INDEX idx_agent_identities_org_status ON public.virtual_users USING btree (org_id, status);

CREATE INDEX idx_agent_identity_connections_identity ON public.virtual_user_connections USING btree (virtual_user_id);

CREATE INDEX idx_agent_keys_org_channel ON public.agent_keys USING btree (org_id, channel_id);

CREATE INDEX idx_agent_mcp_secret_bindings_agent ON public.agent_mcp_secret_bindings USING btree (org_id, agent_id);

CREATE INDEX idx_agent_scripts_agent_id ON public.agent_scripts USING btree (agent_id);

CREATE INDEX idx_agent_scripts_org_id ON public.agent_scripts USING btree (org_id);

CREATE INDEX idx_agent_trigger_deliveries_trigger_created ON public.agent_trigger_deliveries USING btree (trigger_id, created_at DESC);

CREATE INDEX idx_agent_trigger_mcp_subscriptions_refresh ON public.agent_trigger_mcp_subscriptions USING btree (refresh_before) WHERE ((status)::text <> 'pending'::text);

CREATE INDEX idx_agent_trigger_subject_values_trigger_updated ON public.agent_trigger_subject_values USING btree (trigger_id, updated_at DESC);

CREATE INDEX idx_agent_triggers_agent_id ON public.agent_triggers USING btree (agent_id);

CREATE INDEX idx_agent_triggers_org_id ON public.agent_triggers USING btree (org_id);

CREATE INDEX idx_agents_agent_identity_id ON public.agents USING btree (virtual_user_id) WHERE (virtual_user_id IS NOT NULL);

CREATE INDEX idx_agents_api_provider_deletions_due ON public.agents_api_provider_deletions USING btree (next_attempt_at) WHERE (state = 'pending'::text);

CREATE INDEX idx_agents_api_sessions_idle_provider ON public.agents_api_sessions USING btree (updated_at) WHERE (provider_session_id IS NOT NULL);

CREATE INDEX idx_agents_forked_from_agent_id ON public.agents USING btree (forked_from_agent_id);

CREATE INDEX idx_agents_harness_id ON public.agents USING btree (harness_id);

CREATE INDEX idx_agents_org_built_in ON public.agents USING btree (org_id) WHERE is_built_in;

CREATE INDEX idx_agents_org_id ON public.agents USING btree (org_id);

CREATE UNIQUE INDEX idx_agents_org_name ON public.agents USING btree (org_id, name) WHERE ((status)::text <> 'deleted'::text);

CREATE UNIQUE INDEX idx_agents_org_public_id ON public.agents USING btree (org_id, public_id);

CREATE INDEX idx_agents_root_agent_id ON public.agents USING btree (root_agent_id);

CREATE INDEX idx_agents_status ON public.agents USING btree (status);

CREATE INDEX idx_agents_tags ON public.agents USING gin (tags);

CREATE INDEX idx_apps_agent_identity_id ON public.apps USING btree (virtual_user_id);

CREATE INDEX idx_apps_org_agent_identity_id ON public.apps USING btree (org_id, virtual_user_id);

CREATE INDEX idx_apps_org_id ON public.apps USING btree (org_id);

CREATE UNIQUE INDEX idx_apps_org_public_id ON public.apps USING btree (org_id, public_id);

CREATE INDEX idx_apps_org_status ON public.apps USING btree (org_id, status);

CREATE INDEX idx_apps_owner_principal ON public.apps USING btree (owner_principal_id);

CREATE UNIQUE INDEX idx_apps_public_id ON public.apps USING btree (public_id);

CREATE INDEX idx_apps_resolved_owner ON public.apps USING btree (org_id, resolved_owner_user_id);

CREATE INDEX idx_archived_events_session ON public.archived_events USING btree (session_id);

CREATE INDEX idx_archived_events_ts ON public.archived_events USING btree (ts);

CREATE INDEX idx_audit_logs_actor ON public.audit_logs USING btree (actor_id, created_at DESC);

CREATE INDEX idx_audit_logs_event_type ON public.audit_logs USING btree (event_type, created_at DESC);

CREATE INDEX idx_audit_logs_org_action_created ON public.audit_logs USING btree (org_id, action, created_at DESC) WHERE (action <> ''::text);

CREATE INDEX idx_audit_logs_org_created ON public.audit_logs USING btree (org_id, created_at DESC);

CREATE INDEX idx_audit_logs_org_domain_created ON public.audit_logs USING btree (org_id, domain, created_at DESC);

CREATE INDEX idx_budget_ledger_budget ON public.budget_ledger USING btree (budget_id, created_at);

CREATE INDEX idx_budget_ledger_session ON public.budget_ledger USING btree (session_id, created_at);

CREATE INDEX idx_budgets_subject ON public.budgets USING btree (org_id, subject_type, subject_id) WHERE (status = 'active'::text);

CREATE INDEX idx_channel_bindings_session ON public.channel_bindings USING btree (session_id);

CREATE INDEX idx_channel_pending_deliveries_lease ON public.channel_pending_deliveries USING btree (lease_until);

CREATE INDEX idx_channel_seen_age ON public.channel_seen USING btree (channel_id, seen_at);

CREATE INDEX idx_cli_auth_sessions_exchange_code ON public.cli_auth_sessions USING btree (exchange_code);

CREATE INDEX idx_cli_auth_sessions_expires_at ON public.cli_auth_sessions USING btree (expires_at);

CREATE INDEX idx_cli_auth_sessions_state ON public.cli_auth_sessions USING btree (state);

CREATE INDEX idx_declarative_capabilities_org_id ON public.declarative_capabilities USING btree (org_id);

CREATE INDEX idx_declarative_capabilities_status ON public.declarative_capabilities USING btree (status);

CREATE INDEX idx_durable_dlq_activity_type ON public.durable_dead_letter_queue USING btree (activity_type);

CREATE INDEX idx_durable_dlq_dead_at ON public.durable_dead_letter_queue USING btree (dead_at);

CREATE INDEX idx_durable_dlq_workflow ON public.durable_dead_letter_queue USING btree (workflow_id);

CREATE INDEX idx_durable_schedule_executions_running ON public.durable_schedule_executions USING btree (schedule_id) WHERE (status = 'running'::text);

CREATE INDEX idx_durable_schedule_executions_schedule ON public.durable_schedule_executions USING btree (schedule_id, created_at DESC);

CREATE INDEX idx_durable_scheduler_instances_heartbeat ON public.durable_scheduler_instances USING btree (last_heartbeat_at);

CREATE INDEX idx_durable_schedules_polling ON public.durable_schedules USING btree (next_trigger_at) INCLUDE (claimed_by, claimed_at) WHERE (enabled = true);

CREATE INDEX idx_durable_signals_pending ON public.durable_signals USING btree (workflow_id, sequence_num) WHERE (processed_at IS NULL);

CREATE INDEX idx_durable_task_queue_claimed ON public.durable_task_queue USING btree (claimed_by, heartbeat_at) WHERE (status = 'claimed'::text);

CREATE INDEX idx_durable_task_queue_claimed_by ON public.durable_task_queue USING btree (claimed_by) WHERE (claimed_by IS NOT NULL);

CREATE INDEX idx_durable_task_queue_created_at ON public.durable_task_queue USING btree (created_at);

CREATE INDEX idx_durable_task_queue_pending ON public.durable_task_queue USING btree (priority DESC, visible_at, activity_type) WHERE (status = 'pending'::text);

CREATE INDEX idx_durable_task_queue_stale_reclaim ON public.durable_task_queue USING btree (heartbeat_at) WHERE (status = 'claimed'::text);

CREATE INDEX idx_durable_task_queue_standalone ON public.durable_task_queue USING btree (created_at DESC) WHERE (workflow_id IS NULL);

CREATE UNIQUE INDEX idx_durable_task_queue_waiting_turn_resolution ON public.durable_task_queue USING btree (workflow_id, activity_id) WHERE ((workflow_id IS NOT NULL) AND (activity_id ~~ 'waiting_turn_resolution_%'::text));

CREATE INDEX idx_durable_task_queue_workflow ON public.durable_task_queue USING btree (workflow_id);

CREATE INDEX idx_durable_workers_group ON public.durable_workers USING btree (worker_group);

CREATE INDEX idx_durable_workers_heartbeat ON public.durable_workers USING btree (last_heartbeat_at);

CREATE INDEX idx_durable_workers_status ON public.durable_workers USING btree (status) WHERE (status = 'active'::text);

CREATE INDEX idx_durable_workflow_events_workflow ON public.durable_workflow_events USING btree (workflow_id, sequence_num);

CREATE INDEX idx_durable_workflow_instances_continued_as_new_id ON public.durable_workflow_instances USING btree (continued_as_new_id) WHERE (continued_as_new_id IS NOT NULL);

CREATE INDEX idx_durable_workflow_instances_created ON public.durable_workflow_instances USING btree (created_at);

CREATE INDEX idx_durable_workflow_instances_status ON public.durable_workflow_instances USING btree (status);

CREATE INDEX idx_durable_workflow_instances_type ON public.durable_workflow_instances USING btree (workflow_type);

CREATE INDEX idx_durable_workflow_snapshots_latest ON public.durable_workflow_snapshots USING btree (workflow_id, sequence_num DESC);

CREATE UNIQUE INDEX idx_email_verification_tokens_token_hash ON public.email_verification_tokens USING btree (token_hash);

CREATE INDEX idx_email_verification_tokens_user_id ON public.email_verification_tokens USING btree (user_id);

CREATE INDEX idx_eval_case_results_case_id ON public.eval_case_results USING btree (eval_case_id);

CREATE INDEX idx_eval_case_results_run_id ON public.eval_case_results USING btree (eval_run_id);

CREATE INDEX idx_eval_cases_eval_id ON public.eval_cases USING btree (eval_id);

CREATE INDEX idx_eval_run_datasets_org_id ON public.eval_run_datasets USING btree (org_id);

CREATE UNIQUE INDEX idx_eval_run_datasets_request ON public.eval_run_datasets USING btree (org_id, eval_run_id, request) WHERE (eval_run_id IS NOT NULL);

CREATE INDEX idx_eval_run_datasets_run ON public.eval_run_datasets USING btree (eval_run_id, created_at DESC);

CREATE INDEX idx_eval_run_share_tokens_org_id ON public.eval_run_share_tokens USING btree (org_id);

CREATE INDEX idx_eval_run_share_tokens_run ON public.eval_run_share_tokens USING btree (eval_run_id);

CREATE UNIQUE INDEX idx_eval_run_share_tokens_token_hash ON public.eval_run_share_tokens USING btree (token_hash);

CREATE INDEX idx_eval_runs_eval_id ON public.eval_runs USING btree (eval_id);

CREATE INDEX idx_eval_runs_org_id ON public.eval_runs USING btree (org_id);

CREATE INDEX idx_eval_runs_org_id_status ON public.eval_runs USING btree (org_id, status);

CREATE INDEX idx_eval_runs_source_run_id ON public.eval_runs USING btree (org_id, source_run_id) WHERE (source_run_id IS NOT NULL);

CREATE INDEX idx_evals_org_id ON public.evals USING btree (org_id);

CREATE INDEX idx_evals_public_id ON public.evals USING btree (public_id);

CREATE INDEX idx_events_messages ON public.events USING btree (session_id, sequence) WHERE ((event_type)::text = ANY ((ARRAY['input.message'::character varying, 'output.message.completed'::character varying, 'tool.completed'::character varying])::text[]));

COMMENT ON INDEX public.idx_events_messages IS 'Partial index for message reads (input.message, output.message.completed, tool.completed)';

CREATE INDEX idx_events_reporting_repair ON public.events USING btree (created_at, id) WHERE ((event_type)::text = ANY ((ARRAY['tool.completed'::character varying, 'capability.usage'::character varying, 'output.message.replaced'::character varying, 'turn.completed'::character varying, 'turn.failed'::character varying, 'turn.cancelled'::character varying])::text[]));

COMMENT ON INDEX public.idx_events_reporting_repair IS 'Partial ordered index for bounded reporting event projection repair';

CREATE INDEX idx_events_search_vector ON public.events USING gin (search_vector) WHERE ((event_type)::text = ANY ((ARRAY['input.message'::character varying, 'output.message.completed'::character varying, 'tool.completed'::character varying])::text[]));

CREATE INDEX idx_events_session_sequence ON public.events USING btree (session_id, sequence);

CREATE INDEX idx_events_tags ON public.events USING gin (tags);

CREATE INDEX idx_events_tool_calls ON public.events USING btree (session_id, sequence) WHERE ((event_type)::text = ANY ((ARRAY['tool.started'::character varying, 'tool.completed'::character varying])::text[]));

COMMENT ON INDEX public.idx_events_tool_calls IS 'Partial index for tool execution events with results';

CREATE UNIQUE INDEX idx_events_turn_resolution ON public.events USING btree (turn_resolution_id, turn_resolution_event_index) WHERE (turn_resolution_id IS NOT NULL);

CREATE INDEX idx_events_turns ON public.events USING btree (session_id, sequence) WHERE ((event_type)::text = ANY ((ARRAY['turn.started'::character varying, 'turn.completed'::character varying, 'turn.failed'::character varying])::text[]));

COMMENT ON INDEX public.idx_events_turns IS 'Partial index for turn lifecycle events';

CREATE INDEX idx_fact_budget_posting_budget ON public.fact_budget_posting USING btree (org_id, budget_id, created_at);

CREATE INDEX idx_fact_budget_posting_org_created ON public.fact_budget_posting USING btree (org_id, created_at);

CREATE INDEX idx_fact_budget_posting_org_day ON public.fact_budget_posting USING btree (org_id, time_bucket_day);

CREATE INDEX idx_fact_capability_usage_capability ON public.fact_capability_usage USING btree (org_id, capability_id, capability_usage_kind, created_at);

CREATE INDEX idx_fact_capability_usage_org_created ON public.fact_capability_usage USING btree (org_id, created_at);

CREATE INDEX idx_fact_capability_usage_org_day ON public.fact_capability_usage USING btree (org_id, time_bucket_day);

CREATE INDEX idx_fact_capability_usage_org_session ON public.fact_capability_usage USING btree (org_id, session_id) WHERE (capability_usage_kind = 'configured'::text);

CREATE INDEX idx_fact_llm_generation_model ON public.fact_llm_generation USING btree (org_id, model, provider, created_at);

CREATE INDEX idx_fact_llm_generation_org_created ON public.fact_llm_generation USING btree (org_id, created_at);

CREATE INDEX idx_fact_llm_generation_org_day ON public.fact_llm_generation USING btree (org_id, time_bucket_day);

CREATE INDEX idx_fact_session_org_created ON public.fact_session USING btree (org_id, created_at);

CREATE INDEX idx_fact_session_org_day ON public.fact_session USING btree (org_id, time_bucket_day);

CREATE INDEX idx_fact_session_status ON public.fact_session USING btree (org_id, session_status, created_at);

CREATE INDEX idx_fact_tool_call_org_created ON public.fact_tool_call USING btree (org_id, created_at);

CREATE INDEX idx_fact_tool_call_org_day ON public.fact_tool_call USING btree (org_id, time_bucket_day);

CREATE INDEX idx_fact_tool_call_tool ON public.fact_tool_call USING btree (org_id, tool_name, tool_status, created_at);

CREATE INDEX idx_fact_turn_org_created ON public.fact_turn USING btree (org_id, created_at);

CREATE INDEX idx_fact_turn_org_day ON public.fact_turn USING btree (org_id, time_bucket_day);

CREATE INDEX idx_fact_turn_status ON public.fact_turn USING btree (org_id, turn_status, created_at);

CREATE INDEX idx_files_org_id ON public.files USING btree (org_id);

CREATE INDEX idx_github_apps_org_id ON public.github_apps USING btree (org_id);

CREATE INDEX idx_harness_caps_harness_id ON public.harness_capabilities USING btree (harness_id);

CREATE INDEX idx_harnesses_org_id ON public.harnesses USING btree (org_id);

CREATE UNIQUE INDEX idx_harnesses_org_name ON public.harnesses USING btree (org_id, name) WHERE ((status)::text <> 'deleted'::text);

CREATE INDEX idx_harnesses_parent_harness_id ON public.harnesses USING btree (parent_harness_id);

CREATE INDEX idx_images_created_at ON public.images USING btree (created_at DESC);

CREATE INDEX idx_images_org_id ON public.images USING btree (org_id);

CREATE INDEX idx_images_session_id ON public.images USING btree (((metadata ->> 'session_id'::text))) WHERE ((metadata ->> 'session_id'::text) IS NOT NULL);

CREATE INDEX idx_knowledge_bases_org_id ON public.knowledge_bases USING btree (org_id);

CREATE UNIQUE INDEX idx_knowledge_bases_org_name_active ON public.knowledge_bases USING btree (org_id, lower((name)::text)) WHERE ((status)::text <> 'deleted'::text);

CREATE UNIQUE INDEX idx_knowledge_bases_org_public_id ON public.knowledge_bases USING btree (org_id, public_id);

CREATE INDEX idx_knowledge_bases_status ON public.knowledge_bases USING btree (status);

CREATE INDEX idx_knowledge_entries_fts ON public.knowledge_entries USING gin (to_tsvector('english'::regconfig, (((COALESCE(title, ''::character varying))::text || ' '::text) || COALESCE(body, ''::text))));

CREATE INDEX idx_knowledge_entries_kb_id ON public.knowledge_entries USING btree (kb_id);

CREATE INDEX idx_knowledge_entries_kb_kind ON public.knowledge_entries USING btree (kb_id, kind);

CREATE UNIQUE INDEX idx_knowledge_entries_kb_public_id ON public.knowledge_entries USING btree (kb_id, public_id);

CREATE INDEX idx_knowledge_index_chunks_document_id ON public.knowledge_index_chunks USING btree (document_id);

CREATE INDEX idx_knowledge_index_chunks_index_id ON public.knowledge_index_chunks USING btree (index_id);

CREATE UNIQUE INDEX idx_knowledge_index_chunks_index_public_id ON public.knowledge_index_chunks USING btree (index_id, public_id);

CREATE INDEX idx_knowledge_index_documents_index_id ON public.knowledge_index_documents USING btree (index_id);

CREATE UNIQUE INDEX idx_knowledge_index_documents_index_public_id ON public.knowledge_index_documents USING btree (index_id, public_id);

CREATE UNIQUE INDEX idx_knowledge_index_documents_index_source_uri ON public.knowledge_index_documents USING btree (index_id, source_uri);

CREATE INDEX idx_knowledge_indexes_org_id ON public.knowledge_indexes USING btree (org_id);

CREATE UNIQUE INDEX idx_knowledge_indexes_org_name_active ON public.knowledge_indexes USING btree (org_id, lower((name)::text)) WHERE ((status)::text <> 'deleted'::text);

CREATE UNIQUE INDEX idx_knowledge_indexes_org_public_id ON public.knowledge_indexes USING btree (org_id, public_id);

CREATE INDEX idx_knowledge_indexes_status ON public.knowledge_indexes USING btree (status);

CREATE INDEX idx_knowledge_indexes_sync_status ON public.knowledge_indexes USING btree (sync_status);

CREATE INDEX idx_leased_resources_cleaning_started_at ON public.leased_resources USING btree (cleanup_started_at) WHERE (status = 'cleaning'::text);

CREATE INDEX idx_leased_resources_due_cleanup ON public.leased_resources USING btree (lease_expires_at) WHERE (status = ANY (ARRAY['active'::text, 'cleanup_failed'::text]));

CREATE INDEX idx_leased_resources_owner_provider ON public.leased_resources USING btree (owner_user_id, provider);

CREATE INDEX idx_leased_resources_session_created_at ON public.leased_resources USING btree (session_id, created_at DESC);

CREATE INDEX idx_llm_generations_created_at ON public.llm_generations USING btree (created_at);

CREATE UNIQUE INDEX idx_llm_generations_event_id ON public.llm_generations USING btree (event_id) WHERE (event_id IS NOT NULL);

CREATE INDEX idx_llm_generations_org_created ON public.llm_generations USING btree (org_id, created_at);

CREATE INDEX idx_llm_generations_org_id ON public.llm_generations USING btree (org_id);

CREATE INDEX idx_llm_generations_org_sessionless ON public.llm_generations USING btree (org_id, created_at) WHERE (session_id IS NULL);

CREATE INDEX idx_llm_generations_provider_response_id ON public.llm_generations USING btree (provider, provider_response_id) WHERE ((provider_response_id IS NOT NULL) AND (reconciled_at IS NULL));

CREATE INDEX idx_llm_generations_reconcile_ready ON public.llm_generations USING btree (provider, reconcile_after, created_at) WHERE ((provider_response_id IS NOT NULL) AND (reconciled_at IS NULL));

CREATE INDEX idx_llm_generations_session_id ON public.llm_generations USING btree (session_id);

CREATE INDEX idx_llm_generations_turn_id ON public.llm_generations USING btree (turn_id) WHERE (turn_id IS NOT NULL);

CREATE INDEX idx_llm_generations_usage_pending ON public.llm_generations USING btree (reconcile_after, created_at) WHERE usage_pending;

CREATE INDEX idx_mcp_event_subscriptions_org_event ON public.mcp_event_subscriptions USING btree (org_id, event_name, expires_at);

CREATE INDEX idx_mcp_servers_org_id ON public.mcp_servers USING btree (org_id);

CREATE UNIQUE INDEX idx_mcp_servers_org_name_live ON public.mcp_servers USING btree (org_id, name) WHERE (((status)::text = ANY ((ARRAY['active'::character varying, 'disabled'::character varying])::text[])) AND (owner_virtual_user_id IS NULL));

CREATE UNIQUE INDEX idx_mcp_servers_owner_name_live ON public.mcp_servers USING btree (org_id, owner_virtual_user_id, name) WHERE (((status)::text = ANY ((ARRAY['active'::character varying, 'disabled'::character varying])::text[])) AND (owner_virtual_user_id IS NOT NULL));

CREATE INDEX idx_mcp_servers_status ON public.mcp_servers USING btree (status);

CREATE INDEX idx_mcp_service_tool_caches_freshness ON public.mcp_service_tool_caches USING btree (tools_cached_at);

CREATE INDEX idx_mcp_tool_labels_org_server ON public.mcp_tool_labels USING btree (org_id, mcp_server_id);

CREATE UNIQUE INDEX idx_memories_org_agent_scope_owner_active ON public.memories USING btree (org_id, owner_agent_id) WHERE (((scope)::text = 'agent'::text) AND ((status)::text <> 'deleted'::text));

CREATE INDEX idx_memories_org_id ON public.memories USING btree (org_id);

CREATE UNIQUE INDEX idx_memories_org_lower_name_active ON public.memories USING btree (org_id, lower((name)::text)) WHERE ((status)::text <> 'deleted'::text);

CREATE UNIQUE INDEX idx_memories_org_name_active ON public.memories USING btree (org_id, name) WHERE ((status)::text <> 'deleted'::text);

CREATE UNIQUE INDEX idx_memories_org_public_id ON public.memories USING btree (org_id, public_id);

CREATE INDEX idx_memories_org_scope_status ON public.memories USING btree (org_id, scope, status);

CREATE UNIQUE INDEX idx_memories_org_user_scope_owner_active ON public.memories USING btree (org_id, owner_user_id) WHERE (((scope)::text = 'user'::text) AND ((status)::text <> 'deleted'::text));

CREATE UNIQUE INDEX idx_memories_public_id ON public.memories USING btree (public_id);

CREATE INDEX idx_memories_source_type ON public.memories USING btree (org_id, source_type);

CREATE INDEX idx_memories_status ON public.memories USING btree (status);

CREATE INDEX idx_memories_sync_status ON public.memories USING btree (org_id, sync_status) WHERE ((source_type)::text <> 'manual'::text);

CREATE INDEX idx_memory_files_memory_id ON public.memory_files USING btree (memory_id);

CREATE INDEX idx_memory_files_name ON public.memory_files USING btree (memory_id, "substring"(path, '[^/]+$'::text));

CREATE INDEX idx_memory_files_parent ON public.memory_files USING btree (memory_id, "substring"(path, '^(.*)/[^/]+$'::text));

CREATE UNIQUE INDEX idx_memory_files_path ON public.memory_files USING btree (memory_id, path);

CREATE INDEX idx_message_feedback_org_session ON public.message_feedback USING btree (org_id, session_id);

CREATE INDEX idx_model_router_candidates_model_id ON public.model_router_candidates USING btree (model_id);

CREATE INDEX idx_model_router_candidates_route_id ON public.model_router_candidates USING btree (route_id);

CREATE INDEX idx_model_router_candidates_route_position ON public.model_router_candidates USING btree (route_id, "position");

CREATE INDEX idx_model_router_routes_router_id ON public.model_router_routes USING btree (router_id);

CREATE UNIQUE INDEX idx_model_router_routes_router_key ON public.model_router_routes USING btree (router_id, key);

CREATE INDEX idx_model_router_routes_router_position ON public.model_router_routes USING btree (router_id, "position");

CREATE INDEX idx_model_routers_org_id ON public.model_routers USING btree (org_id);

CREATE UNIQUE INDEX idx_model_routers_org_name_active ON public.model_routers USING btree (org_id, name) WHERE ((status)::text <> 'deleted'::text);

CREATE UNIQUE INDEX idx_model_routers_org_public_id ON public.model_routers USING btree (org_id, public_id);

CREATE INDEX idx_model_routers_status ON public.model_routers USING btree (status);

CREATE INDEX idx_models_enabled ON public.models USING btree (enabled) WHERE (enabled = true);

CREATE INDEX idx_models_favorite ON public.models USING btree (is_favorite) WHERE (is_favorite = true);

CREATE INDEX idx_models_last_seen ON public.models USING btree (last_seen_at) WHERE (last_seen_at IS NOT NULL);

CREATE INDEX idx_models_org_id ON public.models USING btree (org_id);

CREATE INDEX idx_models_provider_id ON public.models USING btree (provider_id);

CREATE UNIQUE INDEX idx_models_provider_model ON public.models USING btree (provider_id, model_id);

CREATE INDEX idx_models_source ON public.models USING btree (source);

CREATE INDEX idx_notification_turn_requests_session ON public.notification_turn_requests USING btree (session_id, created_at DESC);

CREATE INDEX idx_notification_turn_requests_user ON public.notification_turn_requests USING btree (user_id, created_at DESC);

CREATE UNIQUE INDEX idx_notifications_active_dedupe ON public.notifications USING btree (org_id, user_id, dedupe_key) WHERE ((dedupe_key IS NOT NULL) AND (viewed_at IS NULL));

CREATE INDEX idx_notifications_unviewed ON public.notifications USING btree (user_id, org_id, viewed_at, created_at DESC);

CREATE INDEX idx_notifications_user_created ON public.notifications USING btree (user_id, org_id, created_at DESC);

CREATE INDEX idx_notifications_user_updated ON public.notifications USING btree (user_id, org_id, updated_at DESC);

CREATE INDEX idx_oauth_authorization_codes_code_hash ON public.oauth_authorization_codes USING btree (code_hash);

CREATE INDEX idx_oauth_authorization_codes_expires_at ON public.oauth_authorization_codes USING btree (expires_at);

CREATE INDEX idx_oauth_clients_client_id ON public.oauth_clients USING btree (client_id);

CREATE INDEX idx_oauth_grants_user_id ON public.oauth_grants USING btree (user_id);

CREATE INDEX idx_oauth_refresh_tokens_client_user ON public.oauth_refresh_tokens USING btree (client_id, user_id);

CREATE INDEX idx_oauth_refresh_tokens_expires_at ON public.oauth_refresh_tokens USING btree (expires_at);

CREATE INDEX idx_oauth_refresh_tokens_grant_id ON public.oauth_refresh_tokens USING btree (grant_id);

CREATE INDEX idx_oauth_refresh_tokens_token_hash ON public.oauth_refresh_tokens USING btree (token_hash);

CREATE INDEX idx_observers_org_active ON public.observers USING btree (org_id) WHERE ((status)::text = 'active'::text);

CREATE INDEX idx_observers_org_id ON public.observers USING btree (org_id);

CREATE INDEX idx_observers_public_id ON public.observers USING btree (public_id);

CREATE INDEX idx_org_feature_flags_org_id ON public.org_feature_flags USING btree (org_id);

CREATE INDEX idx_org_invitations_actionable_email_created_at ON public.org_invitations USING btree (email, created_at DESC) WHERE ((accepted_at IS NULL) AND (revoked_at IS NULL));

CREATE INDEX idx_org_invitations_org_id ON public.org_invitations USING btree (org_id);

CREATE UNIQUE INDEX idx_org_invitations_pending_email ON public.org_invitations USING btree (org_id, email) WHERE ((accepted_at IS NULL) AND (revoked_at IS NULL));

CREATE UNIQUE INDEX idx_org_invitations_token_hash ON public.org_invitations USING btree (token_hash);

CREATE INDEX idx_org_slack_connections_rotation ON public.org_slack_connections USING btree (access_token_expires_at) WHERE (state = 'connected'::text);

CREATE INDEX idx_organization_members_role ON public.organization_members USING btree (org_id, role);

CREATE INDEX idx_organization_members_user_id ON public.organization_members USING btree (user_id);

CREATE INDEX idx_organizations_external_id ON public.organizations USING btree (external_id) WHERE (external_id IS NOT NULL);

CREATE INDEX idx_organizations_public_id ON public.organizations USING btree (public_id);

CREATE UNIQUE INDEX idx_password_reset_tokens_token_hash ON public.password_reset_tokens USING btree (token_hash);

CREATE INDEX idx_password_reset_tokens_user_id ON public.password_reset_tokens USING btree (user_id);

CREATE INDEX idx_payment_accounts_org_owner ON public.payment_accounts USING btree (org_id, owner_type, owner_id);

CREATE INDEX idx_payment_accounts_org_status ON public.payment_accounts USING btree (org_id, status);

CREATE INDEX idx_payment_attempts_org_created ON public.payment_attempts USING btree (org_id, created_at DESC);

CREATE INDEX idx_payment_attempts_policy_created ON public.payment_attempts USING btree (payment_policy_id, created_at DESC) WHERE (payment_policy_id IS NOT NULL);

CREATE INDEX idx_payment_attempts_session_created ON public.payment_attempts USING btree (session_id, created_at DESC) WHERE (session_id IS NOT NULL);

CREATE INDEX idx_payment_policies_account ON public.payment_policies USING btree (payment_account_id);

CREATE INDEX idx_payment_policies_org_subject ON public.payment_policies USING btree (org_id, subject_type, subject_id);

CREATE INDEX idx_personal_access_tokens_expires_at ON public.personal_access_tokens USING btree (expires_at) WHERE (expires_at IS NOT NULL);

CREATE UNIQUE INDEX idx_personal_access_tokens_hash ON public.personal_access_tokens USING btree (token_hash);

CREATE INDEX idx_personal_access_tokens_user_id ON public.personal_access_tokens USING btree (user_id);

CREATE INDEX idx_pinned_sessions_user_org ON public.pinned_sessions USING btree (user_id, org_id);

CREATE INDEX idx_plugin_installs_marketplace_id ON public.plugin_installs USING btree (marketplace_id);

CREATE INDEX idx_plugin_installs_org_id ON public.plugin_installs USING btree (org_id);

CREATE INDEX idx_plugin_installs_status ON public.plugin_installs USING btree (status);

CREATE INDEX idx_plugin_marketplaces_org_id ON public.plugin_marketplaces USING btree (org_id);

CREATE INDEX idx_plugin_marketplaces_status ON public.plugin_marketplaces USING btree (status);

CREATE INDEX idx_principals_kind_status ON public.principals USING btree (org_id, kind, status);

CREATE UNIQUE INDEX idx_principals_org_kind_subject ON public.principals USING btree (org_id, kind, subject_id);

CREATE INDEX idx_principals_parent ON public.principals USING btree (parent_principal_id);

CREATE INDEX idx_principals_resolved_user ON public.principals USING btree (org_id, resolved_user_id);

CREATE INDEX idx_providers_org_id ON public.providers USING btree (org_id);

CREATE INDEX idx_providers_provider_type ON public.providers USING btree (provider_type);

CREATE INDEX idx_providers_status ON public.providers USING btree (status);

CREATE INDEX idx_refresh_tokens_expires_at ON public.refresh_tokens USING btree (expires_at);

CREATE UNIQUE INDEX idx_refresh_tokens_hash ON public.refresh_tokens USING btree (token_hash);

CREATE INDEX idx_refresh_tokens_user_id ON public.refresh_tokens USING btree (user_id);

CREATE INDEX idx_reporting_outbox_org_status ON public.reporting_outbox USING btree (org_id, status, next_attempt_at);

CREATE INDEX idx_reporting_outbox_poll ON public.reporting_outbox USING btree (status, next_attempt_at, org_id, created_at);

CREATE INDEX idx_reporting_saved_reports_org_updated ON public.reporting_saved_reports USING btree (org_id, updated_at DESC);

CREATE INDEX idx_sandbox_checkpoints_sandbox_created ON public.sandbox_checkpoints USING btree (sandbox_id, created_at DESC);

CREATE INDEX idx_sandbox_checkpoints_source_tool_call ON public.sandbox_checkpoints USING btree (source_turn_id, source_tool_call_id) WHERE (source_tool_call_id IS NOT NULL);

CREATE INDEX idx_sandbox_checkpoints_unattached ON public.sandbox_checkpoints USING btree (sandbox_id, created_at) WHERE (attached_at IS NULL);

CREATE INDEX idx_sandbox_instances_external ON public.sandbox_instances USING btree (external_id) WHERE (retired_at IS NULL);

CREATE INDEX idx_sandbox_orphan_sightings_last_seen ON public.sandbox_orphan_sightings USING btree (last_seen_at);

CREATE INDEX idx_sandbox_state_transitions_org_at ON public.sandbox_state_transitions USING btree (org_id, at);

CREATE INDEX idx_sandbox_state_transitions_sandbox_at ON public.sandbox_state_transitions USING btree (sandbox_id, at);

CREATE INDEX idx_sandboxes_org_updated ON public.sandboxes USING btree (org_id, updated_at DESC);

CREATE INDEX idx_session_actors_leased_org ON public.session_actors USING btree (org_id) WHERE (lease_holder IS NOT NULL);

CREATE INDEX idx_session_actors_runnable ON public.session_actors USING btree (runnable_since, session_id) WHERE (runnable_since IS NOT NULL);

CREATE INDEX idx_session_compaction_checkpoints_session ON public.session_compaction_checkpoints USING btree (session_id, source_sequence DESC);

CREATE INDEX idx_session_database_pages_db ON public.session_database_pages USING btree (database_id);

CREATE INDEX idx_session_databases_session_id ON public.session_databases USING btree (session_id);

CREATE INDEX idx_session_git_objects_session ON public.session_git_objects USING btree (session_id);

CREATE INDEX idx_session_git_refs_session ON public.session_git_refs USING btree (session_id);

CREATE INDEX idx_session_key_values_key ON public.session_key_values USING btree (session_id, key);

CREATE INDEX idx_session_key_values_session_id ON public.session_key_values USING btree (session_id);

CREATE INDEX idx_session_memory_mounts_memory_id ON public.session_memory_mounts USING btree (memory_id);

CREATE UNIQUE INDEX idx_session_memory_mounts_path ON public.session_memory_mounts USING btree (session_id, mount_path);

CREATE INDEX idx_session_memory_mounts_session_id ON public.session_memory_mounts USING btree (session_id);

CREATE INDEX idx_session_participants_agent ON public.session_participants USING btree (org_id, agent_id) WHERE (agent_id IS NOT NULL);

CREATE INDEX idx_session_participants_org_session ON public.session_participants USING btree (org_id, session_id, joined_at);

CREATE INDEX idx_session_participants_principal ON public.session_participants USING btree (org_id, principal_id);

CREATE INDEX idx_session_participants_session_left_at ON public.session_participants USING btree (session_id, left_at);

CREATE INDEX idx_session_resources_session_status ON public.session_resources USING btree (session_id, status);

CREATE INDEX idx_session_schedules_active_count ON public.session_schedules USING btree (session_id) WHERE (enabled = true);

CREATE UNIQUE INDEX idx_session_schedules_org_public_id ON public.session_schedules USING btree (org_id, public_id);

CREATE INDEX idx_session_schedules_owner_principal ON public.session_schedules USING btree (owner_principal_id);

CREATE INDEX idx_session_schedules_polling ON public.session_schedules USING btree (next_trigger_at) WHERE ((enabled = true) AND (next_trigger_at IS NOT NULL));

CREATE INDEX idx_session_schedules_resolved_owner ON public.session_schedules USING btree (org_id, resolved_owner_user_id);

CREATE INDEX idx_session_schedules_session ON public.session_schedules USING btree (session_id, created_at DESC);

CREATE INDEX idx_session_secrets_name ON public.session_secrets USING btree (session_id, name);

CREATE INDEX idx_session_secrets_session_id ON public.session_secrets USING btree (session_id);

CREATE INDEX idx_session_trace_steps_errors ON public.session_trace_steps USING btree (session_id, turn_no, step_no) WHERE (status = 'error'::text);

CREATE INDEX idx_session_trace_steps_tool_call ON public.session_trace_steps USING btree (session_id, tool_call_id) WHERE (tool_call_id IS NOT NULL);

CREATE INDEX idx_session_trace_turns_errors ON public.session_trace_turns USING btree (session_id, turn_no) WHERE (error_count > 0);

CREATE INDEX idx_session_trace_turns_start ON public.session_trace_turns USING btree (session_id, start_sequence);

CREATE UNIQUE INDEX idx_session_trace_turns_turn_id ON public.session_trace_turns USING btree (session_id, turn_id);

CREATE INDEX idx_sessions_agent_id ON public.sessions USING btree (agent_id);

CREATE INDEX idx_sessions_agent_identity_id ON public.sessions USING btree (virtual_user_id);

CREATE INDEX idx_sessions_app_id ON public.sessions USING btree (app_id);

CREATE INDEX idx_sessions_channel_id ON public.sessions USING btree (channel_id) WHERE (channel_id IS NOT NULL);

CREATE INDEX idx_sessions_created_at ON public.sessions USING btree (created_at DESC);

CREATE INDEX idx_sessions_forked_from_session_id ON public.sessions USING btree (forked_from_session_id) WHERE (forked_from_session_id IS NOT NULL);

CREATE INDEX idx_sessions_org_active_updated_at ON public.sessions USING btree (org_id, updated_at DESC) WHERE (archived_at IS NULL);

CREATE INDEX idx_sessions_org_agent_id ON public.sessions USING btree (org_id, agent_id);

CREATE INDEX idx_sessions_org_agent_identity_id ON public.sessions USING btree (org_id, virtual_user_id);

CREATE INDEX idx_sessions_org_created_at ON public.sessions USING btree (org_id, created_at DESC);

CREATE INDEX idx_sessions_org_harness_id ON public.sessions USING btree (org_id, harness_id);

CREATE INDEX idx_sessions_org_id ON public.sessions USING btree (org_id);

CREATE INDEX idx_sessions_org_owner_created_at ON public.sessions USING btree (org_id, resolved_owner_user_id, created_at DESC);

CREATE INDEX idx_sessions_org_source_created_at ON public.sessions USING btree (org_id, source, created_at DESC);

CREATE INDEX idx_sessions_org_source_owner_updated_at ON public.sessions USING btree (org_id, source, resolved_owner_user_id, updated_at DESC);

CREATE INDEX idx_sessions_org_status ON public.sessions USING btree (org_id, status);

CREATE INDEX idx_sessions_org_status_last_turn ON public.sessions USING btree (org_id, status, last_turn_status);

CREATE INDEX idx_sessions_org_updated_at ON public.sessions USING btree (org_id, updated_at DESC);

CREATE INDEX idx_sessions_owner_principal ON public.sessions USING btree (owner_principal_id);

CREATE INDEX idx_sessions_parent_session_id ON public.sessions USING btree (parent_session_id) WHERE (parent_session_id IS NOT NULL);

CREATE UNIQUE INDEX idx_sessions_platform_chat_starter_owner ON public.sessions USING btree (org_id, owner_principal_id) WHERE (tags @> ARRAY['platform-chat-starter'::text]);

CREATE INDEX idx_sessions_resolved_owner ON public.sessions USING btree (org_id, resolved_owner_user_id);

CREATE INDEX idx_sessions_status ON public.sessions USING btree (status);

CREATE INDEX idx_sessions_tags ON public.sessions USING gin (tags);

CREATE INDEX idx_sessions_trigger_id ON public.sessions USING btree (trigger_id) WHERE (trigger_id IS NOT NULL);

CREATE INDEX idx_sessions_workspace_id ON public.sessions USING btree (workspace_id);

CREATE INDEX idx_skill_files_skill_id ON public.skill_files USING btree (skill_id);

CREATE INDEX idx_skills_org_id ON public.skills USING btree (org_id);

CREATE UNIQUE INDEX idx_skills_org_name ON public.skills USING btree (org_id, name);

CREATE UNIQUE INDEX idx_skills_org_public_id ON public.skills USING btree (org_id, public_id);

CREATE INDEX idx_skills_status ON public.skills USING btree (status);

CREATE INDEX idx_spawn_handles_child ON public.subagent_spawn_handles USING btree (child_session_id) WHERE (child_session_id IS NOT NULL);

CREATE INDEX idx_spawn_handles_parent ON public.subagent_spawn_handles USING btree (parent_session_id);

CREATE INDEX idx_trace_scores_observer_created ON public.trace_scores USING btree (observer_id, created_at DESC);

CREATE INDEX idx_trace_scores_queue ON public.trace_scores USING btree (created_at) WHERE ((status)::text = ANY ((ARRAY['pending'::character varying, 'scoring'::character varying])::text[]));

CREATE INDEX idx_trace_scores_session_id ON public.trace_scores USING btree (session_id);

CREATE INDEX idx_usage_journal_kind_created ON public.usage_journal USING btree (kind, created_at DESC);

CREATE UNIQUE INDEX idx_usage_journal_kind_event ON public.usage_journal USING btree (kind, event_id) WHERE (event_id IS NOT NULL);

CREATE INDEX idx_usage_journal_org_created ON public.usage_journal USING btree (org_id, created_at DESC);

CREATE INDEX idx_usage_journal_session_created ON public.usage_journal USING btree (session_id, created_at DESC) WHERE (session_id IS NOT NULL);

CREATE UNIQUE INDEX idx_usage_journal_source ON public.usage_journal USING btree (source_type, source_id) WHERE ((source_type IS NOT NULL) AND (source_id IS NOT NULL));

CREATE INDEX idx_usage_ledger_budget_created ON public.usage_ledger USING btree (budget_id, created_at DESC) WHERE (budget_id IS NOT NULL);

CREATE UNIQUE INDEX idx_usage_ledger_journal_budget_currency ON public.usage_ledger USING btree (journal_id, budget_id, currency);

CREATE INDEX idx_usage_ledger_journal_created ON public.usage_ledger USING btree (journal_id, created_at DESC);

CREATE INDEX idx_usage_ledger_org_created ON public.usage_ledger USING btree (org_id, created_at DESC);

CREATE INDEX idx_usage_ledger_session_created ON public.usage_ledger USING btree (session_id, created_at DESC) WHERE (session_id IS NOT NULL);

CREATE INDEX idx_user_connections_installation ON public.pending_user_connections USING btree (installation_id) WHERE (installation_id IS NOT NULL);

CREATE INDEX idx_user_connections_provider ON public.pending_user_connections USING btree (user_id, provider);

CREATE INDEX idx_user_connections_user_id ON public.pending_user_connections USING btree (user_id);

CREATE INDEX idx_user_preferences_user_id ON public.user_preferences USING btree (user_id);

CREATE INDEX idx_users_auth_provider ON public.users USING btree (auth_provider, auth_provider_id) WHERE (auth_provider IS NOT NULL);

CREATE UNIQUE INDEX idx_users_email ON public.users USING btree (lower(email));

CREATE INDEX idx_users_external_id ON public.users USING btree (external_id) WHERE (external_id IS NOT NULL);

CREATE INDEX idx_workspace_files_name ON public.workspace_files USING btree (workspace_id, "substring"(path, '[^/]+$'::text));

CREATE INDEX idx_workspace_files_parent ON public.workspace_files USING btree (workspace_id, "substring"(path, '^(.*)/[^/]+$'::text));

CREATE UNIQUE INDEX idx_workspace_files_path ON public.workspace_files USING btree (workspace_id, path);

CREATE INDEX idx_workspace_files_workspace_id ON public.workspace_files USING btree (workspace_id);

CREATE INDEX idx_workspaces_org_id ON public.workspaces USING btree (org_id);

CREATE UNIQUE INDEX idx_workspaces_org_lower_name_active ON public.workspaces USING btree (org_id, lower((name)::text)) WHERE ((status)::text <> 'deleted'::text);

CREATE UNIQUE INDEX idx_workspaces_org_name_active ON public.workspaces USING btree (org_id, name) WHERE ((status)::text <> 'deleted'::text);

CREATE UNIQUE INDEX idx_workspaces_org_public_id ON public.workspaces USING btree (org_id, public_id);

CREATE UNIQUE INDEX idx_workspaces_public_id ON public.workspaces USING btree (public_id);

CREATE INDEX idx_workspaces_status ON public.workspaces USING btree (status);

CREATE UNIQUE INDEX notifications_health_episode ON public.notifications USING btree (org_id, user_id, dedupe_key) WHERE ((kind)::text = 'health.issue'::text);

CREATE INDEX organization_task_webhooks_org_id_idx ON public.organization_task_webhooks USING btree (org_id);

CREATE INDEX pact_device_authorizations_expiry ON public.pact_device_authorizations USING btree (expires_at);

CREATE INDEX pact_device_authorizations_user_code ON public.pact_device_authorizations USING btree (channel_id, user_code);

CREATE INDEX pact_grants_user ON public.pact_grants USING btree (channel_id, client_id, brand_user_id);

CREATE INDEX pact_refresh_tokens_grant ON public.pact_refresh_tokens USING btree (grant_id);

CREATE INDEX pact_used_assertions_expiry ON public.pact_used_assertions USING btree (expires_at);

CREATE INDEX poppy_conversations_owner ON public.poppy_conversations USING btree (channel_id, client_id, user_id);

CREATE INDEX poppy_sessions_user ON public.poppy_sessions USING btree (channel_id, client_id, user_id);

CREATE UNIQUE INDEX sandboxes_one_primary_per_session ON public.sandboxes USING btree (session_id) WHERE (role = 'primary'::text);

CREATE UNIQUE INDEX sandboxes_one_profiled_environment_per_session ON public.sandboxes USING btree (session_id) WHERE (profile_snapshot IS NOT NULL);

CREATE UNIQUE INDEX session_participants_one_active_agent_member_idx ON public.session_participants USING btree (session_id, agent_id) WHERE ((kind = 'agent'::text) AND (role = 'member'::text) AND (left_at IS NULL));

CREATE UNIQUE INDEX session_participants_one_active_host_idx ON public.session_participants USING btree (session_id) WHERE ((kind = 'agent'::text) AND (role = 'host'::text) AND (left_at IS NULL));

CREATE UNIQUE INDEX session_participants_one_active_user_principal_idx ON public.session_participants USING btree (session_id, principal_id) WHERE ((kind = 'user'::text) AND (left_at IS NULL));

CREATE INDEX session_task_messages_cursor_idx ON public.session_task_messages USING btree (task_id, created_at, id);

CREATE INDEX session_task_messages_task_idx ON public.session_task_messages USING btree (task_id, created_at);

CREATE INDEX session_task_push_configs_session_task_idx ON public.session_task_push_configs USING btree (session_id, task_id);

CREATE INDEX session_tasks_created_at_idx ON public.session_tasks USING btree (created_at DESC, id DESC);

CREATE INDEX session_tasks_root_session_id_idx ON public.session_tasks USING btree (root_session_id);

CREATE INDEX session_tasks_running_monitors_idx ON public.session_tasks USING btree (((spec ->> 'schedule_id'::text))) WHERE ((kind = 'monitor'::text) AND (state = 'running'::text));

CREATE INDEX session_tasks_session_state_idx ON public.session_tasks USING btree (session_id, state);

CREATE INDEX session_tasks_terminal_finished_at_idx ON public.session_tasks USING btree (finished_at) WHERE (state = ANY (ARRAY['succeeded'::text, 'failed'::text, 'canceled'::text]));

CREATE INDEX sessions_root_session_id_idx ON public.sessions USING btree (root_session_id);

CREATE UNIQUE INDEX virtual_user_default_binding ON public.virtual_user_bindings USING btree (org_id, management_user_id) WHERE (management_user_id IS NOT NULL);

CREATE TRIGGER agent_service_virtual_user_guard BEFORE INSERT OR UPDATE OF virtual_user_id ON public.agents FOR EACH ROW EXECUTE FUNCTION public.enforce_agent_service_virtual_user();

CREATE TRIGGER agents_api_sessions_tombstone_on_delete AFTER DELETE ON public.agents_api_sessions FOR EACH ROW WHEN ((old.provider_session_id IS NOT NULL)) EXECUTE FUNCTION public.agents_api_tombstone_provider_session();

CREATE TRIGGER agents_api_sessions_tombstone_on_replace AFTER UPDATE OF provider_session_id ON public.agents_api_sessions FOR EACH ROW WHEN (((old.provider_session_id IS NOT NULL) AND (old.provider_session_id IS DISTINCT FROM new.provider_session_id))) EXECUTE FUNCTION public.agents_api_tombstone_provider_session();

CREATE TRIGGER channel_virtual_user_org_guard BEFORE INSERT OR UPDATE OF virtual_user_id, agent_id ON public.agent_channels FOR EACH ROW EXECUTE FUNCTION public.enforce_channel_virtual_user_org();

CREATE TRIGGER durable_task_stat_counters_delete AFTER DELETE ON public.durable_task_queue REFERENCING OLD TABLE AS old_rows FOR EACH STATEMENT EXECUTE FUNCTION public.durable_task_stat_counters();

CREATE TRIGGER durable_task_stat_counters_insert AFTER INSERT ON public.durable_task_queue REFERENCING NEW TABLE AS new_rows FOR EACH STATEMENT EXECUTE FUNCTION public.durable_task_stat_counters();

CREATE TRIGGER durable_task_stat_counters_update AFTER UPDATE ON public.durable_task_queue REFERENCING OLD TABLE AS old_rows NEW TABLE AS new_rows FOR EACH STATEMENT EXECUTE FUNCTION public.durable_task_stat_counters();

CREATE TRIGGER durable_workflow_stat_counters_delete AFTER DELETE ON public.durable_workflow_instances REFERENCING OLD TABLE AS old_rows FOR EACH STATEMENT EXECUTE FUNCTION public.durable_workflow_stat_counters();

CREATE TRIGGER durable_workflow_stat_counters_insert AFTER INSERT ON public.durable_workflow_instances REFERENCING NEW TABLE AS new_rows FOR EACH STATEMENT EXECUTE FUNCTION public.durable_workflow_stat_counters();

CREATE TRIGGER durable_workflow_stat_counters_update AFTER UPDATE ON public.durable_workflow_instances REFERENCING OLD TABLE AS old_rows NEW TABLE AS new_rows FOR EACH STATEMENT EXECUTE FUNCTION public.durable_workflow_stat_counters();

CREATE TRIGGER event_insert_notify AFTER INSERT ON public.events FOR EACH ROW EXECUTE FUNCTION public.notify_event_available();

CREATE TRIGGER event_sequences_removed_count_delete AFTER DELETE ON public.events REFERENCING OLD TABLE AS old_rows FOR EACH STATEMENT EXECUTE FUNCTION public.event_sequences_removed_count_delete();

CREATE TRIGGER events_append_only_delete BEFORE DELETE ON public.events FOR EACH ROW EXECUTE FUNCTION public.prevent_event_mutation();

CREATE TRIGGER events_append_only_update BEFORE UPDATE ON public.events FOR EACH ROW EXECUTE FUNCTION public.prevent_event_mutation();

CREATE TRIGGER health_issue_deleted BEFORE DELETE ON public.health_issues FOR EACH ROW EXECUTE FUNCTION public.remove_health_issue_notifications();

CREATE TRIGGER keep_platform_chat_starter_tag BEFORE UPDATE OF tags ON public.sessions FOR EACH ROW EXECUTE FUNCTION public.keep_platform_chat_starter_tag();

CREATE TRIGGER mark_platform_chat_starter_tag BEFORE INSERT ON public.sessions FOR EACH ROW WHEN ((((new.title)::text = ANY ((ARRAY['Chat'::character varying, 'Platform Chat'::character varying])::text[])) AND (new.source = 'chat'::text) AND ('chat'::text = ANY (new.tags)) AND (new.parent_session_id IS NULL) AND (new.forked_from_session_id IS NULL))) EXECUTE FUNCTION public.mark_platform_chat_starter_tag();

CREATE TRIGGER notifications_change_notify AFTER INSERT OR UPDATE ON public.notifications FOR EACH ROW EXECUTE FUNCTION public.notify_notification_available();

CREATE TRIGGER prevent_budget_ledger_mutation BEFORE DELETE OR UPDATE ON public.budget_ledger FOR EACH ROW EXECUTE FUNCTION public.prevent_event_mutation();

CREATE TRIGGER prevent_usage_journal_mutation BEFORE DELETE OR UPDATE ON public.usage_journal FOR EACH ROW EXECUTE FUNCTION public.prevent_usage_record_mutation();

CREATE TRIGGER prevent_usage_ledger_mutation BEFORE DELETE OR UPDATE ON public.usage_ledger FOR EACH ROW EXECUTE FUNCTION public.prevent_usage_record_mutation();

CREATE TRIGGER sandboxes_record_transition_insert AFTER INSERT ON public.sandboxes FOR EACH ROW EXECUTE FUNCTION public.sandboxes_record_transition();

CREATE TRIGGER sandboxes_record_transition_update AFTER UPDATE OF observed_state, generation ON public.sandboxes FOR EACH ROW WHEN (((old.observed_state IS DISTINCT FROM new.observed_state) OR (old.generation IS DISTINCT FROM new.generation))) EXECUTE FUNCTION public.sandboxes_record_transition();

CREATE TRIGGER sandboxes_stamp_deleted_at BEFORE INSERT OR UPDATE OF observed_state ON public.sandboxes FOR EACH ROW EXECUTE FUNCTION public.sandboxes_stamp_deleted_at();

CREATE TRIGGER session_tasks_count_delete AFTER DELETE ON public.session_tasks REFERENCING OLD TABLE AS old_rows FOR EACH STATEMENT EXECUTE FUNCTION public.sessions_task_count_delete();

CREATE TRIGGER session_tasks_count_insert AFTER INSERT ON public.session_tasks REFERENCING NEW TABLE AS new_rows FOR EACH STATEMENT EXECUTE FUNCTION public.sessions_task_count_insert();

CREATE TRIGGER sessions_last_turn_delete AFTER DELETE ON public.events REFERENCING OLD TABLE AS old_rows FOR EACH STATEMENT EXECUTE FUNCTION public.sessions_last_turn_delete();

CREATE TRIGGER sessions_reporting_event_counts_delete AFTER DELETE ON public.events REFERENCING OLD TABLE AS old_rows FOR EACH STATEMENT EXECUTE FUNCTION public.sessions_reporting_event_counts_delete();

CREATE TRIGGER sessions_turn_finished AFTER INSERT ON public.events FOR EACH ROW WHEN (((new.event_type)::text = ANY ((ARRAY['turn.completed'::character varying, 'turn.failed'::character varying, 'turn.cancelled'::character varying])::text[]))) EXECUTE FUNCTION public.sessions_turn_finished();

CREATE TRIGGER set_budgets_updated_at BEFORE UPDATE ON public.budgets FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER sync_user_oauth_identity_after_write AFTER INSERT OR UPDATE OF auth_provider, auth_provider_id ON public.users FOR EACH ROW EXECUTE FUNCTION public.sync_user_oauth_identity();

CREATE TRIGGER task_enqueue_notify AFTER INSERT ON public.durable_task_queue FOR EACH ROW WHEN ((new.status = 'pending'::text)) EXECUTE FUNCTION public.notify_task_available();

CREATE TRIGGER task_pending_notify AFTER UPDATE ON public.durable_task_queue FOR EACH ROW WHEN (((old.status <> 'pending'::text) AND (new.status = 'pending'::text))) EXECUTE FUNCTION public.notify_task_available();

CREATE TRIGGER trigger_durable_schedules_updated_at BEFORE UPDATE ON public.durable_schedules FOR EACH ROW EXECUTE FUNCTION public.update_durable_updated_at();

CREATE TRIGGER trigger_durable_workflow_instances_updated_at BEFORE UPDATE ON public.durable_workflow_instances FOR EACH ROW EXECUTE FUNCTION public.update_durable_updated_at();

CREATE TRIGGER update_agent_channels_updated_at BEFORE UPDATE ON public.agent_channels FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_agent_check_rules_updated_at BEFORE UPDATE ON public.agent_check_rules FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_agent_health_check_runs_updated_at BEFORE UPDATE ON public.agent_health_check_runs FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_agent_identities_updated_at BEFORE UPDATE ON public.virtual_users FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_agent_identity_connections_updated_at BEFORE UPDATE ON public.virtual_user_connections FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_agent_keys_updated_at BEFORE UPDATE ON public.agent_keys FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_agent_mcp_secret_bindings_updated_at BEFORE UPDATE ON public.agent_mcp_secret_bindings FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_agent_scripts_updated_at BEFORE UPDATE ON public.agent_scripts FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_agent_trigger_mcp_subscriptions_updated_at BEFORE UPDATE ON public.agent_trigger_mcp_subscriptions FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_agent_triggers_updated_at BEFORE UPDATE ON public.agent_triggers FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_agents_updated_at BEFORE UPDATE ON public.agents FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_apps_updated_at BEFORE UPDATE ON public.apps FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_declarative_capabilities_updated_at BEFORE UPDATE ON public.declarative_capabilities FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_eval_case_results_updated_at BEFORE UPDATE ON public.eval_case_results FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_eval_cases_updated_at BEFORE UPDATE ON public.eval_cases FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_eval_run_datasets_updated_at BEFORE UPDATE ON public.eval_run_datasets FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_eval_run_share_tokens_updated_at BEFORE UPDATE ON public.eval_run_share_tokens FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_eval_runs_updated_at BEFORE UPDATE ON public.eval_runs FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_evals_updated_at BEFORE UPDATE ON public.evals FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_github_apps_updated_at BEFORE UPDATE ON public.github_apps FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_harnesses_updated_at BEFORE UPDATE ON public.harnesses FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_knowledge_bases_updated_at BEFORE UPDATE ON public.knowledge_bases FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_knowledge_entries_updated_at BEFORE UPDATE ON public.knowledge_entries FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_knowledge_index_documents_updated_at BEFORE UPDATE ON public.knowledge_index_documents FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_knowledge_indexes_updated_at BEFORE UPDATE ON public.knowledge_indexes FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_leased_resources_updated_at BEFORE UPDATE ON public.leased_resources FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_llm_models_updated_at BEFORE UPDATE ON public.models FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_llm_providers_updated_at BEFORE UPDATE ON public.providers FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_mcp_event_subscriptions_updated_at BEFORE UPDATE ON public.mcp_event_subscriptions FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_mcp_servers_updated_at BEFORE UPDATE ON public.mcp_servers FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_mcp_tool_labels_updated_at BEFORE UPDATE ON public.mcp_tool_labels FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_memories_updated_at BEFORE UPDATE ON public.memories FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_memory_files_updated_at BEFORE UPDATE ON public.memory_files FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_message_feedback_updated_at BEFORE UPDATE ON public.message_feedback FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_model_router_candidates_updated_at BEFORE UPDATE ON public.model_router_candidates FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_model_router_routes_updated_at BEFORE UPDATE ON public.model_router_routes FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_model_routers_updated_at BEFORE UPDATE ON public.model_routers FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_notifications_updated_at BEFORE UPDATE ON public.notifications FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_observers_updated_at BEFORE UPDATE ON public.observers FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_org_invitations_updated_at BEFORE UPDATE ON public.org_invitations FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_org_slack_connections_updated_at BEFORE UPDATE ON public.org_slack_connections FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_organizations_updated_at BEFORE UPDATE ON public.organizations FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_payment_accounts_updated_at BEFORE UPDATE ON public.payment_accounts FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_payment_attempts_updated_at BEFORE UPDATE ON public.payment_attempts FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_payment_policies_updated_at BEFORE UPDATE ON public.payment_policies FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_plugin_installs_updated_at BEFORE UPDATE ON public.plugin_installs FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_plugin_marketplaces_updated_at BEFORE UPDATE ON public.plugin_marketplaces FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_poppy_agent_conversations_updated_at BEFORE UPDATE ON public.poppy_agent_conversations FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_principals_updated_at BEFORE UPDATE ON public.principals FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_reporting_outbox_updated_at BEFORE UPDATE ON public.reporting_outbox FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_reporting_saved_reports_updated_at BEFORE UPDATE ON public.reporting_saved_reports FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_session_actors_updated_at BEFORE UPDATE ON public.session_actors FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_session_databases_updated_at BEFORE UPDATE ON public.session_databases FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_session_key_values_updated_at BEFORE UPDATE ON public.session_key_values FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_session_participants_updated_at BEFORE UPDATE ON public.session_participants FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_session_resources_updated_at BEFORE UPDATE ON public.session_resources FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_session_secrets_updated_at BEFORE UPDATE ON public.session_secrets FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_sessions_updated_at BEFORE UPDATE ON public.sessions FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_skills_updated_at BEFORE UPDATE ON public.skills FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_trace_scores_updated_at BEFORE UPDATE ON public.trace_scores FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_user_connections_updated_at BEFORE UPDATE ON public.pending_user_connections FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_user_preferences_updated_at BEFORE UPDATE ON public.user_preferences FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_users_updated_at BEFORE UPDATE ON public.users FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_workspace_files_updated_at BEFORE UPDATE ON public.workspace_files FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_workspaces_updated_at BEFORE UPDATE ON public.workspaces FOR EACH ROW EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER workspace_files_count_delete AFTER DELETE ON public.workspace_files REFERENCING OLD TABLE AS old_rows FOR EACH STATEMENT EXECUTE FUNCTION public.workspaces_file_count_delete();

CREATE TRIGGER workspace_files_count_insert AFTER INSERT ON public.workspace_files REFERENCING NEW TABLE AS new_rows FOR EACH STATEMENT EXECUTE FUNCTION public.workspaces_file_count_insert();

CREATE TRIGGER workspace_files_count_update AFTER UPDATE ON public.workspace_files REFERENCING OLD TABLE AS old_rows NEW TABLE AS new_rows FOR EACH STATEMENT EXECUTE FUNCTION public.workspaces_file_count_update();

ALTER TABLE ONLY public.a2a_push_configs
    ADD CONSTRAINT a2a_push_configs_session_id_fkey FOREIGN KEY (session_id) REFERENCES public.sessions(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.agent_avatar_variants
    ADD CONSTRAINT agent_avatar_variants_avatar_id_fkey FOREIGN KEY (avatar_id) REFERENCES public.agent_avatars(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.agent_avatars
    ADD CONSTRAINT agent_avatars_agent_id_fkey FOREIGN KEY (agent_id) REFERENCES public.agents(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.agent_avatars
    ADD CONSTRAINT agent_avatars_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id) ON DELETE CASCADE;

ALTER TABLE ONLY public.agent_capabilities
    ADD CONSTRAINT agent_capabilities_agent_id_fkey FOREIGN KEY (agent_id) REFERENCES public.agents(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.agent_channels
    ADD CONSTRAINT agent_channels_agent_id_fkey FOREIGN KEY (agent_id) REFERENCES public.agents(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.agent_channels
    ADD CONSTRAINT agent_channels_agent_identity_id_fkey FOREIGN KEY (virtual_user_id) REFERENCES public.virtual_users(id);

ALTER TABLE ONLY public.agent_channels
    ADD CONSTRAINT agent_channels_app_id_fkey FOREIGN KEY (app_id) REFERENCES public.apps(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.agent_channels
    ADD CONSTRAINT agent_channels_durable_schedule_id_fkey FOREIGN KEY (durable_schedule_id) REFERENCES public.durable_schedules(id) ON DELETE SET NULL;

ALTER TABLE ONLY public.agent_channels
    ADD CONSTRAINT agent_channels_owner_principal_id_fkey FOREIGN KEY (owner_principal_id) REFERENCES public.principals(id);

ALTER TABLE ONLY public.agent_channels
    ADD CONSTRAINT agent_channels_resolved_owner_user_id_fkey FOREIGN KEY (resolved_owner_user_id) REFERENCES public.users(id);

ALTER TABLE ONLY public.agent_check_rules
    ADD CONSTRAINT agent_check_rules_org_id_fk FOREIGN KEY (org_id) REFERENCES public.organizations(org_id) ON DELETE CASCADE;

ALTER TABLE ONLY public.agent_health_check_runs
    ADD CONSTRAINT agent_health_check_runs_agent_id_fk FOREIGN KEY (agent_id) REFERENCES public.agents(id) ON DELETE SET NULL;

ALTER TABLE ONLY public.agent_health_check_runs
    ADD CONSTRAINT agent_health_check_runs_org_id_fk FOREIGN KEY (org_id) REFERENCES public.organizations(org_id);

ALTER TABLE ONLY public.virtual_users
    ADD CONSTRAINT agent_identities_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id);

ALTER TABLE ONLY public.virtual_user_connections
    ADD CONSTRAINT agent_identity_connections_agent_identity_id_fkey FOREIGN KEY (virtual_user_id) REFERENCES public.virtual_users(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.agent_keys
    ADD CONSTRAINT agent_keys_channel_id_fkey FOREIGN KEY (channel_id) REFERENCES public.agent_channels(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.agent_keys
    ADD CONSTRAINT agent_keys_created_by_user_id_fkey FOREIGN KEY (created_by_user_id) REFERENCES public.users(id) ON DELETE SET NULL;

ALTER TABLE ONLY public.agent_keys
    ADD CONSTRAINT agent_keys_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id);

ALTER TABLE ONLY public.agent_mcp_secret_bindings
    ADD CONSTRAINT agent_mcp_secret_bindings_agent_id_fkey FOREIGN KEY (agent_id) REFERENCES public.agents(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.agent_mcp_secret_bindings
    ADD CONSTRAINT agent_mcp_secret_bindings_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id);

ALTER TABLE ONLY public.agent_scripts
    ADD CONSTRAINT agent_scripts_agent_id_fkey FOREIGN KEY (agent_id) REFERENCES public.agents(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.agent_scripts
    ADD CONSTRAINT agent_scripts_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id);

ALTER TABLE ONLY public.agent_trigger_deliveries
    ADD CONSTRAINT agent_trigger_deliveries_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id);

ALTER TABLE ONLY public.agent_trigger_deliveries
    ADD CONSTRAINT agent_trigger_deliveries_trigger_id_fkey FOREIGN KEY (trigger_id) REFERENCES public.agent_triggers(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.agent_trigger_mcp_subscriptions
    ADD CONSTRAINT agent_trigger_mcp_subscriptions_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id) ON DELETE CASCADE;

ALTER TABLE ONLY public.agent_trigger_mcp_subscriptions
    ADD CONSTRAINT agent_trigger_mcp_subscriptions_trigger_id_fkey FOREIGN KEY (trigger_id) REFERENCES public.agent_triggers(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.agent_trigger_subject_values
    ADD CONSTRAINT agent_trigger_subject_values_trigger_id_fkey FOREIGN KEY (trigger_id) REFERENCES public.agent_triggers(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.agent_triggers
    ADD CONSTRAINT agent_triggers_agent_id_fkey FOREIGN KEY (agent_id) REFERENCES public.agents(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.agent_triggers
    ADD CONSTRAINT agent_triggers_durable_schedule_id_fkey FOREIGN KEY (durable_schedule_id) REFERENCES public.durable_schedules(id) ON DELETE SET NULL;

ALTER TABLE ONLY public.agent_triggers
    ADD CONSTRAINT agent_triggers_execution_agent_identity_id_fkey FOREIGN KEY (execution_virtual_user_id) REFERENCES public.virtual_users(id) ON DELETE SET NULL;

ALTER TABLE ONLY public.agent_triggers
    ADD CONSTRAINT agent_triggers_execution_app_id_fkey FOREIGN KEY (execution_app_id) REFERENCES public.apps(id) ON DELETE SET NULL;

ALTER TABLE ONLY public.agent_triggers
    ADD CONSTRAINT agent_triggers_execution_harness_id_fkey FOREIGN KEY (execution_harness_id) REFERENCES public.harnesses(id) ON DELETE RESTRICT;

ALTER TABLE ONLY public.agent_triggers
    ADD CONSTRAINT agent_triggers_execution_owner_principal_id_fkey FOREIGN KEY (execution_owner_principal_id) REFERENCES public.principals(id) ON DELETE RESTRICT;

ALTER TABLE ONLY public.agent_triggers
    ADD CONSTRAINT agent_triggers_execution_resolved_owner_user_id_fkey FOREIGN KEY (execution_resolved_owner_user_id) REFERENCES public.users(id) ON DELETE SET NULL;

ALTER TABLE ONLY public.agent_triggers
    ADD CONSTRAINT agent_triggers_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id);

ALTER TABLE ONLY public.agentid_agent_owners
    ADD CONSTRAINT agentid_agent_owners_org_id_virtual_user_id_fkey FOREIGN KEY (org_id, virtual_user_id) REFERENCES public.virtual_users(org_id, id) ON DELETE CASCADE;

ALTER TABLE ONLY public.agentid_login_states
    ADD CONSTRAINT agentid_login_states_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id) ON DELETE CASCADE;

ALTER TABLE ONLY public.agents
    ADD CONSTRAINT agents_agent_identity_id_fkey FOREIGN KEY (virtual_user_id) REFERENCES public.virtual_users(id);

ALTER TABLE ONLY public.agents_api_sessions
    ADD CONSTRAINT agents_api_sessions_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id);

ALTER TABLE ONLY public.agents_api_sessions
    ADD CONSTRAINT agents_api_sessions_session_id_fkey FOREIGN KEY (session_id) REFERENCES public.sessions(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.agents
    ADD CONSTRAINT agents_avatar_id_fkey FOREIGN KEY (avatar_id) REFERENCES public.agent_avatars(id) ON DELETE SET NULL;

ALTER TABLE ONLY public.agents
    ADD CONSTRAINT agents_default_model_id_fkey FOREIGN KEY (default_model_id) REFERENCES public.models(id) ON DELETE SET NULL;

ALTER TABLE ONLY public.agents
    ADD CONSTRAINT agents_forked_from_agent_id_fkey FOREIGN KEY (forked_from_agent_id) REFERENCES public.agents(id) ON DELETE SET NULL;

ALTER TABLE ONLY public.agents
    ADD CONSTRAINT agents_harness_id_fkey FOREIGN KEY (harness_id) REFERENCES public.harnesses(id);

ALTER TABLE ONLY public.agents
    ADD CONSTRAINT agents_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id);

ALTER TABLE ONLY public.agents
    ADD CONSTRAINT agents_root_agent_id_fkey FOREIGN KEY (root_agent_id) REFERENCES public.agents(id) ON DELETE SET NULL;

ALTER TABLE ONLY public.agents
    ADD CONSTRAINT agents_service_virtual_user_org_fk FOREIGN KEY (org_id, virtual_user_id) REFERENCES public.virtual_users(org_id, id) ON DELETE SET NULL (virtual_user_id);

ALTER TABLE ONLY public.personal_access_tokens
    ADD CONSTRAINT api_keys_user_id_fkey FOREIGN KEY (user_id) REFERENCES public.users(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.apps
    ADD CONSTRAINT apps_agent_id_fkey FOREIGN KEY (agent_id) REFERENCES public.agents(id);

ALTER TABLE ONLY public.apps
    ADD CONSTRAINT apps_agent_identity_id_fkey FOREIGN KEY (virtual_user_id) REFERENCES public.virtual_users(id);

ALTER TABLE ONLY public.apps
    ADD CONSTRAINT apps_harness_id_fkey FOREIGN KEY (harness_id) REFERENCES public.harnesses(id);

ALTER TABLE ONLY public.apps
    ADD CONSTRAINT apps_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id);

ALTER TABLE ONLY public.apps
    ADD CONSTRAINT apps_owner_principal_id_fkey FOREIGN KEY (owner_principal_id) REFERENCES public.principals(id);

ALTER TABLE ONLY public.apps
    ADD CONSTRAINT apps_resolved_owner_user_id_fkey FOREIGN KEY (resolved_owner_user_id) REFERENCES public.users(id);

ALTER TABLE ONLY public.apps
    ADD CONSTRAINT apps_virtual_user_org_fk FOREIGN KEY (org_id, virtual_user_id) REFERENCES public.virtual_users(org_id, id) ON DELETE SET NULL (virtual_user_id);

ALTER TABLE ONLY public.audit_logs
    ADD CONSTRAINT audit_logs_actor_id_fkey FOREIGN KEY (actor_id) REFERENCES public.users(id) ON DELETE SET NULL;

ALTER TABLE ONLY public.audit_logs
    ADD CONSTRAINT audit_logs_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id);

ALTER TABLE ONLY public.budget_ledger
    ADD CONSTRAINT budget_ledger_budget_id_fkey FOREIGN KEY (budget_id) REFERENCES public.budgets(id);

ALTER TABLE ONLY public.budgets
    ADD CONSTRAINT budgets_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id);

ALTER TABLE ONLY public.channel_bindings
    ADD CONSTRAINT channel_bindings_channel_id_fkey FOREIGN KEY (channel_id) REFERENCES public.agent_channels(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.channel_bindings
    ADD CONSTRAINT channel_bindings_session_id_fkey FOREIGN KEY (session_id) REFERENCES public.sessions(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.channel_pending_deliveries
    ADD CONSTRAINT channel_pending_deliveries_channel_id_fkey FOREIGN KEY (channel_id) REFERENCES public.agent_channels(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.channel_pending_deliveries
    ADD CONSTRAINT channel_pending_deliveries_session_id_fkey FOREIGN KEY (session_id) REFERENCES public.sessions(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.channel_seen
    ADD CONSTRAINT channel_seen_channel_id_fkey FOREIGN KEY (channel_id) REFERENCES public.agent_channels(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.cli_auth_sessions
    ADD CONSTRAINT cli_auth_sessions_user_id_fkey FOREIGN KEY (user_id) REFERENCES public.users(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.command_idempotency_keys
    ADD CONSTRAINT command_idempotency_keys_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id) ON DELETE CASCADE;

ALTER TABLE ONLY public.decision_model_defaults
    ADD CONSTRAINT decision_model_defaults_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id) ON DELETE CASCADE;

ALTER TABLE ONLY public.decision_model_defaults
    ADD CONSTRAINT decision_model_defaults_org_id_model_id_fkey FOREIGN KEY (org_id, model_id) REFERENCES public.models(org_id, id) ON DELETE CASCADE;

ALTER TABLE ONLY public.declarative_capabilities
    ADD CONSTRAINT declarative_capabilities_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id);

ALTER TABLE ONLY public.durable_dead_letter_queue
    ADD CONSTRAINT durable_dead_letter_queue_workflow_id_fkey FOREIGN KEY (workflow_id) REFERENCES public.durable_workflow_instances(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.durable_schedule_executions
    ADD CONSTRAINT durable_schedule_executions_schedule_id_fkey FOREIGN KEY (schedule_id) REFERENCES public.durable_schedules(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.durable_signals
    ADD CONSTRAINT durable_signals_workflow_id_fkey FOREIGN KEY (workflow_id) REFERENCES public.durable_workflow_instances(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.durable_task_queue
    ADD CONSTRAINT durable_task_queue_workflow_id_fkey FOREIGN KEY (workflow_id) REFERENCES public.durable_workflow_instances(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.durable_workflow_events
    ADD CONSTRAINT durable_workflow_events_workflow_id_fkey FOREIGN KEY (workflow_id) REFERENCES public.durable_workflow_instances(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.durable_workflow_instances
    ADD CONSTRAINT durable_workflow_instances_continued_as_new_id_fkey FOREIGN KEY (continued_as_new_id) REFERENCES public.durable_workflow_instances(id);

ALTER TABLE ONLY public.durable_workflow_snapshots
    ADD CONSTRAINT durable_workflow_snapshots_workflow_id_fkey FOREIGN KEY (workflow_id) REFERENCES public.durable_workflow_instances(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.email_verification_tokens
    ADD CONSTRAINT email_verification_tokens_user_id_fkey FOREIGN KEY (user_id) REFERENCES public.users(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.entity_changes
    ADD CONSTRAINT entity_changes_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id) ON DELETE CASCADE;

ALTER TABLE ONLY public.entity_manager_context
    ADD CONSTRAINT entity_manager_context_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id) ON DELETE CASCADE;

ALTER TABLE ONLY public.eval_case_results
    ADD CONSTRAINT eval_case_results_case_id_fk FOREIGN KEY (eval_case_id) REFERENCES public.eval_cases(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.eval_case_results
    ADD CONSTRAINT eval_case_results_run_id_fk FOREIGN KEY (eval_run_id) REFERENCES public.eval_runs(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.eval_case_results
    ADD CONSTRAINT eval_case_results_session_id_fk FOREIGN KEY (session_id) REFERENCES public.sessions(id) ON DELETE SET NULL;

ALTER TABLE ONLY public.eval_cases
    ADD CONSTRAINT eval_cases_eval_id_fk FOREIGN KEY (eval_id) REFERENCES public.evals(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.eval_run_datasets
    ADD CONSTRAINT eval_run_datasets_eval_run_id_fk FOREIGN KEY (eval_run_id) REFERENCES public.eval_runs(id) ON DELETE SET NULL;

ALTER TABLE ONLY public.eval_run_datasets
    ADD CONSTRAINT eval_run_datasets_org_id_fk FOREIGN KEY (org_id) REFERENCES public.organizations(org_id);

ALTER TABLE ONLY public.eval_run_share_tokens
    ADD CONSTRAINT eval_run_share_tokens_created_by_fk FOREIGN KEY (created_by) REFERENCES public.users(id) ON DELETE SET NULL;

ALTER TABLE ONLY public.eval_run_share_tokens
    ADD CONSTRAINT eval_run_share_tokens_eval_run_id_fk FOREIGN KEY (eval_run_id) REFERENCES public.eval_runs(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.eval_run_share_tokens
    ADD CONSTRAINT eval_run_share_tokens_org_id_fk FOREIGN KEY (org_id) REFERENCES public.organizations(org_id);

ALTER TABLE ONLY public.eval_runs
    ADD CONSTRAINT eval_runs_eval_id_fk FOREIGN KEY (eval_id) REFERENCES public.evals(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.eval_runs
    ADD CONSTRAINT eval_runs_org_id_fk FOREIGN KEY (org_id) REFERENCES public.organizations(org_id);

ALTER TABLE ONLY public.evals
    ADD CONSTRAINT evals_org_id_fk FOREIGN KEY (org_id) REFERENCES public.organizations(org_id);

ALTER TABLE ONLY public.event_sequences
    ADD CONSTRAINT event_sequences_session_id_fkey FOREIGN KEY (session_id) REFERENCES public.sessions(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.events
    ADD CONSTRAINT events_session_id_fkey FOREIGN KEY (session_id) REFERENCES public.sessions(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.execution_environment_revisions
    ADD CONSTRAINT execution_environment_revisions_environment_id_fkey FOREIGN KEY (environment_id) REFERENCES public.execution_environments(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.execution_environments
    ADD CONSTRAINT execution_environments_current_revision_fk FOREIGN KEY (current_revision_id) REFERENCES public.execution_environment_revisions(id);

ALTER TABLE ONLY public.execution_environments
    ADD CONSTRAINT execution_environments_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id) ON DELETE CASCADE;

ALTER TABLE ONLY public.fact_budget_posting
    ADD CONSTRAINT fact_budget_posting_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id);

ALTER TABLE ONLY public.fact_capability_usage
    ADD CONSTRAINT fact_capability_usage_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id);

ALTER TABLE ONLY public.fact_llm_generation
    ADD CONSTRAINT fact_llm_generation_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id);

ALTER TABLE ONLY public.fact_session
    ADD CONSTRAINT fact_session_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id);

ALTER TABLE ONLY public.fact_tool_call
    ADD CONSTRAINT fact_tool_call_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id);

ALTER TABLE ONLY public.fact_turn
    ADD CONSTRAINT fact_turn_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id);

ALTER TABLE ONLY public.github_apps
    ADD CONSTRAINT github_apps_agent_identity_id_fkey FOREIGN KEY (virtual_user_id) REFERENCES public.virtual_users(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.github_apps
    ADD CONSTRAINT github_apps_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id);

ALTER TABLE ONLY public.github_apps
    ADD CONSTRAINT github_apps_virtual_user_org_fk FOREIGN KEY (org_id, virtual_user_id) REFERENCES public.virtual_users(org_id, id) ON DELETE CASCADE;

ALTER TABLE ONLY public.harness_capabilities
    ADD CONSTRAINT harness_capabilities_harness_id_fkey FOREIGN KEY (harness_id) REFERENCES public.harnesses(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.harnesses
    ADD CONSTRAINT harnesses_default_model_id_fkey FOREIGN KEY (default_model_id) REFERENCES public.models(id);

ALTER TABLE ONLY public.harnesses
    ADD CONSTRAINT harnesses_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id) ON DELETE CASCADE;

ALTER TABLE ONLY public.harnesses
    ADD CONSTRAINT harnesses_parent_harness_id_fkey FOREIGN KEY (parent_harness_id) REFERENCES public.harnesses(id) ON DELETE RESTRICT;

ALTER TABLE ONLY public.health_issue_reminders
    ADD CONSTRAINT health_issue_reminders_issue_id_fkey FOREIGN KEY (issue_id) REFERENCES public.health_issues(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.health_issue_reminders
    ADD CONSTRAINT health_issue_reminders_user_id_fkey FOREIGN KEY (user_id) REFERENCES public.users(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.health_issues
    ADD CONSTRAINT health_issues_channel_id_fkey FOREIGN KEY (channel_id) REFERENCES public.agent_channels(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.health_issues
    ADD CONSTRAINT health_issues_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id) ON DELETE CASCADE;

ALTER TABLE ONLY public.image_blobs
    ADD CONSTRAINT image_blobs_image_id_fkey FOREIGN KEY (image_id) REFERENCES public.images(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.images
    ADD CONSTRAINT images_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id);

ALTER TABLE ONLY public.knowledge_bases
    ADD CONSTRAINT knowledge_bases_embedding_model_id_fkey FOREIGN KEY (embedding_model_id) REFERENCES public.models(id) ON DELETE SET NULL;

ALTER TABLE ONLY public.knowledge_bases
    ADD CONSTRAINT knowledge_bases_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id);

ALTER TABLE ONLY public.knowledge_entries
    ADD CONSTRAINT knowledge_entries_kb_id_fkey FOREIGN KEY (kb_id) REFERENCES public.knowledge_bases(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.knowledge_index_chunks
    ADD CONSTRAINT knowledge_index_chunks_document_id_fkey FOREIGN KEY (document_id) REFERENCES public.knowledge_index_documents(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.knowledge_index_chunks
    ADD CONSTRAINT knowledge_index_chunks_index_id_fkey FOREIGN KEY (index_id) REFERENCES public.knowledge_indexes(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.knowledge_index_documents
    ADD CONSTRAINT knowledge_index_documents_index_id_fkey FOREIGN KEY (index_id) REFERENCES public.knowledge_indexes(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.knowledge_indexes
    ADD CONSTRAINT knowledge_indexes_embedding_model_id_fkey FOREIGN KEY (embedding_model_id) REFERENCES public.models(id);

ALTER TABLE ONLY public.knowledge_indexes
    ADD CONSTRAINT knowledge_indexes_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id);

ALTER TABLE ONLY public.leased_resources
    ADD CONSTRAINT leased_resources_connection_id_fkey FOREIGN KEY (connection_id) REFERENCES public.virtual_user_connections(id) ON DELETE SET NULL;

ALTER TABLE ONLY public.leased_resources
    ADD CONSTRAINT leased_resources_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id) ON DELETE CASCADE;

ALTER TABLE ONLY public.leased_resources
    ADD CONSTRAINT leased_resources_owner_virtual_user_fk FOREIGN KEY (org_id, owner_user_id) REFERENCES public.virtual_users(org_id, id) ON DELETE SET NULL (owner_user_id);

ALTER TABLE ONLY public.leased_resources
    ADD CONSTRAINT leased_resources_pending_connection_id_fkey FOREIGN KEY (pending_connection_id) REFERENCES public.pending_user_connections(id) ON DELETE RESTRICT;

ALTER TABLE ONLY public.leased_resources
    ADD CONSTRAINT leased_resources_session_id_fkey FOREIGN KEY (session_id) REFERENCES public.sessions(id) ON DELETE SET NULL;

ALTER TABLE ONLY public.llm_generations
    ADD CONSTRAINT llm_generations_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id);

ALTER TABLE ONLY public.llm_generations
    ADD CONSTRAINT llm_generations_session_id_fkey FOREIGN KEY (session_id) REFERENCES public.sessions(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.models
    ADD CONSTRAINT llm_models_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id);

ALTER TABLE ONLY public.models
    ADD CONSTRAINT llm_models_provider_id_fkey FOREIGN KEY (provider_id) REFERENCES public.providers(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.providers
    ADD CONSTRAINT llm_providers_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id);

ALTER TABLE ONLY public.mcp_event_subscriptions
    ADD CONSTRAINT mcp_event_subscriptions_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id) ON DELETE CASCADE;

ALTER TABLE ONLY public.mcp_event_subscriptions
    ADD CONSTRAINT mcp_event_subscriptions_user_id_fkey FOREIGN KEY (user_id) REFERENCES public.users(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.mcp_servers
    ADD CONSTRAINT mcp_servers_catalog_mcp_server_id_fkey FOREIGN KEY (catalog_mcp_server_id) REFERENCES public.mcp_servers(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.mcp_servers
    ADD CONSTRAINT mcp_servers_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id);

ALTER TABLE ONLY public.mcp_servers
    ADD CONSTRAINT mcp_servers_owner_virtual_user_fk FOREIGN KEY (org_id, owner_virtual_user_id) REFERENCES public.virtual_users(org_id, id) ON DELETE CASCADE;

ALTER TABLE ONLY public.mcp_service_tool_caches
    ADD CONSTRAINT mcp_service_tool_caches_agent_id_fkey FOREIGN KEY (agent_id) REFERENCES public.agents(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.mcp_service_tool_caches
    ADD CONSTRAINT mcp_service_tool_caches_mcp_server_id_fkey FOREIGN KEY (mcp_server_id) REFERENCES public.mcp_servers(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.mcp_service_tool_caches
    ADD CONSTRAINT mcp_service_tool_caches_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id) ON DELETE CASCADE;

ALTER TABLE ONLY public.mcp_tool_labels
    ADD CONSTRAINT mcp_tool_labels_mcp_server_id_fkey FOREIGN KEY (mcp_server_id) REFERENCES public.mcp_servers(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.mcp_tool_labels
    ADD CONSTRAINT mcp_tool_labels_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id);

ALTER TABLE ONLY public.mcp_tool_labels
    ADD CONSTRAINT mcp_tool_labels_set_by_fkey FOREIGN KEY (set_by) REFERENCES public.users(id) ON DELETE SET NULL;

ALTER TABLE ONLY public.memories
    ADD CONSTRAINT memories_owner_agent_id_fkey FOREIGN KEY (owner_agent_id) REFERENCES public.agents(id);

ALTER TABLE ONLY public.memories
    ADD CONSTRAINT memories_owner_user_id_fkey FOREIGN KEY (owner_user_id) REFERENCES public.users(id);

ALTER TABLE ONLY public.message_feedback
    ADD CONSTRAINT message_feedback_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id);

ALTER TABLE ONLY public.message_feedback
    ADD CONSTRAINT message_feedback_session_id_fkey FOREIGN KEY (session_id) REFERENCES public.sessions(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.message_feedback
    ADD CONSTRAINT message_feedback_user_id_fkey FOREIGN KEY (user_id) REFERENCES public.users(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.model_router_candidates
    ADD CONSTRAINT model_router_candidates_model_id_fkey FOREIGN KEY (model_id) REFERENCES public.models(id);

ALTER TABLE ONLY public.model_router_candidates
    ADD CONSTRAINT model_router_candidates_route_id_fkey FOREIGN KEY (route_id) REFERENCES public.model_router_routes(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.model_router_routes
    ADD CONSTRAINT model_router_routes_router_id_fkey FOREIGN KEY (router_id) REFERENCES public.model_routers(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.model_routers
    ADD CONSTRAINT model_routers_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id);

ALTER TABLE ONLY public.native_async_checkpoints
    ADD CONSTRAINT native_async_checkpoints_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id);

ALTER TABLE ONLY public.native_async_checkpoints
    ADD CONSTRAINT native_async_checkpoints_session_id_fkey FOREIGN KEY (session_id) REFERENCES public.sessions(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.notification_turn_requests
    ADD CONSTRAINT notification_turn_requests_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id) ON DELETE CASCADE;

ALTER TABLE ONLY public.notification_turn_requests
    ADD CONSTRAINT notification_turn_requests_session_id_fkey FOREIGN KEY (session_id) REFERENCES public.sessions(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.notification_turn_requests
    ADD CONSTRAINT notification_turn_requests_user_id_fkey FOREIGN KEY (user_id) REFERENCES public.users(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.notifications
    ADD CONSTRAINT notifications_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id) ON DELETE CASCADE;

ALTER TABLE ONLY public.notifications
    ADD CONSTRAINT notifications_user_id_fkey FOREIGN KEY (user_id) REFERENCES public.users(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.oauth_authorization_codes
    ADD CONSTRAINT oauth_authorization_codes_user_id_fkey FOREIGN KEY (user_id) REFERENCES public.users(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.oauth_grants
    ADD CONSTRAINT oauth_grants_client_id_fkey FOREIGN KEY (client_id) REFERENCES public.oauth_clients(client_id) ON DELETE CASCADE;

ALTER TABLE ONLY public.oauth_grants
    ADD CONSTRAINT oauth_grants_user_id_fkey FOREIGN KEY (user_id) REFERENCES public.users(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.oauth_refresh_tokens
    ADD CONSTRAINT oauth_refresh_tokens_grant_id_fkey FOREIGN KEY (grant_id) REFERENCES public.oauth_grants(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.oauth_refresh_tokens
    ADD CONSTRAINT oauth_refresh_tokens_user_id_fkey FOREIGN KEY (user_id) REFERENCES public.users(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.observers
    ADD CONSTRAINT observers_org_id_fk FOREIGN KEY (org_id) REFERENCES public.organizations(org_id);

ALTER TABLE ONLY public.org_feature_flags
    ADD CONSTRAINT org_feature_flags_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id) ON DELETE CASCADE;

ALTER TABLE ONLY public.org_invitations
    ADD CONSTRAINT org_invitations_accepted_by_fkey FOREIGN KEY (accepted_by) REFERENCES public.users(id) ON DELETE SET NULL;

ALTER TABLE ONLY public.org_invitations
    ADD CONSTRAINT org_invitations_invited_by_fkey FOREIGN KEY (invited_by) REFERENCES public.users(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.org_invitations
    ADD CONSTRAINT org_invitations_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id) ON DELETE CASCADE;

ALTER TABLE ONLY public.org_slack_connections
    ADD CONSTRAINT org_slack_connections_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id) ON DELETE CASCADE;

ALTER TABLE ONLY public.organization_members
    ADD CONSTRAINT organization_members_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id) ON DELETE CASCADE;

ALTER TABLE ONLY public.organization_members
    ADD CONSTRAINT organization_members_user_id_fkey FOREIGN KEY (user_id) REFERENCES public.users(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.organization_settings
    ADD CONSTRAINT organization_settings_base_harness_id_fkey FOREIGN KEY (base_harness_id) REFERENCES public.harnesses(id) ON DELETE SET NULL;

ALTER TABLE ONLY public.organization_settings
    ADD CONSTRAINT organization_settings_default_harness_id_fkey FOREIGN KEY (default_harness_id) REFERENCES public.harnesses(id) ON DELETE SET NULL;

ALTER TABLE ONLY public.organization_settings
    ADD CONSTRAINT organization_settings_default_model_id_fkey FOREIGN KEY (default_model_id) REFERENCES public.models(id) ON DELETE SET NULL;

ALTER TABLE ONLY public.organization_settings
    ADD CONSTRAINT organization_settings_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id) ON DELETE CASCADE;

ALTER TABLE ONLY public.organization_task_webhooks
    ADD CONSTRAINT organization_task_webhooks_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id) ON DELETE CASCADE;

ALTER TABLE ONLY public.organizations
    ADD CONSTRAINT organizations_created_by_fkey FOREIGN KEY (created_by) REFERENCES public.users(id) ON DELETE SET NULL;

ALTER TABLE ONLY public.pact_device_authorizations
    ADD CONSTRAINT pact_device_authorizations_channel_id_fkey FOREIGN KEY (channel_id) REFERENCES public.agent_channels(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.pact_device_authorizations
    ADD CONSTRAINT pact_device_authorizations_grant_id_fkey FOREIGN KEY (grant_id) REFERENCES public.pact_grants(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.pact_grants
    ADD CONSTRAINT pact_grants_channel_id_fkey FOREIGN KEY (channel_id) REFERENCES public.agent_channels(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.pact_refresh_tokens
    ADD CONSTRAINT pact_refresh_tokens_grant_id_fkey FOREIGN KEY (grant_id) REFERENCES public.pact_grants(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.pact_signing_keys
    ADD CONSTRAINT pact_signing_keys_channel_id_fkey FOREIGN KEY (channel_id) REFERENCES public.agent_channels(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.pact_used_assertions
    ADD CONSTRAINT pact_used_assertions_channel_id_fkey FOREIGN KEY (channel_id) REFERENCES public.agent_channels(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.password_reset_tokens
    ADD CONSTRAINT password_reset_tokens_user_id_fkey FOREIGN KEY (user_id) REFERENCES public.users(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.payment_accounts
    ADD CONSTRAINT payment_accounts_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id);

ALTER TABLE ONLY public.payment_attempts
    ADD CONSTRAINT payment_attempts_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id);

ALTER TABLE ONLY public.payment_attempts
    ADD CONSTRAINT payment_attempts_payment_account_id_fkey FOREIGN KEY (payment_account_id) REFERENCES public.payment_accounts(id) ON DELETE SET NULL;

ALTER TABLE ONLY public.payment_attempts
    ADD CONSTRAINT payment_attempts_payment_policy_id_fkey FOREIGN KEY (payment_policy_id) REFERENCES public.payment_policies(id) ON DELETE SET NULL;

ALTER TABLE ONLY public.payment_attempts
    ADD CONSTRAINT payment_attempts_session_id_fkey FOREIGN KEY (session_id) REFERENCES public.sessions(id) ON DELETE SET NULL;

ALTER TABLE ONLY public.payment_policies
    ADD CONSTRAINT payment_policies_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id);

ALTER TABLE ONLY public.payment_policies
    ADD CONSTRAINT payment_policies_payment_account_id_fkey FOREIGN KEY (payment_account_id) REFERENCES public.payment_accounts(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.pinned_sessions
    ADD CONSTRAINT pinned_sessions_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id) ON DELETE CASCADE;

ALTER TABLE ONLY public.pinned_sessions
    ADD CONSTRAINT pinned_sessions_session_id_fkey FOREIGN KEY (session_id) REFERENCES public.sessions(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.pinned_sessions
    ADD CONSTRAINT pinned_sessions_virtual_user_fkey FOREIGN KEY (user_id) REFERENCES public.virtual_users(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.plugin_installs
    ADD CONSTRAINT plugin_installs_marketplace_id_fkey FOREIGN KEY (marketplace_id) REFERENCES public.plugin_marketplaces(id) ON DELETE SET NULL;

ALTER TABLE ONLY public.plugin_installs
    ADD CONSTRAINT plugin_installs_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id);

ALTER TABLE ONLY public.plugin_marketplaces
    ADD CONSTRAINT plugin_marketplaces_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id) ON DELETE CASCADE;

ALTER TABLE ONLY public.poppy_agent_accounts
    ADD CONSTRAINT poppy_agent_accounts_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id) ON DELETE CASCADE;

ALTER TABLE ONLY public.poppy_agent_accounts
    ADD CONSTRAINT poppy_agent_accounts_owner_principal_id_fkey FOREIGN KEY (owner_principal_id) REFERENCES public.principals(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.poppy_agent_conversations
    ADD CONSTRAINT poppy_agent_conversations_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id) ON DELETE CASCADE;

ALTER TABLE ONLY public.poppy_agent_conversations
    ADD CONSTRAINT poppy_agent_conversations_session_id_fkey FOREIGN KEY (session_id) REFERENCES public.sessions(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.poppy_conversations
    ADD CONSTRAINT poppy_conversations_channel_id_fkey FOREIGN KEY (channel_id) REFERENCES public.agent_channels(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.poppy_conversations
    ADD CONSTRAINT poppy_conversations_session_id_fkey FOREIGN KEY (session_id) REFERENCES public.sessions(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.poppy_messages
    ADD CONSTRAINT poppy_messages_channel_id_fkey FOREIGN KEY (channel_id) REFERENCES public.agent_channels(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.poppy_messages
    ADD CONSTRAINT poppy_messages_session_id_fkey FOREIGN KEY (session_id) REFERENCES public.sessions(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.poppy_sessions
    ADD CONSTRAINT poppy_sessions_channel_id_fkey FOREIGN KEY (channel_id) REFERENCES public.agent_channels(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.principals
    ADD CONSTRAINT principals_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id) ON DELETE CASCADE;

ALTER TABLE ONLY public.principals
    ADD CONSTRAINT principals_parent_principal_id_fkey FOREIGN KEY (parent_principal_id) REFERENCES public.principals(id) ON DELETE RESTRICT;

ALTER TABLE ONLY public.principals
    ADD CONSTRAINT principals_resolved_user_id_fkey FOREIGN KEY (resolved_user_id) REFERENCES public.users(id) ON DELETE RESTRICT;

ALTER TABLE ONLY public.refresh_tokens
    ADD CONSTRAINT refresh_tokens_user_id_fkey FOREIGN KEY (user_id) REFERENCES public.users(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.reporting_outbox
    ADD CONSTRAINT reporting_outbox_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id);

ALTER TABLE ONLY public.reporting_saved_reports
    ADD CONSTRAINT reporting_saved_reports_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id);

ALTER TABLE ONLY public.runtime_invocations
    ADD CONSTRAINT runtime_invocations_management_user_id_fkey FOREIGN KEY (management_user_id) REFERENCES public.users(id) ON DELETE SET NULL;

ALTER TABLE ONLY public.runtime_invocations
    ADD CONSTRAINT runtime_invocations_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id) ON DELETE CASCADE;

ALTER TABLE ONLY public.runtime_invocations
    ADD CONSTRAINT runtime_invocations_org_id_virtual_user_id_fkey FOREIGN KEY (org_id, virtual_user_id) REFERENCES public.virtual_users(org_id, id) ON DELETE RESTRICT;

ALTER TABLE ONLY public.runtime_invocations
    ADD CONSTRAINT runtime_invocations_responder_agent_id_fkey FOREIGN KEY (responder_agent_id) REFERENCES public.agents(id) ON DELETE SET NULL;

ALTER TABLE ONLY public.runtime_invocations
    ADD CONSTRAINT runtime_invocations_session_id_fkey FOREIGN KEY (session_id) REFERENCES public.sessions(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.sandbox_checkpoints
    ADD CONSTRAINT sandbox_checkpoints_sandbox_id_fkey FOREIGN KEY (sandbox_id) REFERENCES public.sandboxes(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.sandbox_instances
    ADD CONSTRAINT sandbox_instances_sandbox_id_fkey FOREIGN KEY (sandbox_id) REFERENCES public.sandboxes(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.sandbox_orphan_sightings
    ADD CONSTRAINT sandbox_orphan_sightings_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id) ON DELETE CASCADE;

ALTER TABLE ONLY public.sandbox_state_transitions
    ADD CONSTRAINT sandbox_state_transitions_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id) ON DELETE CASCADE;

ALTER TABLE ONLY public.sandbox_state_transitions
    ADD CONSTRAINT sandbox_state_transitions_sandbox_id_fkey FOREIGN KEY (sandbox_id) REFERENCES public.sandboxes(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.sandboxes
    ADD CONSTRAINT sandboxes_current_checkpoint_fk FOREIGN KEY (current_checkpoint_id) REFERENCES public.sandbox_checkpoints(id) ON DELETE RESTRICT;

ALTER TABLE ONLY public.sandboxes
    ADD CONSTRAINT sandboxes_current_instance_fk FOREIGN KEY (current_instance_id) REFERENCES public.sandbox_instances(id) ON DELETE SET NULL;

ALTER TABLE ONLY public.sandboxes
    ADD CONSTRAINT sandboxes_environment_revision_id_fkey FOREIGN KEY (environment_revision_id) REFERENCES public.execution_environment_revisions(id);

ALTER TABLE ONLY public.sandboxes
    ADD CONSTRAINT sandboxes_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id) ON DELETE CASCADE;

ALTER TABLE ONLY public.sandboxes
    ADD CONSTRAINT sandboxes_session_id_fkey FOREIGN KEY (session_id) REFERENCES public.sessions(id) ON DELETE SET NULL;

ALTER TABLE ONLY public.session_actors
    ADD CONSTRAINT session_actors_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id);

ALTER TABLE ONLY public.session_actors
    ADD CONSTRAINT session_actors_session_id_fkey FOREIGN KEY (session_id) REFERENCES public.sessions(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.session_compaction_checkpoints
    ADD CONSTRAINT session_compaction_checkpoints_session_id_fkey FOREIGN KEY (session_id) REFERENCES public.sessions(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.session_database_pages
    ADD CONSTRAINT session_database_pages_database_id_fkey FOREIGN KEY (database_id) REFERENCES public.session_databases(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.session_databases
    ADD CONSTRAINT session_databases_session_id_fkey FOREIGN KEY (session_id) REFERENCES public.sessions(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.session_git_objects
    ADD CONSTRAINT session_git_objects_session_id_fkey FOREIGN KEY (session_id) REFERENCES public.sessions(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.session_git_refs
    ADD CONSTRAINT session_git_refs_session_id_fkey FOREIGN KEY (session_id) REFERENCES public.sessions(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.session_key_values
    ADD CONSTRAINT session_key_values_session_id_fkey FOREIGN KEY (session_id) REFERENCES public.sessions(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.session_participants
    ADD CONSTRAINT session_participants_agent_id_fkey FOREIGN KEY (agent_id) REFERENCES public.agents(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.session_participants
    ADD CONSTRAINT session_participants_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id) ON DELETE CASCADE;

ALTER TABLE ONLY public.session_participants
    ADD CONSTRAINT session_participants_principal_id_fkey FOREIGN KEY (principal_id) REFERENCES public.principals(id) ON DELETE RESTRICT;

ALTER TABLE ONLY public.session_participants
    ADD CONSTRAINT session_participants_session_id_fkey FOREIGN KEY (session_id) REFERENCES public.sessions(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.session_resources
    ADD CONSTRAINT session_resources_session_id_fkey FOREIGN KEY (session_id) REFERENCES public.sessions(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.session_schedules
    ADD CONSTRAINT session_schedules_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id) ON DELETE CASCADE;

ALTER TABLE ONLY public.session_schedules
    ADD CONSTRAINT session_schedules_owner_principal_id_fkey FOREIGN KEY (owner_principal_id) REFERENCES public.principals(id);

ALTER TABLE ONLY public.session_schedules
    ADD CONSTRAINT session_schedules_resolved_owner_user_id_fkey FOREIGN KEY (resolved_owner_user_id) REFERENCES public.users(id);

ALTER TABLE ONLY public.session_schedules
    ADD CONSTRAINT session_schedules_session_id_fkey FOREIGN KEY (session_id) REFERENCES public.sessions(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.session_secrets
    ADD CONSTRAINT session_secrets_session_id_fkey FOREIGN KEY (session_id) REFERENCES public.sessions(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.session_secrets
    ADD CONSTRAINT session_secrets_virtual_user_id_fkey FOREIGN KEY (virtual_user_id) REFERENCES public.virtual_users(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.session_task_messages
    ADD CONSTRAINT session_task_messages_task_id_fkey FOREIGN KEY (task_id) REFERENCES public.session_tasks(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.session_task_push_configs
    ADD CONSTRAINT session_task_push_configs_session_id_fkey FOREIGN KEY (session_id) REFERENCES public.sessions(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.session_task_push_configs
    ADD CONSTRAINT session_task_push_configs_task_id_fkey FOREIGN KEY (task_id) REFERENCES public.session_tasks(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.session_tasks
    ADD CONSTRAINT session_tasks_session_id_fkey FOREIGN KEY (session_id) REFERENCES public.sessions(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.session_trace_state
    ADD CONSTRAINT session_trace_state_session_id_fkey FOREIGN KEY (session_id) REFERENCES public.sessions(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.session_trace_steps
    ADD CONSTRAINT session_trace_steps_session_id_turn_no_fkey FOREIGN KEY (session_id, turn_no) REFERENCES public.session_trace_turns(session_id, turn_no) ON DELETE CASCADE;

ALTER TABLE ONLY public.session_trace_turns
    ADD CONSTRAINT session_trace_turns_session_id_fkey FOREIGN KEY (session_id) REFERENCES public.sessions(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.session_memory_mounts
    ADD CONSTRAINT session_volume_mounts_session_id_fkey FOREIGN KEY (session_id) REFERENCES public.sessions(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.session_memory_mounts
    ADD CONSTRAINT session_volume_mounts_volume_id_fkey FOREIGN KEY (memory_id) REFERENCES public.memories(id);

ALTER TABLE ONLY public.sessions
    ADD CONSTRAINT sessions_agent_id_fkey FOREIGN KEY (agent_id) REFERENCES public.agents(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.sessions
    ADD CONSTRAINT sessions_agent_identity_id_fkey FOREIGN KEY (virtual_user_id) REFERENCES public.virtual_users(id);

ALTER TABLE ONLY public.sessions
    ADD CONSTRAINT sessions_app_id_fkey FOREIGN KEY (app_id) REFERENCES public.apps(id) ON DELETE SET NULL;

ALTER TABLE ONLY public.sessions
    ADD CONSTRAINT sessions_channel_id_fkey FOREIGN KEY (channel_id) REFERENCES public.agent_channels(id) ON DELETE SET NULL;

ALTER TABLE ONLY public.sessions
    ADD CONSTRAINT sessions_forked_from_session_id_fkey FOREIGN KEY (forked_from_session_id) REFERENCES public.sessions(id) ON DELETE SET NULL;

ALTER TABLE ONLY public.sessions
    ADD CONSTRAINT sessions_harness_id_fkey FOREIGN KEY (harness_id) REFERENCES public.harnesses(id);

ALTER TABLE ONLY public.sessions
    ADD CONSTRAINT sessions_model_id_fkey FOREIGN KEY (model_id) REFERENCES public.models(id);

ALTER TABLE ONLY public.sessions
    ADD CONSTRAINT sessions_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id);

ALTER TABLE ONLY public.sessions
    ADD CONSTRAINT sessions_owner_principal_id_fkey FOREIGN KEY (owner_principal_id) REFERENCES public.principals(id);

ALTER TABLE ONLY public.sessions
    ADD CONSTRAINT sessions_parent_session_id_fkey FOREIGN KEY (parent_session_id) REFERENCES public.sessions(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.sessions
    ADD CONSTRAINT sessions_playground_user_org_fk FOREIGN KEY (org_id, playground_user_id) REFERENCES public.virtual_users(org_id, id) ON DELETE RESTRICT;

ALTER TABLE ONLY public.sessions
    ADD CONSTRAINT sessions_resolved_owner_user_id_fkey FOREIGN KEY (resolved_owner_user_id) REFERENCES public.users(id);

ALTER TABLE ONLY public.sessions
    ADD CONSTRAINT sessions_root_session_id_fkey FOREIGN KEY (root_session_id) REFERENCES public.sessions(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.sessions
    ADD CONSTRAINT sessions_trigger_id_fkey FOREIGN KEY (trigger_id) REFERENCES public.agent_triggers(id) ON DELETE SET NULL;

ALTER TABLE ONLY public.sessions
    ADD CONSTRAINT sessions_virtual_user_org_fk FOREIGN KEY (org_id, virtual_user_id) REFERENCES public.virtual_users(org_id, id) ON DELETE SET NULL (virtual_user_id);

ALTER TABLE ONLY public.sessions
    ADD CONSTRAINT sessions_workspace_id_fkey FOREIGN KEY (workspace_id) REFERENCES public.workspaces(id);

ALTER TABLE ONLY public.skill_files
    ADD CONSTRAINT skill_files_skill_id_fkey FOREIGN KEY (skill_id) REFERENCES public.skills(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.skills
    ADD CONSTRAINT skills_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id);

ALTER TABLE ONLY public.subagent_spawn_handles
    ADD CONSTRAINT subagent_spawn_handles_parent_session_id_fkey FOREIGN KEY (parent_session_id) REFERENCES public.sessions(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.trace_scores
    ADD CONSTRAINT trace_scores_observer_id_fk FOREIGN KEY (observer_id) REFERENCES public.observers(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.trace_scores
    ADD CONSTRAINT trace_scores_org_id_fk FOREIGN KEY (org_id) REFERENCES public.organizations(org_id);

ALTER TABLE ONLY public.trace_scores
    ADD CONSTRAINT trace_scores_session_id_fk FOREIGN KEY (session_id) REFERENCES public.sessions(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.agent_triggers
    ADD CONSTRAINT triggers_virtual_user_org_fk FOREIGN KEY (org_id, execution_virtual_user_id) REFERENCES public.virtual_users(org_id, id) ON DELETE SET NULL (execution_virtual_user_id);

ALTER TABLE ONLY public.usage_journal
    ADD CONSTRAINT usage_journal_agent_id_fkey FOREIGN KEY (agent_id) REFERENCES public.agents(id) ON DELETE SET NULL;

ALTER TABLE ONLY public.usage_journal
    ADD CONSTRAINT usage_journal_event_id_fkey FOREIGN KEY (event_id) REFERENCES public.events(id) ON DELETE SET NULL;

ALTER TABLE ONLY public.usage_journal
    ADD CONSTRAINT usage_journal_harness_id_fkey FOREIGN KEY (harness_id) REFERENCES public.harnesses(id) ON DELETE SET NULL;

ALTER TABLE ONLY public.usage_journal
    ADD CONSTRAINT usage_journal_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id);

ALTER TABLE ONLY public.usage_journal
    ADD CONSTRAINT usage_journal_principal_id_fkey FOREIGN KEY (principal_id) REFERENCES public.principals(id) ON DELETE SET NULL;

ALTER TABLE ONLY public.usage_journal
    ADD CONSTRAINT usage_journal_session_id_fkey FOREIGN KEY (session_id) REFERENCES public.sessions(id) ON DELETE SET NULL;

ALTER TABLE ONLY public.usage_journal
    ADD CONSTRAINT usage_journal_user_id_fkey FOREIGN KEY (user_id) REFERENCES public.users(id) ON DELETE SET NULL;

ALTER TABLE ONLY public.usage_ledger
    ADD CONSTRAINT usage_ledger_agent_id_fkey FOREIGN KEY (agent_id) REFERENCES public.agents(id) ON DELETE SET NULL;

ALTER TABLE ONLY public.usage_ledger
    ADD CONSTRAINT usage_ledger_budget_id_fkey FOREIGN KEY (budget_id) REFERENCES public.budgets(id) ON DELETE SET NULL;

ALTER TABLE ONLY public.usage_ledger
    ADD CONSTRAINT usage_ledger_harness_id_fkey FOREIGN KEY (harness_id) REFERENCES public.harnesses(id) ON DELETE SET NULL;

ALTER TABLE ONLY public.usage_ledger
    ADD CONSTRAINT usage_ledger_journal_id_fkey FOREIGN KEY (journal_id) REFERENCES public.usage_journal(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.usage_ledger
    ADD CONSTRAINT usage_ledger_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id);

ALTER TABLE ONLY public.usage_ledger
    ADD CONSTRAINT usage_ledger_principal_id_fkey FOREIGN KEY (principal_id) REFERENCES public.principals(id) ON DELETE SET NULL;

ALTER TABLE ONLY public.usage_ledger
    ADD CONSTRAINT usage_ledger_session_id_fkey FOREIGN KEY (session_id) REFERENCES public.sessions(id) ON DELETE SET NULL;

ALTER TABLE ONLY public.usage_ledger
    ADD CONSTRAINT usage_ledger_user_id_fkey FOREIGN KEY (user_id) REFERENCES public.users(id) ON DELETE SET NULL;

ALTER TABLE ONLY public.pending_user_connections
    ADD CONSTRAINT user_connections_user_id_fkey FOREIGN KEY (user_id) REFERENCES public.users(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.user_oauth_identities
    ADD CONSTRAINT user_oauth_identities_user_id_fkey FOREIGN KEY (user_id) REFERENCES public.users(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.user_preferences
    ADD CONSTRAINT user_preferences_user_id_fkey FOREIGN KEY (user_id) REFERENCES public.users(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.virtual_user_bindings
    ADD CONSTRAINT virtual_user_bindings_management_user_id_fkey FOREIGN KEY (management_user_id) REFERENCES public.users(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.virtual_user_bindings
    ADD CONSTRAINT virtual_user_bindings_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id) ON DELETE CASCADE;

ALTER TABLE ONLY public.virtual_user_bindings
    ADD CONSTRAINT virtual_user_bindings_org_id_virtual_user_id_fkey FOREIGN KEY (org_id, virtual_user_id) REFERENCES public.virtual_users(org_id, id) ON DELETE CASCADE;

ALTER TABLE ONLY public.virtual_user_preferences
    ADD CONSTRAINT virtual_user_preferences_virtual_user_id_fkey FOREIGN KEY (virtual_user_id) REFERENCES public.virtual_users(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.memory_files
    ADD CONSTRAINT volume_files_volume_id_fkey FOREIGN KEY (memory_id) REFERENCES public.memories(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.memories
    ADD CONSTRAINT volumes_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id);

ALTER TABLE ONLY public.workspace_file_blobs
    ADD CONSTRAINT workspace_file_blobs_file_id_fkey FOREIGN KEY (file_id) REFERENCES public.workspace_files(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.workspace_files
    ADD CONSTRAINT workspace_files_workspace_id_fkey FOREIGN KEY (workspace_id) REFERENCES public.workspaces(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.workspaces
    ADD CONSTRAINT workspaces_org_id_fkey FOREIGN KEY (org_id) REFERENCES public.organizations(org_id);

-- Data plane v2 review fixes.

-- 1. A receipt per host call that writes (`domains/data_plane/host_calls.rs`):
-- a call id the control plane has seen runs once. A duplicate gets the
-- answer the first run sealed, or, while the first is still running (or
-- never finished), a refusal that says the outcome is unknown.
CREATE TABLE dp_host_call_receipts (
    -- The id the answer is sealed under (`host-call:{call}`, `lookup:{id}`).
    receipt_id TEXT PRIMARY KEY,
    org_id BIGINT NOT NULL,
    call_name TEXT NOT NULL,
    -- The sealed answer, once the call finished.
    answer JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX dp_host_call_receipts_created ON dp_host_call_receipts (created_at);

-- 2. The org settings a data plane holds a copy of (`org_settings.rs`) are
-- owed a sync whenever their rows change, in the same transaction as the
-- change: `settings_rev` counts changes, `settings_synced_rev` is the last
-- change a sync covered. The dispatcher syncs every org that is behind, so a
-- sync that failed after the change committed is retried, never lost.
ALTER TABLE dp_orgs
    ADD COLUMN settings_rev BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN settings_synced_rev BIGINT NOT NULL DEFAULT 0;

CREATE FUNCTION public.dp_org_settings_changed() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    UPDATE dp_orgs SET settings_rev = settings_rev + 1
    WHERE org_id = COALESCE(NEW.org_id, OLD.org_id);
    RETURN NULL;
END;
$$;

CREATE TRIGGER dp_org_settings_changed
    AFTER INSERT OR UPDATE OR DELETE ON public.organization_settings
    FOR EACH ROW EXECUTE FUNCTION public.dp_org_settings_changed();

-- An org's connections (its search accounts and their keys) are the
-- connections of its organization virtual user.
CREATE FUNCTION public.dp_org_connections_changed() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    UPDATE dp_orgs SET settings_rev = settings_rev + 1
    WHERE org_id IN (
        SELECT org_id FROM virtual_users
        WHERE id = COALESCE(NEW.virtual_user_id, OLD.virtual_user_id)
          AND usage = 'organization'
    );
    RETURN NULL;
END;
$$;

CREATE TRIGGER dp_org_connections_changed
    AFTER INSERT OR UPDATE OR DELETE ON public.virtual_user_connections
    FOR EACH ROW EXECUTE FUNCTION public.dp_org_connections_changed();

-- 3. The signing secrets of v2 task push configs stay in the control plane,
-- encrypted with the server's secrets key; the cell keeps only that the
-- config has one (`domains/data_plane/task_webhooks.rs`).
CREATE TABLE dp_task_push_secrets (
    config_id TEXT PRIMARY KEY,
    -- Row id for secrets key rotation, which walks rows by a UUID.
    id UUID NOT NULL UNIQUE DEFAULT gen_random_uuid(),
    org_id BIGINT NOT NULL,
    session_id TEXT NOT NULL,
    secret_encrypted BYTEA NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

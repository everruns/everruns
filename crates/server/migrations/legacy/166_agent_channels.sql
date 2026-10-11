-- Channels own the same ingress records; identities, secrets and attribution
-- remain intact. Roll API and workers together: older builds name the old table,
-- session column and spend subjects. This is a catalog rename, not a backfill.
ALTER TABLE agent_endpoints RENAME TO agent_channels;
ALTER TABLE sessions RENAME COLUMN endpoint_id TO channel_id;

-- PostgreSQL retains dependent views and foreign keys on a relation rename.
-- Rename their catalog labels as well so operational tools use one vocabulary.
DO $$
DECLARE item RECORD; renamed TEXT;
BEGIN
    FOR item IN
        SELECT conrelid::regclass AS relation, conname
        FROM pg_constraint
        WHERE conrelid IN ('agent_channels'::regclass, 'sessions'::regclass)
          AND (conname LIKE '%agent_endpoints%' OR conname LIKE '%endpoint_id%')
    LOOP
        renamed := replace(replace(item.conname, 'agent_endpoints', 'agent_channels'), 'endpoint_id', 'channel_id');
        EXECUTE format('ALTER TABLE %s RENAME CONSTRAINT %I TO %I', item.relation, item.conname, renamed);
    END LOOP;
    FOR item IN
        SELECT i.indexrelid::regclass AS relation, c.relname
        FROM pg_index i JOIN pg_class c ON c.oid = i.indexrelid
        WHERE i.indrelid IN ('agent_channels'::regclass, 'sessions'::regclass)
          AND (c.relname LIKE '%agent_endpoints%' OR c.relname LIKE '%endpoint_id%')
    LOOP
        renamed := replace(replace(item.relname, 'agent_endpoints', 'agent_channels'), 'endpoint_id', 'channel_id');
        EXECUTE format('ALTER INDEX %s RENAME TO %I', item.relation, renamed);
    END LOOP;
END;
$$;
ALTER TRIGGER update_agent_endpoints_updated_at ON agent_channels RENAME TO update_agent_channels_updated_at;
ALTER TRIGGER endpoint_virtual_user_org_guard ON agent_channels RENAME TO channel_virtual_user_org_guard;
ALTER FUNCTION enforce_endpoint_virtual_user_org() RENAME TO enforce_channel_virtual_user_org;
CREATE OR REPLACE FUNCTION enforce_channel_virtual_user_org() RETURNS trigger LANGUAGE plpgsql AS $$
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

ALTER TABLE budgets DROP CONSTRAINT budgets_subject_type_check;
UPDATE budgets SET subject_type = 'agent_channel' WHERE subject_type = 'agent_endpoint';
ALTER TABLE budgets ADD CONSTRAINT budgets_subject_type_check
    CHECK (subject_type IN ('session', 'agent', 'user', 'org', 'agent_trigger', 'agent_channel'));
ALTER TABLE payment_policies DROP CONSTRAINT payment_policies_subject_type_check;
UPDATE payment_policies SET subject_type = 'agent_channel' WHERE subject_type = 'agent_endpoint';
ALTER TABLE payment_policies ADD CONSTRAINT payment_policies_subject_type_check
    CHECK (subject_type IN ('user', 'virtual_user', 'agent', 'agent_channel', 'session', 'org'));

COMMENT ON COLUMN sessions.channel_id IS 'Channel whose ingress created this session. Archival App attribution remains in app_id.';
COMMENT ON COLUMN sessions.app_id IS 'Archival provenance for sessions created through the retired App model; channel_id is the live ingress pointer.';
COMMENT ON TABLE agent_channels IS 'Agent-owned communication channels. Public IDs, archival App aliases and encrypted configuration are preserved.';

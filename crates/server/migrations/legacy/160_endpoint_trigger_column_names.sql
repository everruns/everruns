-- EVE-1131 step 5: endpoint/trigger-oriented names for the App-era columns
-- that live ingress and trigger execution still read. Pure renames: values,
-- types, nullability, and indexes are unchanged (a RENAME is catalog-only).
--
-- Rollout: like 154_virtual_users.sql, this is a plain rename rather than
-- expand/contract. Migrations run at server startup, so an instance still on
-- the previous build fails queries that name the old columns until it is
-- replaced; roll the API and worker together.

-- Immutable `{app_id}` segment of the permanent /v1/apps/{app_id}/... aliases.
ALTER TABLE agent_endpoints RENAME COLUMN legacy_app_public_id TO legacy_alias_id;
ALTER INDEX idx_agent_endpoints_legacy_app_channel_type
    RENAME TO idx_agent_endpoints_legacy_alias_channel_type;

-- Same alias identity for webhook triggers migrated from Apps (migration 138).
ALTER TABLE agent_triggers RENAME COLUMN execution_app_public_id TO legacy_alias_id;
ALTER TABLE agent_triggers RENAME COLUMN execution_app_name TO legacy_alias_name;

-- The version columns are live trigger configuration since EVE-1139 (written
-- by the trigger API as `agent_version_policy` / `agent_version_id`), so they
-- take the same names as on agent_endpoints. The wire names do not change.
ALTER TABLE agent_triggers RENAME COLUMN execution_agent_version_policy TO agent_version_policy;
ALTER TABLE agent_triggers RENAME COLUMN execution_agent_version_id TO agent_version_id;

COMMENT ON COLUMN agent_endpoints.legacy_alias_id IS
    'Immutable alias identity (the {app_id} segment) for permanent /v1/apps/{app_id}/... ingress routes. Native Agent endpoints leave it null.';
COMMENT ON COLUMN agent_triggers.legacy_alias_id IS
    'Frozen permanent alias identity (the {app_id} segment of /v1/apps/{app_id}/webhooks/...) for webhook triggers migrated from Apps. Native triggers leave it null.';
COMMENT ON COLUMN agent_triggers.legacy_alias_name IS
    'Frozen display name of the retired App, used by migrated webhook templates ({{app.name}}) and session titles. Native triggers leave it null.';
COMMENT ON COLUMN agent_triggers.agent_version_policy IS
    'Agent version selection for trigger sessions: default, latest, or pinned. NULL means default.';
COMMENT ON COLUMN agent_triggers.agent_version_id IS
    'Pinned agent version for trigger sessions when agent_version_policy is pinned.';

-- Archival App foreign keys keep their names: they reference the frozen `apps`
-- table (EVE-1011), whose rows are never read to serve traffic, so a rename
-- buys nothing. agent_endpoints.app_id is already documented as archival (142).
COMMENT ON COLUMN sessions.app_id IS
    'Archival (EVE-1011): provenance for sessions created through the retired App model; keeps its App-era name because it references the frozen apps table. endpoint_id is the live ingress pointer.';
COMMENT ON COLUMN agent_triggers.execution_app_id IS
    'Archival (EVE-1011): the retired App a webhook trigger was migrated from; keeps its App-era name because it references the frozen apps table. Non-null marks a migrated trigger and is copied to sessions.app_id; NULL for native triggers.';
